//! Release-specific primary erasure replay. The platform wrapper must verify
//! Docker quarantine and authenticate independent ledger objects first.
use g3_brand::privacy::{PrivacyKind, RecoveryEvent};
use std::{io::Read, sync::Arc, time::Duration};
use surrealdb::{Surreal, engine::any::Any};
use surrealdb_types::{RecordId, ToSql};

include!(concat!(env!("OUT_DIR"), "/privacy_schema.rs"));
type Result<T> = std::result::Result<T, &'static str>;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    schema_version: u32,
    app: String,
    request_id: String,
    subject: String,
    subject_version: u64,
    action: PrivacyKind,
    #[serde(default)]
    purpose: Option<String>,
    #[serde(default)]
    corrections: Option<std::collections::BTreeMap<String, serde_json::Value>>,
}

fn parse(body: &[u8]) -> Result<RecoveryEvent> {
    if body.len() > 32768 {
        return Err("Recovery event exceeds limit");
    }
    let input: Input = serde_json::from_slice(body).map_err(|_| "Invalid recovery event")?;
    let event = RecoveryEvent {
        schema_version: input.schema_version,
        app: input.app,
        request_id: input.request_id,
        subject: input.subject,
        subject_version: input.subject_version,
        action: input.action,
        purpose: input.purpose,
        corrections: input.corrections,
    };
    event.validate().map_err(|_| "Invalid recovery event")?;
    if event.app != "tawny"
        || event.action != PrivacyKind::Delete
        || event.purpose.is_some()
        || event.corrections.is_some()
        || uuid::Uuid::parse_str(&event.request_id).is_err()
        || event.subject_version > i64::MAX as u64
        || event.subject.len() != 64
        || !event
            .subject
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("Unsupported app recovery event");
    }
    Ok(event)
}

fn validate_target(read: impl Fn(&str) -> String) -> Result<()> {
    let container = read("RESTORE_CONTAINER");
    if container.is_empty()
        || container.len() > 100
        || !container
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        || read("G3_PRIVACY_RESTORE") != "quarantined-v1"
        || read("SURREALDB_AUTH_LEVEL") != "root"
        || read("SURREALDB_NAMESPACE") != "restore"
        || read("SURREALDB_NAME") != "tawny"
        || read("SURREALDB_HOST") != format!("ws://{container}:8000")
    {
        return Err("A quarantined restore target is required");
    }
    Ok(())
}

pub(crate) fn main(args: &[String]) -> i32 {
    let result = (|| {
        let apply = match args {
            [mode] if mode == "--apply" => true,
            [mode] if mode == "--verify" => false,
            _ => return Err("Choose apply or verify"),
        };
        validate_target(|name| std::env::var(name).unwrap_or_default())?;
        let mut body = Vec::new();
        std::io::stdin()
            .take(32769)
            .read_to_end(&mut body)
            .map_err(|_| "Could not read event")?;
        let event = parse(&body)?;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|_| "Restore runtime failed")?;
        runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(240), async {
                let db = Arc::new(
                    crate::db::connect()
                        .await
                        .map_err(|_| "Restore connection failed")?,
                );
                if apply {
                    apply_event(Arc::clone(&db), &event).await?;
                }
                verify_event(&db, &event).await
            })
            .await
            .map_err(|_| "Restore replay timed out")?
        })
    })();
    match result {
        Ok(()) => {
            println!("event=privacy_replay_verified");
            0
        }
        Err(_) => {
            eprintln!("event=privacy_replay_failed_quarantine_preserved");
            1
        }
    }
}

async fn schema(db: &Surreal<Any>) -> Result<()> {
    for source in RESTORE_SCHEMA {
        // Backup metadata hashes are not proof that a guard still exists.
        // Reinstall this release's definitions; never run startup data repairs.
        db.query(source.replace("IF NOT EXISTS", "OVERWRITE"))
            .await
            .and_then(|response| response.check())
            .map_err(|_| "Restore schema requires review")?;
    }
    Ok(())
}

async fn info(db: &Surreal<Any>, query: impl Into<String>) -> Result<serde_json::Value> {
    db.query(query.into())
        .await
        .and_then(|response| response.check())
        .map_err(|_| "Restore schema requires review")?
        .take::<Option<serde_json::Value>>(0)
        .map_err(|_| "Restore schema requires review")?
        .ok_or("Restore schema requires review")
}

