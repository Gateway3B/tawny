//! Primary database fences. Backups and independent recovery have separate
//! lifecycles; this only prevents orphaned live writes after account removal.
#[cfg(test)]
mod tests {
    use crate::{auth::Owner, models::VideoProgress, server::AppServerState};

    #[tokio::test]
    async fn delayed_library_saves_cannot_recreate_deleted_account_data() {
        let state = AppServerState::initialize().await.unwrap();
        let owner = Owner(state.accounts().create_guest().await.unwrap().id);
        let other = Owner(state.accounts().create_guest().await.unwrap().id);
        state.seed_new_account(&owner.0).await.unwrap();
        state.seed_new_account(&other.0).await.unwrap();
        state.set_queue(&owner.0, &["before".into()]).await.unwrap();
        state
            .accounts()
            .delete(&owner, Some(&owner.0))
            .await
            .unwrap();
        assert!(
            state
                .set_queue(&owner.0, &["late-private-canary".into()])
                .await
                .is_err()
        );
        assert!(
            state
                .save_playlist(&owner.0, "late", "late-private-canary")
                .await
                .is_err()
        );
        assert!(
            state
                .save_progress(
                    &owner.0,
                    &[VideoProgress {
                        video_id: "late-private-canary".into(),
                        watched: true,
                        progress_seconds: 10,
                        audio_only: false,
                    }]
                )
                .await
                .is_err()
        );
        state
            .set_queue(&other.0, &["other-still-active".into()])
            .await
            .unwrap();
        assert!(state.accounts().find(&owner.0).await.is_err());
        for table in [
            "playlist",
            "library_state",
            "video_progress",
            "user_subscription",
            "subscription_group",
            "brand_legal_receipt",
        ] {
            let rows: Vec<surrealdb_types::RecordId> = state
                .db
                .query(format!(
                    "SELECT VALUE id FROM {table} WHERE owner = type::record($owner)"
                ))
                .bind(("owner", owner.0.clone()))
                .await
                .unwrap()
                .check()
                .unwrap()
                .take(0)
                .unwrap();
            assert!(rows.is_empty(), "Recreated {table}");
        }
        assert_eq!(
            state.viewer(&other.0).await.unwrap().queue,
            ["other-still-active"]
        );
    }

    #[tokio::test]
    async fn every_private_table_and_relation_rejects_a_retired_source() {
        let state = AppServerState::initialize().await.unwrap();
        let owner = Owner(state.accounts().create_guest().await.unwrap().id);
        state.seed_new_account(&owner.0).await.unwrap();
        let playlists: Vec<surrealdb_types::RecordId> = state
            .db
            .query("SELECT VALUE id FROM playlist WHERE owner = type::record($owner) LIMIT 1;")
            .bind(("owner", owner.0.clone()))
            .await
            .unwrap()
            .check()
            .unwrap()
            .take(0)
            .unwrap();
        state
            .accounts()
            .delete(&owner, Some(&owner.0))
            .await
            .unwrap();
        for sql in [
            "CREATE user_subscription SET owner = type::record($owner), channel_id = 'late';",
            "CREATE subscription_group SET owner = type::record($owner), group_id = 'late', name = 'Late';",
            "CREATE library_state SET owner = type::record($owner);",
            "CREATE video_progress SET owner = type::record($owner), video_id = 'late';",
            "CREATE playlist SET owner = type::record($owner), playlist_id = 'late', name = 'Late';",
            "CREATE brand_legal_receipt SET app = 'tawny', owner = type::record($owner), subject = 'synthetic', document_set_digest = 'synthetic', action = 'acknowledged', purpose = '', created_at = time::now();",
            "LET $account = type::record($owner); RELATE $account->subscription->channel:late;",
            "RELATE $playlist->playlist_item->video:late SET position = 0;",
        ] {
            let response = state
                .db
                .query(sql)
                .bind(("owner", owner.0.clone()))
                .bind(("playlist", playlists[0].clone()))
                .await
                .unwrap();
            let error = response.check().unwrap_err().to_string();
            assert!(
                error.contains("Account is unavailable")
                    || error.contains("Playlist is unavailable"),
                "Unexpected rejection for {sql}: {error}"
            );
        }
        assert!(state.accounts().find(&owner.0).await.is_err());
    }

    #[tokio::test]
    async fn a_transaction_with_an_old_account_snapshot_cannot_commit_after_deletion() {
        let state = AppServerState::initialize().await.unwrap();
        let owner = Owner(state.accounts().create_guest().await.unwrap().id);
        let transaction = state.db.as_ref().clone().begin().await.unwrap();
        transaction
            .query("SELECT * FROM type::record($owner)")
            .bind(("owner", owner.0.clone()))
            .await
            .unwrap()
            .check()
            .unwrap();
        state
            .accounts()
            .delete(&owner, Some(&owner.0))
            .await
            .unwrap();
        let written = transaction.query("CREATE video_progress SET owner = type::record($owner), video_id = 'stale-snapshot';")
            .bind(("owner", owner.0.clone())).await;
        if written.and_then(|response| response.check()).is_ok() {
            assert!(transaction.commit().await.is_err());
        } else {
            let _ = transaction.cancel().await;
        }
        let rows: Vec<surrealdb_types::RecordId> = state
            .db
            .query("SELECT VALUE id FROM video_progress WHERE owner = type::record($owner)")
            .bind(("owner", owner.0.clone()))
            .await
            .unwrap()
            .check()
            .unwrap()
            .take(0)
            .unwrap();
        assert!(rows.is_empty());
    }

    #[tokio::test]
    async fn private_ownership_cannot_be_reassigned_and_legacy_unowned_rows_stay_unowned() {
        let state = AppServerState::initialize().await.unwrap();
        let owner = Owner(state.accounts().create_guest().await.unwrap().id);
        let other = Owner(state.accounts().create_guest().await.unwrap().id);
        state.seed_new_account(&owner.0).await.unwrap();
        assert!(state.db.query("UPDATE playlist SET owner = type::record($other) WHERE owner = type::record($owner);")
            .bind(("owner", owner.0.clone())).bind(("other", other.0.clone())).await.unwrap().check().is_err());
        state.db.query("CREATE playlist:legacy_fixture SET playlist_id = 'legacy', name = 'Legacy'; RELATE playlist:legacy_fixture->playlist_item->video:legacy SET position = 0;")
            .await.unwrap().check().unwrap();
        state
            .accounts()
            .delete(&owner, Some(&owner.0))
            .await
            .unwrap();
        let existing: Vec<surrealdb_types::RecordId> = state
            .db
            .query("SELECT VALUE id FROM playlist:legacy_fixture;")
            .await
            .unwrap()
            .check()
            .unwrap()
            .take(0)
            .unwrap();
        assert_eq!(existing.len(), 1);
    }

    #[test]
    fn owner_table_inventory_requires_a_database_write_guard() {
        let guards = include_str!("../../database/schema/zz_privacy_owner.surql");
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("database/schema");
        for entry in std::fs::read_dir(root).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|value| value.to_str()) != Some("surql") {
                continue;
            }
            if std::fs::read_to_string(&path)
                .unwrap()
                .contains("DEFINE FIELD IF NOT EXISTS owner ON")
            {
                let table = path.file_stem().unwrap().to_str().unwrap();
                assert!(
                    guards.contains(&format!("ON TABLE {table} WHEN")),
                    "Review write guard for {table}"
                );
            }
        }
    }
}
