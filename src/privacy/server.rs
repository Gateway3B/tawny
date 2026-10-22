//! Fixed, owner-scoped projections. New private columns never silently join
//! downloads. Collection is live, bounded and rejected rather than truncated.
use crate::auth::Owner;
use dioxus::prelude::*;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use surrealdb::{Surreal, engine::any::Any};
use surrealdb_types::{RecordId, SurrealValue};

pub(super) struct Limits {
    pub page_size: usize,
    pub max_rows: usize,
    pub max_bytes: usize,
    pub max_row_chars: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            page_size: 128,
            max_rows: 100_000,
            max_bytes: 32 * 1024 * 1024,
            max_row_chars: 64 * 1024,
        }
    }
}

fn error(message: &str) -> dioxus::CapturedError {
    ServerFnError::new(message).into()
}

fn too_large() -> dioxus::CapturedError {
    error(
        "This account needs an assisted export. Contact the operator of your Tawny server; no partial download was created.",
    )
}

#[derive(SurrealValue)]
struct Row {
    id: RecordId,
    data: Option<Value>,
}

// Only checked-in SQL fragments are formatted into queries. All owner, cursor
// and limit values are bound separately. The outer projection suppresses an
// oversized row before it can cross the database connection into this process.
const SECTIONS: &[(&str, &str, &str, &str)] = &[
    (
        "privacy_requests",
        "privacy_request",
        "account = $user",
        "{request_id: request_id, state: state, created_at: created_at, updated_at: updated_at}",
    ),
    (
        "playlists",
        "playlist",
        "owner = $user",
        "{id: type::string(id), playlist_id: playlist_id, name: name, description: description, pinned: pinned, sort_order: sort_order, video_ids: video_ids, updated_at: updated_at}",
    ),
    (
        "playlist_items",
        "playlist_item",
        "in.owner = $user",
        "{id: type::string(id), playlist_id: in.playlist_id, video: type::string(out), position: position, added_at: added_at}",
    ),
    (
        "subscription_groups",
        "subscription_group",
        "owner = $user",
        "{id: type::string(id), group_id: group_id, name: name, channel_ids: channel_ids, updated_at: updated_at}",
    ),
    (
        "library_state",
        "library_state",
        "owner = $user",
        "{queue_json: queue_json, history_json: history_json, updated_at: updated_at}",
    ),
    (
        "video_progress",
        "video_progress",
        "owner = $user",
        "{video_id: video_id, watched: watched, progress_seconds: progress_seconds, audio_only: audio_only, updated_at: updated_at}",
    ),
    (
        "subscriptions",
        "user_subscription",
        "owner = $user",
        "{channel_id: channel_id, subscribed: subscribed, content: content, updated_at: updated_at}",
    ),
    (
        "legacy_subscription_links",
        "subscription",
        "in = $user",
        "{id: type::string(id), channel_id: out.channel_id, created_at: created_at}",
    ),
    (
        "legal_receipts",
        "brand_legal_receipt",
        "owner = $user",
        "{id: type::string(id), app: app, document_set_digest: document_set_digest, action: action, purpose: purpose, created_at: created_at}",
    ),
];

const PROFILE: &str = r#"
SELECT id, IF string::len(type::string(data)) <= $row_limit { data } ELSE { NONE } AS data
FROM (SELECT id, {id: type::string(id), display_name: display_name, email: email,
    is_guest: is_guest, created_at: created_at, updated_at: updated_at} AS data
    FROM $user LIMIT 1);
"#;