async fn reference() -> Result<Surreal<Any>> {
    let db = surrealdb::engine::any::connect("mem://")
        .await
        .map_err(|_| "Restore schema requires review")?;
    db.use_ns("reference")
        .use_db("reference")
        .await
        .map_err(|_| "Restore schema requires review")?;
    schema(&db).await?;
    Ok(db)
}

async fn reviewed_tables(db: &Surreal<Any>, reference: &Surreal<Any>) -> Result<()> {
    let actual = info(db, "INFO FOR DB").await?;
    let expected = info(reference, "INFO FOR DB").await?;
    let actual = actual["tables"]
        .as_object()
        .ok_or("Restore schema requires review")?;
    let expected = expected["tables"]
        .as_object()
        .ok_or("Restore schema requires review")?;
    // SurrealKit metadata is never used as evidence for replay.
    if actual.keys().any(|name| {
        !matches!(name.as_str(), "__entity" | "__rollout") && !expected.contains_key(name)
    }) {
        return Err("Unknown restored tables require review");
    }
    Ok(())
}

async fn verify_schema(db: &Surreal<Any>, reference: &Surreal<Any>) -> Result<()> {
    reviewed_tables(db, reference).await?;
    let expected = info(reference, "INFO FOR DB").await?;
    let actual = info(db, "INFO FOR DB").await?;
    if actual["functions"] != expected["functions"] {
        return Err("Restore guards require review");
    }
    for name in expected["tables"]
        .as_object()
        .ok_or("Restore schema requires review")?
        .keys()
    {
        // Names come only from source definitions, never event data.
        if actual["tables"][name] != expected["tables"][name]
            || info(db, format!("INFO FOR TABLE {name}")).await?
                != info(reference, format!("INFO FOR TABLE {name}")).await?
        {
            return Err("Restore definitions require review");
        }
    }
    Ok(())
}

const CANDIDATES: &[&str] = &[
    "SELECT VALUE id FROM app_user WHERE crypto::sha256(<string>id) = $subject LIMIT 2",
    "SELECT VALUE owner FROM playlist WHERE owner != NONE AND crypto::sha256(<string>owner) = $subject LIMIT 2",
    "SELECT VALUE owner FROM library_state WHERE owner != NONE AND crypto::sha256(<string>owner) = $subject LIMIT 2",
    "SELECT VALUE owner FROM subscription_group WHERE owner != NONE AND crypto::sha256(<string>owner) = $subject LIMIT 2",
    "SELECT VALUE owner FROM user_subscription WHERE owner != NONE AND crypto::sha256(<string>owner) = $subject LIMIT 2",
    "SELECT VALUE owner FROM video_progress WHERE owner != NONE AND crypto::sha256(<string>owner) = $subject LIMIT 2",
    "SELECT VALUE owner FROM brand_legal_receipt WHERE owner != NONE AND crypto::sha256(<string>owner) = $subject LIMIT 2",
    "SELECT VALUE in FROM subscription WHERE crypto::sha256(<string>in) = $subject LIMIT 2",
    "SELECT VALUE session_account FROM sessions WHERE session_account != NONE AND crypto::sha256(<string>session_account) = $subject LIMIT 2",
    "SELECT VALUE account FROM auth_session_subject WHERE crypto::sha256(<string>account) = $subject LIMIT 2",
    "SELECT VALUE account FROM privacy_request WHERE account != NONE AND crypto::sha256(<string>account) = $subject LIMIT 2",
];

async fn account(db: &Surreal<Any>, subject: &str) -> Result<RecordId> {
    let mut matched = None;
    for query in CANDIDATES {
        let rows: Vec<RecordId> = db
            .query(*query)
            .bind(("subject", subject.to_owned()))
            .await
            .and_then(|response| response.check())
            .map_err(|_| "Restored identity requires review")?
            .take(0)
            .map_err(|_| "Restored identity requires review")?;
        for row in rows {
            if row.table.as_str() != "app_user"
                || g3_brand::model::digest(row.to_sql().as_bytes()) != subject
                || matched.as_ref().is_some_and(|previous| previous != &row)
            {
                return Err("Restored identity requires review");
            }
            matched = Some(row);
        }
    }
    matched.ok_or("Restored identity requires review")
}

