//! Migrate before accepting sessions; fail closed on unknown legacy payloads.
use std::{sync::Arc, time::Duration};

use anyhow::{Result, anyhow};
use surrealdb::{Surreal, engine::any::Any};
use surrealdb_types::{RecordId, SurrealValue};

pub(crate) async fn prepare(db: &Surreal<Any>) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(180), migrate(db))
        .await
        .map_err(|_| anyhow!("Account session migration timed out; keep the app offline."))?
        .map_err(|_| {
            anyhow!("Account session migration is pending; review the private database offline.")
        })
}

async fn migrate(db: &Surreal<Any>) -> Result<()> {
    // Keep exceptional legacy payloads out of the adapter's 128-row batches.
    // Never include their contents in a startup error or diagnostic.
    let large: Vec<RecordId> = db
        .query("SELECT VALUE id FROM sessions WHERE string::len(sessionstore) > 65536 LIMIT 1")
        .await?
        .check()?
        .take(0)?;
    if !large.is_empty() {
        return Err(anyhow!("session_payload_review"));
    }
    crate::auth::session_pool(Arc::new(db.clone()))
        .index_account_sessions("sessions")
        .await?;
    #[derive(SurrealValue)]
    struct Orphan {
        id: RecordId,
        session_account: RecordId,
    }
    let mut after = None::<RecordId>;
    loop {
        let rows: Vec<Orphan> = db.query("SELECT id, session_account FROM sessions WHERE session_account != NONE AND !record::exists(session_account) AND ($after = NONE OR id > $after) ORDER BY id LIMIT 128")
            .bind(("after", after.clone())).await?.check()?.take(0)?;
        let full = rows.len() == 128;
        for row in rows {
            db.query(
                "BEGIN TRANSACTION; fn::privacy_retire_sessions_v1($account); COMMIT TRANSACTION;",
            )
            .bind(("account", row.session_account))
            .await?
            .check()?;
            after = Some(row.id);
        }
        if !full {
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum_session::DatabasePool;
    use surrealdb_types::ToSql;

    fn payload(account: Option<&str>) -> String {
        let data = account
            .map(|key| serde_json::json!({"user_auth_session_id": serde_json::to_string(key).unwrap()}))
            .unwrap_or_else(|| serde_json::json!({}));
        serde_json::json!({"data": data, "private_fixture": "session-secret-canary"}).to_string()
    }

    async fn legacy_database() -> Surreal<Any> {
        let db = surrealdb::engine::any::connect("mem://").await.unwrap();
        db.use_ns("test").use_db("test").await.unwrap();
        db.query("DEFINE TABLE sessions SCHEMALESS PERMISSIONS NONE; CREATE app_user:kept SET display_name = 'Kept', is_guest = true, created_at = time::now(), updated_at = time::now(); CREATE app_user:other SET display_name = 'Other', is_guest = true, created_at = time::now(), updated_at = time::now();")
            .await.unwrap().check().unwrap();
        db
    }

    fn expiry() -> i64 {
        time::OffsetDateTime::now_utc().unix_timestamp() + 3600
    }

    async fn load(pool: &g3_auth::SurrealSessionPool<Any>, id: &str) -> Option<String> {
        pool.load(id, "sessions").await.unwrap()
    }

    #[tokio::test]
    async fn migration_and_deletion_remove_exact_payloads_and_block_delayed_stores() {
        let db = Arc::new(legacy_database().await);
        let legacy = g3_auth::SurrealSessionPool::new(Arc::clone(&db));
        for (id, account) in [
            ("kept-session", Some("kept")),
            ("other-session", Some("other")),
            ("anonymous-session", None),
            ("orphan-session", Some("already-gone")),
        ] {
            legacy
                .store(id, &payload(account), expiry(), "sessions")
                .await
                .unwrap();
        }
        crate::db::sync_schema(&db).await.unwrap();
        prepare(&db).await.unwrap();
        prepare(&db).await.unwrap();
        let pool = crate::auth::session_pool(Arc::clone(&db));
        assert_eq!(
            load(&pool, "kept-session").await,
            Some(payload(Some("kept")))
        );
        assert_eq!(
            load(&pool, "other-session").await,
            Some(payload(Some("other")))
        );
        assert_eq!(load(&pool, "anonymous-session").await, Some(payload(None)));
        assert!(load(&pool, "orphan-session").await.is_none());

        let owner = crate::auth::Owner("app_user:kept".into());
        crate::auth::Accounts::new(&db)
            .delete(&owner, Some(&owner.0))
            .await
            .unwrap();
        assert!(load(&pool, "kept-session").await.is_none());
        pool.store(
            "delayed-session",
            &payload(Some("kept")),
            expiry(),
            "sessions",
        )
        .await
        .unwrap();
        assert!(load(&pool, "delayed-session").await.is_none());
        assert_eq!(
            load(&pool, "other-session").await,
            Some(payload(Some("other")))
        );
        assert_eq!(load(&pool, "anonymous-session").await, Some(payload(None)));
        // An accidentally rolled-back binary cannot bypass tracking.
        assert!(
            legacy
                .store("old-writer", &payload(Some("kept")), expiry(), "sessions")
                .await
                .is_err()
        );
        assert!(load(&pool, "old-writer").await.is_none());
        let fence = RecordId::new(
            "auth_session_subject",
            RecordId::new("app_user", "kept").to_sql(),
        );
        let retired: Option<bool> = db
            .query("SELECT VALUE retired FROM ONLY $fence")
            .bind(("fence", fence))
            .await
            .unwrap()
            .check()
            .unwrap()
            .take(0)
            .unwrap();
        assert_eq!(retired, Some(true));
    }

    #[tokio::test]
    async fn unknown_or_large_legacy_payloads_hold_startup_and_deletion_without_leaking_contents() {
        for contents in [
            "session-secret-canary-invalid-json".to_string(),
            serde_json::json!({"data": {"user_auth_session_id": ["session-secret-canary"]}})
                .to_string(),
            format!("{}session-secret-canary", "x".repeat(65537)),
        ] {
            let db = Arc::new(legacy_database().await);
            let legacy = g3_auth::SurrealSessionPool::new(Arc::clone(&db));
            legacy
                .store("unknown", &contents, expiry(), "sessions")
                .await
                .unwrap();
            crate::db::sync_schema(&db).await.unwrap();
            let error = prepare(&db).await.unwrap_err().to_string();
            assert!(error.contains("migration is pending"));
            assert!(!error.contains("session-secret-canary"));
            let owner = crate::auth::Owner("app_user:kept".into());
            assert!(
                crate::auth::Accounts::new(&db)
                    .delete(&owner, Some(&owner.0))
                    .await
                    .is_err()
            );
            assert!(crate::auth::Accounts::new(&db).find(&owner.0).await.is_ok());
            assert_eq!(
                load(&legacy, "unknown").await.as_deref(),
                Some(contents.as_str())
            );
        }
    }

    #[tokio::test]
    async fn concurrent_session_responses_cannot_restore_payloads_after_successful_deletion() {
        let state = crate::server::AppServerState::initialize().await.unwrap();
        let pool = crate::auth::session_pool(Arc::clone(&state.db));
        for _ in 0..12 {
            let owner = crate::auth::Owner(state.accounts().create_guest().await.unwrap().id);
            let key = owner.0.strip_prefix("app_user:").unwrap();
            let content = payload(Some(key));
            pool.store("racing", &content, expiry(), "sessions")
                .await
                .unwrap();
            let accounts = state.accounts();
            let (removed, _) = tokio::join!(
                accounts.delete(&owner, Some(&owner.0)),
                pool.store("racing", &content, expiry(), "sessions")
            );
            if removed.is_err() {
                // A serialization conflict must be retried, never called success.
                state
                    .accounts()
                    .delete(&owner, Some(&owner.0))
                    .await
                    .unwrap();
            }
            pool.store("late", &content, expiry(), "sessions")
                .await
                .unwrap();
            assert!(load(&pool, "racing").await.is_none());
            assert!(load(&pool, "late").await.is_none());
            assert!(state.accounts().find(&owner.0).await.is_err());
        }
    }
}