pub(super) async fn export(
    db: &Surreal<Any>,
    owner: &Owner,
    expected: Option<&str>,
    limits: Limits,
) -> Result<String> {
    owner
        .require_reviewed(expected)
        .map_err(|problem| error(&problem.to_string()))?;
    if limits.page_size == 0 || limits.page_size > 128 || limits.max_row_chars == 0 {
        return Err(error("Invalid account download configuration."));
    }
    let user =
        RecordId::parse_simple(&owner.0).map_err(|_| error("Your account is unavailable."))?;
    let started = crate::brand::timestamp();
    let mut rows: Vec<Row> = db
        .query(PROFILE)
        .bind(("user", user.clone()))
        .bind(("row_limit", limits.max_row_chars as i64))
        .await
        .map_err(|_| error("Couldn't prepare your account download."))?
        .check()
        .map_err(|_| error("Couldn't prepare your account download."))?
        .take(0)
        .map_err(|_| error("Couldn't prepare your account download."))?;
    let profile = rows
        .pop()
        .ok_or_else(|| error("Your account is no longer available."))?
        .data
        .ok_or_else(too_large)?;
    let mut bytes = serde_json::to_vec(&profile)
        .map_err(|_| error("Couldn't prepare your account download."))?
        .len();
    let mut count = 0_usize;
    let mut sections = BTreeMap::<&str, Vec<Value>>::new();
    for &(name, table, scope, fields) in SECTIONS {
        let query = format!(
            "SELECT id, IF string::len(type::string(data)) <= $row_limit {{ data }} ELSE {{ NONE }} AS data FROM (SELECT id, {fields} AS data FROM {table} WHERE ({scope}) AND ($after = NONE OR id > $after) ORDER BY id LIMIT $limit);"
        );
        let mut after = None::<RecordId>;
        let mut values = Vec::new();
        loop {
            let rows: Vec<Row> = db
                .query(&query)
                .bind(("user", user.clone()))
                .bind(("after", after.clone()))
                .bind(("limit", limits.page_size as i64))
                .bind(("row_limit", limits.max_row_chars as i64))
                .await
                .map_err(|_| error("Couldn't prepare your account download."))?
                .check()
                .map_err(|_| error("Couldn't prepare your account download."))?
                .take(0)
                .map_err(|_| error("Couldn't prepare your account download."))?;
            let full = rows.len() == limits.page_size;
            for row in rows {
                if after.as_ref().is_some_and(|previous| row.id <= *previous) {
                    return Err(error("Your download couldn't be verified. Please retry."));
                }
                let mut data = row.data.ok_or_else(too_large)?;
                count += 1;
                bytes = bytes.saturating_add(
                    serde_json::to_vec(&data)
                        .map_err(|_| error("Couldn't prepare your account download."))?
                        .len()
                        + 1,
                );
                if count > limits.max_rows || bytes > limits.max_bytes {
                    return Err(too_large());
                }
                if name == "library_state" {
                    for (stored, label) in [("queue_json", "queue"), ("history_json", "history")] {
                        let raw = data.get(stored).and_then(Value::as_str).ok_or_else(|| {
                            error("Saved library data needs review by your server operator.")
                        })?;
                        let parsed: Value = serde_json::from_str(raw).map_err(|_| {
                            error("Saved library data needs review by your server operator.")
                        })?;
                        if !parsed.is_array() {
                            return Err(error(
                                "Saved library data needs review by your server operator.",
                            ));
                        }
                        let object = data
                            .as_object_mut()
                            .ok_or_else(|| error("Couldn't prepare your account download."))?;
                        object.remove(stored);
                        object.insert(label.into(), parsed);
                    }
                }
                after = Some(row.id);
                values.push(data);
            }
            if !full {
                break;
            }
        }
        sections.insert(name, values);
    }
    // This is a live collection, not a database snapshot. Do not report success
    // for an account that disappeared while its pages were being collected.
    crate::auth::Accounts::new(db)
        .find(&owner.0)
        .await
        .map_err(|_| error("Your account is no longer available."))?;
    let body = serde_json::to_string_pretty(&json!({
        "format": "tawny-account-data", "schema_version": 1,
        "collection_started_at_unix": started, "collection_finished_at_unix": crate::brand::timestamp(),
        "profile": profile, "sections": sections,
        "notes": [
            "Collected live between the stated times; concurrent edits may require another download.",
            "Includes this server's account profile, library choices, progress, relations and legal receipts. Public YouTube metadata is represented by reference IDs.",
            "Passwords, session/cookie payloads, operational credentials and unrelated accounts are excluded.",
            "Device settings/caches, prior downloads, operational logs, processor records and backup copies need separate handling by you or the server operator."
        ]
    })).map_err(|_| error("Couldn't prepare your account download."))?;
    if body.len() > limits.max_bytes {
        return Err(too_large());
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn missing_or_switched_review_is_rejected_before_database_access() {
        let db = Surreal::<Any>::init();
        for expected in [None, Some("app_user:other")] {
            assert!(
                export(
                    &db,
                    &Owner("app_user:current".into()),
                    expected,
                    Limits::default()
                )
                .await
                .is_err()
            );
        }
    }

    #[tokio::test]
    async fn download_pages_only_the_owner_and_excludes_credentials_and_future_fields() {
        let state = crate::server::AppServerState::initialize().await.unwrap();
        let first = Owner(state.accounts().create_guest().await.unwrap().id);
        let second = Owner(state.accounts().create_guest().await.unwrap().id);
        state
            .db
            .query(
                "DEFINE FIELD IF NOT EXISTS future_private_marker ON app_user TYPE option<string>;",
            )
            .await
            .unwrap()
            .check()
            .unwrap();
        state.db.query("UPDATE type::record($owner) SET email = 'first@example.invalid', password_hash = 'credential-canary', future_private_marker = 'future-field-canary'; CREATE sessions:export_canary SET sessionid = 'session-secret-canary', sessionstore = 'cookie-secret-canary', sessionexpires = '4102444800', account_indexed = true; CREATE channel:export_shared SET channel_id = 'UCSynthetic', name = 'Public fixture', handle = '@fixture';")
            .bind(("owner", first.0.clone())).await.unwrap().check().unwrap();
        state
            .db
            .query("UPDATE type::record($owner) SET email = 'second@example.invalid';")
            .bind(("owner", second.0.clone()))
            .await
            .unwrap()
            .check()
            .unwrap();
        for (owner, marker) in [(&first, "owned"), (&second, "other-private-canary")] {
            state.seed_new_account(&owner.0).await.unwrap();
            state.set_queue(&owner.0, &[marker.into()]).await.unwrap();
            state.db.query("CREATE user_subscription SET owner = type::record($owner), channel_id = $marker; CREATE subscription_group SET owner = type::record($owner), group_id = $marker, name = $marker; CREATE video_progress SET owner = type::record($owner), video_id = $marker; CREATE brand_legal_receipt SET app = 'tawny', owner = type::record($owner), subject = 'synthetic-subject', document_set_digest = $marker, action = 'acknowledged', purpose = '', created_at = time::now(); LET $user = type::record($owner); RELATE $user->subscription->channel:export_shared;")
                .bind(("owner", owner.0.clone())).bind(("marker", marker)).await.unwrap().check().unwrap();
            state.db.query("LET $playlist = array::first(SELECT VALUE id FROM playlist WHERE owner = type::record($owner)); RELATE $playlist->playlist_item->video:synthetic SET position = 0;")
                .bind(("owner", owner.0.clone())).await.unwrap().check().unwrap();
        }
        let body = export(
            &state.db,
            &first,
            Some(&first.0),
            Limits {
                page_size: 1,
                ..Limits::default()
            },
        )
        .await
        .unwrap();
        let value: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(value["profile"]["email"], "first@example.invalid");
        assert_eq!(value["sections"]["library_state"][0]["queue"][0], "owned");
        for name in [
            "playlists",
            "playlist_items",
            "subscription_groups",
            "library_state",
            "video_progress",
            "subscriptions",
            "legacy_subscription_links",
            "legal_receipts",
        ] {
            assert!(
                !value["sections"][name].as_array().unwrap().is_empty(),
                "Missing {name}"
            );
        }
        for forbidden in [
            "second@example.invalid",
            "other-private-canary",
            "credential-canary",
            "future-field-canary",
            "session-secret-canary",
            "cookie-secret-canary",
            "synthetic-subject",
        ] {
            assert!(!body.contains(forbidden), "Leaked {forbidden}");
        }
    }

    #[tokio::test]
    async fn limits_and_malformed_library_data_fail_without_partial_downloads() {
        let state = crate::server::AppServerState::initialize().await.unwrap();
        let owner = Owner(state.accounts().create_guest().await.unwrap().id);
        state.seed_new_account(&owner.0).await.unwrap();
        state
            .set_queue(&owner.0, &["fixture".into()])
            .await
            .unwrap();
        for limits in [
            Limits {
                max_rows: 0,
                ..Limits::default()
            },
            Limits {
                max_bytes: 32,
                ..Limits::default()
            },
            Limits {
                page_size: 0,
                ..Limits::default()
            },
        ] {
            assert!(
                export(&state.db, &owner, Some(&owner.0), limits)
                    .await
                    .is_err()
            );
        }
        state
            .db
            .query("UPDATE playlist SET description = $large WHERE owner = type::record($owner);")
            .bind(("owner", owner.0.clone()))
            .bind(("large", "x".repeat(4096)))
            .await
            .unwrap()
            .check()
            .unwrap();
        let result = export(
            &state.db,
            &owner,
            Some(&owner.0),
            Limits {
                max_row_chars: 1024,
                ..Limits::default()
            },
        )
        .await;
        assert!(result.unwrap_err().to_string().contains("assisted export"));
        state.db.query("UPDATE playlist SET description = '' WHERE owner = type::record($owner); UPDATE library_state SET queue_json = 'not-json' WHERE owner = type::record($owner);")
            .bind(("owner", owner.0.clone())).await.unwrap().check().unwrap();
        assert!(
            export(&state.db, &owner, Some(&owner.0), Limits::default())
                .await
                .unwrap_err()
                .to_string()
                .contains("needs review")
        );
        state
            .accounts()
            .delete(&owner, Some(&owner.0))
            .await
            .unwrap();
        assert!(
            export(&state.db, &owner, Some(&owner.0), Limits::default())
                .await
                .is_err()
        );
    }

    #[test]
    fn every_owner_table_has_an_explicit_export_projection() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("database/schema");
        for entry in std::fs::read_dir(root).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|value| value.to_str()) != Some("surql") {
                continue;
            }
            let schema = std::fs::read_to_string(&path).unwrap();
            if schema.contains("DEFINE FIELD IF NOT EXISTS owner ON") {
                let table = path.file_stem().unwrap().to_str().unwrap();
                assert!(
                    SECTIONS
                        .iter()
                        .any(|(_, candidate, _, _)| *candidate == table),
                    "Review export scope for {table}"
                );
            }
        }
    }
}