const PURGE: &str = concat!(
    r#"
BEGIN TRANSACTION;
LET $existing = (SELECT * FROM ONLY $id);
IF $existing != NONE AND array::len(array::complement(object::keys($existing), $reviewed_request_fields)) != 0 { THROW 'Request fields require review'; };
LET $profile = (SELECT * FROM ONLY type::record($owner));
IF $profile != NONE AND array::len(array::complement(object::keys($profile), ['id', 'display_name', 'email', 'password_hash', 'is_guest', 'created_at', 'updated_at', 'privacy_revision'])) != 0 { THROW 'Account fields require review'; };
IF $existing != NONE AND ($existing.app != 'tawny' OR $existing.request_id != $request_id
    OR $existing.subject != $subject OR $existing.subject_version != $version
    OR ($existing.account != NONE AND $existing.account != type::record($owner))
    OR ($existing.account = NONE AND $existing.state != 'primary_verified')
    OR ($existing.recovery_digest != NONE AND $existing.recovery_digest != $digest)
    OR ($existing.state = 'verified' AND $existing.recovery_digest != NONE)
    OR ($existing.state != 'verified' AND $existing.recovery_digest = NONE)) {
    THROW 'Conflicting recovery evidence';
};
IF count(SELECT id FROM privacy_request WHERE subject = $subject AND id != $id) != 0 { THROW 'Conflicting recovery revision'; };
IF $existing = NONE {
    CREATE $id SET app = 'tawny', request_id = $request_id, subject = $subject,
        subject_version = $version, account = type::record($owner), state = 'verified';
};
IF (SELECT VALUE state FROM ONLY $id) = 'verified' {
    UPDATE $id SET recovery_digest = $digest, state = 'recovery_recorded', updated_at = time::now();
};
"#,
    crate::privacy::primary::erase_primary_sql!(),
    r#"
IF (SELECT VALUE state FROM ONLY $id) = 'recovery_recorded' {
    UPDATE $id SET state = 'primary_removed', updated_at = time::now();
};
UPDATE type::record('auth_session_subject', $owner) SET privacy_version = math::max([privacy_version ?? 0, $version]);
COMMIT TRANSACTION;
"#
);

fn digest(event: &RecoveryEvent) -> Result<String> {
    serde_json::to_vec(event)
        .map(|body| g3_brand::model::digest(&body))
        .map_err(|_| "Invalid recovery event")
}

pub(super) async fn apply_event(db: Arc<Surreal<Any>>, event: &RecoveryEvent) -> Result<()> {
    let event = parse(&serde_json::to_vec(event).map_err(|_| "Invalid recovery event")?)?;
    let expected = reference().await?;
    reviewed_tables(&db, &expected).await?;
    schema(&db).await?;
    verify_schema(&db, &expected).await?;
    super::sessions::prepare(&db)
        .await
        .map_err(|_| "Restored sessions require review")?;
    let account = account(&db, &event.subject).await?;
    db.query(PURGE)
        .bind((
            "reviewed_request_fields",
            super::recovery::REQUEST_FIELDS
                .iter()
                .map(|name| (*name).to_owned())
                .collect::<Vec<_>>(),
        ))
        .bind((
            "id",
            RecordId::new("privacy_request", event.request_id.clone()),
        ))
        .bind(("request_id", event.request_id.clone()))
        .bind(("subject", event.subject.clone()))
        .bind(("version", event.subject_version as i64))
        .bind(("digest", digest(&event)?))
        .bind(("owner", account.to_sql()))
        .await
        .and_then(|response| response.check())
        .map_err(|_| "Restored primary removal requires review")?;
    super::recovery::verify_primary(&db, &account).await?;
    crate::auth::session_pool(Arc::clone(&db))
        .retire_account_sessions("sessions", &account)
        .await
        .map_err(|_| "Restored sessions require review")?;
    db.query("UPDATE $id SET state = 'primary_verified', account = NONE, primary_verified_at = time::now(), updated_at = time::now(), lease_owner = NONE, lease_until = NONE, last_error_code = NONE WHERE state = 'primary_removed' AND recovery_digest = $digest")
        .bind(("id", RecordId::new("privacy_request", event.request_id.clone())))
        .bind(("digest", digest(&event)?))
        .await.and_then(|response| response.check()).map_err(|_| "Restored checkpoint requires review")?;
    verify_event(&db, &event).await
}

const ORPHANS: &str = r#"RETURN count(SELECT id FROM sessions WHERE account_indexed != true)
        + count(SELECT id FROM playlist WHERE (IF type::is_record(owner) { !record::exists(owner) } ELSE { owner != NONE }))
        + count(SELECT id FROM library_state WHERE (IF type::is_record(owner) { !record::exists(owner) } ELSE { owner != NONE }))
        + count(SELECT id FROM subscription_group WHERE (IF type::is_record(owner) { !record::exists(owner) } ELSE { owner != NONE }))
        + count(SELECT id FROM user_subscription WHERE (IF type::is_record(owner) { !record::exists(owner) } ELSE { owner != NONE }))
        + count(SELECT id FROM video_progress WHERE (IF type::is_record(owner) { !record::exists(owner) } ELSE { owner != NONE }))
        + count(SELECT id FROM brand_legal_receipt WHERE (IF type::is_record(owner) { !record::exists(owner) } ELSE { owner != NONE }))
        + count(SELECT id FROM subscription WHERE in.id = NONE)
        + count(SELECT id FROM playlist_item WHERE !record::exists(in) OR (IF type::is_record(in.owner) { !record::exists(in.owner) } ELSE { in.owner != NONE }));"#;

pub(super) async fn verify_event(db: &Surreal<Any>, event: &RecoveryEvent) -> Result<()> {
    let event = parse(&serde_json::to_vec(event).map_err(|_| "Invalid recovery event")?)?;
    verify_schema(db, &reference().await?).await?;
    let account = account(db, &event.subject).await?;
    super::recovery::verify_primary(db, &account).await?;
    let count: Option<i64> = db.query("RETURN count(SELECT id FROM $id WHERE app = 'tawny' AND request_id = $request_id AND subject = $subject AND subject_version = $version AND recovery_digest = $digest AND state = 'primary_verified' AND account = NONE AND primary_verified_at != NONE AND lease_owner = NONE AND lease_until = NONE AND last_error_code = NONE)")
        .bind(("id", RecordId::new("privacy_request", event.request_id.clone())))
        .bind(("request_id", event.request_id.clone())).bind(("subject", event.subject.clone()))
        .bind(("version", event.subject_version as i64)).bind(("digest", digest(&event)?))
        .await.and_then(|response| response.check()).map_err(|_| "Restored checkpoint requires review")?
        .take(0).map_err(|_| "Restored checkpoint requires review")?;
    if count != Some(1) {
        return Err("Restored checkpoint requires review");
    }
    let orphans: Option<i64> = db
        .query(ORPHANS)
        .await
        .and_then(|response| response.check())
        .map_err(|_| "Restored orphan review is pending")?
        .take(0)
        .map_err(|_| "Restored orphan review is pending")?;
    if orphans != Some(0) {
        return Err("Restored orphan review is pending");
    }
    Ok(())
}

#[cfg(test)]
pub(crate) use tests::wire_snapshot;

#[cfg(test)]
mod tests {
    use super::*;
    use axum_session::DatabasePool;

    const LEGACY: &str = r#"
DEFINE TABLE app_user SCHEMALESS PERMISSIONS NONE;
DEFINE TABLE playlist SCHEMALESS PERMISSIONS NONE;
DEFINE TABLE library_state SCHEMALESS PERMISSIONS NONE;
DEFINE TABLE sessions SCHEMALESS PERMISSIONS NONE;
CREATE app_user:deleted SET display_name = 'Private profile canary', is_guest = true, created_at = time::now(), updated_at = time::now();
CREATE app_user:kept SET display_name = 'Preserved owner', is_guest = true, created_at = time::now(), updated_at = time::now();
CREATE playlist:deleted SET owner = app_user:deleted, playlist_id = 'saved', name = 'Private playlist canary';
CREATE playlist:kept SET owner = app_user:kept, playlist_id = 'saved', name = 'Preserved playlist';
CREATE playlist:demo SET playlist_id = 'demo', name = 'Unowned demo';
CREATE library_state:deleted SET owner = app_user:deleted, queue_json = '["private-video-canary"]';
CREATE library_state:kept SET owner = app_user:kept, queue_json = '["preserved-video"]';
CREATE video:shared SET video_id = 'synthetic-shared-video', title = 'Shared fact', channel_id = 'synthetic-channel',
    channel_name = 'Shared channel', thumbnail_url = '', published_at = '', published_sort = '',
    duration_seconds = 0, view_count = '', is_live = false, is_short = false,
    progress_seconds = 0, watched = false, audio_only = false;
RELATE playlist:deleted->playlist_item->video:shared SET position = 0;
RELATE playlist:kept->playlist_item->video:shared SET position = 0;
"#;

    fn event() -> RecoveryEvent {
        RecoveryEvent {
            schema_version: 1,
            app: "tawny".into(),
            request_id: uuid::Uuid::new_v4().to_string(),
            subject: g3_brand::model::digest(
                RecordId::new("app_user", "deleted").to_sql().as_bytes(),
            ),
            subject_version: 1,
            action: PrivacyKind::Delete,
            purpose: None,
            corrections: None,
        }
    }

    async fn seed(db: &Surreal<Any>) {
        db.query(LEGACY).await.unwrap().check().unwrap();
        let pool = g3_auth::SurrealSessionPool::new(Arc::new(db.clone()));
        for (id, account) in [
            ("deleted", Some("deleted")),
            ("kept", Some("kept")),
            ("anonymous", None),
        ] {
            let data = account.map(|value| serde_json::json!({"user_auth_session_id":serde_json::to_string(value).unwrap()}))
                .unwrap_or_else(|| serde_json::json!({}));
            let payload =
                serde_json::json!({"data":data, "private_fixture":"session-secret-canary"})
                    .to_string();
            pool.store(
                id,
                &payload,
                time::OffsetDateTime::now_utc().unix_timestamp() + 3600,
                "sessions",
            )
            .await
            .unwrap();
        }
    }

    async fn legacy() -> Arc<Surreal<Any>> {
        let db = surrealdb::engine::any::connect("mem://").await.unwrap();
        db.use_ns("restore").use_db("tawny").await.unwrap();
        seed(&db).await;
        Arc::new(db)
    }

    async fn present(db: &Surreal<Any>, id: &str) -> bool {
        let row: Option<serde_json::Value> = db
            .select(RecordId::parse_simple(id).unwrap())
            .await
            .unwrap();
        row.is_some()
    }

    async fn assert_removed(db: &Surreal<Any>, event: &RecoveryEvent) {
        verify_event(db, event).await.unwrap();
        assert!(!present(db, "app_user:deleted").await);
        assert!(!present(db, "playlist:deleted").await);
        assert!(!present(db, "library_state:deleted").await);
        assert!(present(db, "app_user:kept").await);
        assert!(present(db, "playlist:kept").await);
        assert!(present(db, "playlist:demo").await);
        assert!(present(db, "video:shared").await);
        let edges: Vec<RecordId> = db
            .query("SELECT VALUE in FROM playlist_item")
            .await
            .unwrap()
            .check()
            .unwrap()
            .take(0)
            .unwrap();
        assert_eq!(edges, [RecordId::new("playlist", "kept")]);
        let pool = crate::auth::session_pool(Arc::new(db.clone()));
        assert!(pool.load("deleted", "sessions").await.unwrap().is_none());
        assert!(pool.load("kept", "sessions").await.unwrap().is_some());
        assert!(pool.load("anonymous", "sessions").await.unwrap().is_some());
        assert!(db.query("CREATE library_state SET owner = app_user:deleted, queue_json = '[\"late-private-canary\"]'")
            .await.unwrap().check().is_err());
        let late = serde_json::json!({"data":{"user_auth_session_id":"\"deleted\""}}).to_string();
        pool.store(
            "late",
            &late,
            time::OffsetDateTime::now_utc().unix_timestamp() + 3600,
            "sessions",
        )
        .await
        .unwrap();
        assert!(pool.load("late", "sessions").await.unwrap().is_none());
    }

    #[test]
    fn strict_event_boundary_rejects_duplicates_scopes_and_unreviewed_actions() {
        let valid = serde_json::to_vec(&event()).unwrap();
        assert!(parse(&valid).is_ok());
        assert!(parse(&vec![b' '; 32769]).is_err());
        let value: serde_json::Value = serde_json::from_slice(&valid).unwrap();
        for (key, wrong) in [
            ("app", serde_json::json!("other")),
            ("request_id", serde_json::json!("sql:injection")),
            ("subject", serde_json::json!("A".repeat(64))),
            ("subject_version", serde_json::json!(0)),
            ("subject_version", serde_json::json!(u64::MAX)),
            ("action", serde_json::json!("correct")),
            ("purpose", serde_json::json!("tracking")),
            ("corrections", serde_json::json!({"display_name":"private"})),
            ("future_processor", serde_json::json!(true)),
        ] {
            let mut input = value.clone();
            input[key] = wrong;
            assert!(
                parse(&serde_json::to_vec(&input).unwrap()).is_err(),
                "{key}"
            );
        }
        let duplicate = String::from_utf8(valid)
            .unwrap()
            .replacen('{', "{\"app\":\"tawny\",", 1);
        assert!(parse(duplicate.as_bytes()).is_err());
    }

    #[test]
    fn target_is_exact_and_never_loads_normal_app_configuration() {
        let values = std::collections::BTreeMap::from([
            ("RESTORE_CONTAINER", "owned-scratch"),
            ("G3_PRIVACY_RESTORE", "quarantined-v1"),
            ("SURREALDB_AUTH_LEVEL", "root"),
            ("SURREALDB_NAMESPACE", "restore"),
            ("SURREALDB_NAME", "tawny"),
            ("SURREALDB_HOST", "ws://owned-scratch:8000"),
        ]);
        assert!(validate_target(|name| values.get(name).unwrap_or(&"").to_string()).is_ok());
        for (key, value) in [
            ("RESTORE_CONTAINER", "../app"),
            ("RESTORE_CONTAINER", ""),
            ("G3_PRIVACY_RESTORE", ""),
            ("SURREALDB_AUTH_LEVEL", "database"),
            ("SURREALDB_NAMESPACE", "production"),
            ("SURREALDB_NAME", "main"),
            ("SURREALDB_HOST", "ws://127.0.0.1:8000"),
            ("SURREALDB_HOST", "wss://owned-scratch:8000"),
        ] {
            let mut changed = values.clone();
            changed.insert(key, value);
            assert!(validate_target(|name| changed.get(name).unwrap_or(&"").to_string()).is_err());
        }
        assert_eq!(main(&["--sql".into()]), 1);
    }

    #[tokio::test]
    async fn older_backup_without_requests_replays_exact_primary_data_and_sessions_idempotently() {
        let db = legacy().await;
        let event = event();
        assert!(verify_event(&db, &event).await.is_err());
        apply_event(Arc::clone(&db), &event).await.unwrap();
        assert_removed(&db, &event).await;
        apply_event(Arc::clone(&db), &event).await.unwrap();
        assert_removed(&db, &event).await;
    }

    #[tokio::test]
    async fn normal_startup_metadata_does_not_replace_independent_recovery_evidence() {
        let db = legacy().await;
        crate::db::sync_schema(&db).await.unwrap();
        let event = event();
        assert!(verify_event(&db, &event).await.is_err());
        apply_event(Arc::clone(&db), &event).await.unwrap();
        assert_removed(&db, &event).await;
    }

    #[tokio::test]
    async fn saved_checkpoints_never_override_conflicting_independent_evidence() {
        for checkpoint in [
            "verified",
            "recovery_recorded",
            "primary_removed",
            "primary_verified",
        ] {
            let db = legacy().await;
            schema(&db).await.unwrap();
            super::super::sessions::prepare(&db).await.unwrap();
            let event = event();
            let saved = super::super::recovery::enqueue(
                &db,
                &crate::auth::Owner("app_user:deleted".into()),
            )
            .await
            .unwrap();
            let mut exact = event.clone();
            exact.request_id = saved;
            if checkpoint != "verified" {
                db.query("UPDATE $id SET state = 'recovery_recorded', recovery_digest = $digest")
                    .bind((
                        "id",
                        RecordId::new("privacy_request", exact.request_id.clone()),
                    ))
                    .bind(("digest", digest(&exact).unwrap()))
                    .await
                    .unwrap()
                    .check()
                    .unwrap();
            }
            if ["primary_removed", "primary_verified"].contains(&checkpoint) {
                db.query("UPDATE $id SET state = 'primary_removed'")
                    .bind((
                        "id",
                        RecordId::new("privacy_request", exact.request_id.clone()),
                    ))
                    .await
                    .unwrap()
                    .check()
                    .unwrap();
            }
            if checkpoint == "primary_verified" {
                db.query("UPDATE $id SET state = 'primary_verified', account = NONE, primary_verified_at = time::now()")
                    .bind(("id", RecordId::new("privacy_request", exact.request_id.clone()))).await.unwrap().check().unwrap();
            }
            // A different request for the same subject cannot replace the saved one.
            assert!(apply_event(Arc::clone(&db), &event).await.is_err());
            let mut wrong_revision = exact.clone();
            wrong_revision.subject_version += 1;
            assert!(apply_event(Arc::clone(&db), &wrong_revision).await.is_err());
            assert!(present(&db, "app_user:deleted").await);
            apply_event(Arc::clone(&db), &exact).await.unwrap();
            assert_removed(&db, &exact).await;
        }
    }

    #[tokio::test]
    async fn future_schema_fields_and_unknown_sessions_hold_without_primary_erasure() {
        for sql in [
            "DEFINE TABLE future_processor SCHEMALESS; CREATE future_processor SET token = 'private-processor-canary';",
            "DEFINE FIELD future_token ON app_user TYPE option<string>; UPDATE app_user:deleted SET future_token = 'private-processor-canary';",
            "UPDATE app_user:deleted SET future_token = 'private-processor-canary';",
            "UPDATE sessions SET sessionstore = 'unknown-encrypted-session-canary';",
        ] {
            let db = legacy().await;
            db.query(sql).await.unwrap().check().unwrap();
            let problem = apply_event(Arc::clone(&db), &event()).await.unwrap_err();
            assert!(!problem.contains("canary"));
            assert!(present(&db, "app_user:deleted").await);
            assert!(present(&db, "app_user:kept").await);
        }
    }

    #[tokio::test]
    async fn missing_profile_uses_surviving_ownership_but_unknown_identity_stays_closed() {
        let db = legacy().await;
        db.query("DELETE app_user:deleted")
            .await
            .unwrap()
            .check()
            .unwrap();
        let event = event();
        apply_event(Arc::clone(&db), &event).await.unwrap();
        assert_removed(&db, &event).await;
        let mut unknown = event.clone();
        unknown.request_id = uuid::Uuid::new_v4().to_string();
        unknown.subject = g3_brand::model::digest(b"app_user:no-surviving-identity");
        assert!(apply_event(Arc::clone(&db), &unknown).await.is_err());
        assert!(verify_event(&db, &unknown).await.is_err());
    }

    #[tokio::test]
    async fn verification_detects_missing_guards_and_unattributable_orphans_without_repairing_them()
    {
        let db = legacy().await;
        let event = event();
        apply_event(Arc::clone(&db), &event).await.unwrap();
        db.query("REMOVE EVENT privacy_owner_write_v1 ON library_state; CREATE library_state:orphan SET owner = app_user:unrelated_missing;")
            .await.unwrap().check().unwrap();
        assert!(verify_event(&db, &event).await.is_err());
        assert!(present(&db, "library_state:orphan").await);
        // Reinstalling source guards does not attribute or silently discard it.
        assert!(apply_event(Arc::clone(&db), &event).await.is_err());
        assert!(present(&db, "library_state:orphan").await);
    }

    // Called only from ci/privacy_auth_drill.py's owned, loopback-only fixture.
    pub(crate) async fn wire_snapshot(db: &Surreal<Any>, endpoint: &str) {
        db.query("DEFINE NAMESPACE restore")
            .await
            .unwrap()
            .check()
            .unwrap();
        db.use_ns("restore").use_db("before").await.unwrap();
        seed(db).await;
        let event = event();
        let http = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap();
        let origin = endpoint.replacen("ws://", "http://", 1);
        let snapshot = http
            .get(format!("{origin}/export"))
            .basic_auth("fixture-root", Some("synthetic-privacy-fixture-password"))
            .header("surreal-ns", "restore")
            .header("surreal-db", "before")
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .bytes()
            .await
            .unwrap();
        assert!(!snapshot.is_empty() && snapshot.len() < 1024 * 1024);
        db.use_db("tawny").await.unwrap();
        let imported: serde_json::Value = http
            .post(format!("{origin}/import"))
            .basic_auth("fixture-root", Some("synthetic-privacy-fixture-password"))
            .header("surreal-ns", "restore")
            .header("surreal-db", "tawny")
            .header("content-type", "text/plain")
            .header("accept", "application/json")
            .body(snapshot)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(
            imported
                .as_array()
                .is_some_and(|rows| rows.iter().all(|row| row["status"] == "OK")),
            "Synthetic import result: {imported}"
        );
        assert!(present(db, "app_user:deleted").await);
        assert!(verify_event(db, &event).await.is_err());
        apply_event(Arc::new(db.clone()), &event).await.unwrap();
        assert_removed(db, &event).await;
        apply_event(Arc::new(db.clone()), &event).await.unwrap();
        assert_removed(db, &event).await;
    }
}
