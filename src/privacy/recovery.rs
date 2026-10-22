//! Optional durable deletion queue. Exact independent-ledger acknowledgment
//! precedes primary erasure; outages keep the saved request pending.
use g3_brand::privacy::{PrivacyKind, RecoveryEvent};
use std::{
    future::Future,
    sync::{Arc, OnceLock},
    time::Duration,
};
use surrealdb::{Surreal, engine::any::Any};
use surrealdb_types::{RecordId, SurrealValue, ToSql};

type Result<T> = std::result::Result<T, &'static str>;
static CONFIG: OnceLock<Option<Configuration>> = OnceLock::new();

// Never Debug: the transport token is server-only credential material.
#[derive(Clone)]
struct Configuration {
    endpoint: String,
    token: String,
}
impl Configuration {
    fn new(endpoint: String, token: String) -> Result<Self> {
        let url = reqwest::Url::parse(&endpoint).map_err(|_| "Invalid recovery configuration")?;
        let secure = url.scheme() == "https"
            || (url.scheme() == "http"
                && matches!(
                    url.host_str(),
                    Some("privacy-ledger" | "localhost" | "127.0.0.1")
                ));
        if !secure
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.path() != "/events"
            || !(32..=4096).contains(&token.len())
            || !token.bytes().all(|byte| byte.is_ascii_graphic())
        {
            return Err("Invalid recovery configuration");
        }
        Ok(Self { endpoint, token })
    }
    fn read(read: impl Fn(&str) -> Option<String>) -> Result<Option<Self>> {
        match read("G3_PRIVACY_RECOVERY").as_deref().unwrap_or("disabled") {
            "disabled" => Ok(None),
            "required" => Self::new(
                read("G3_PRIVACY_LEDGER_URL").unwrap_or_default(),
                read("G3_PRIVACY_LEDGER_TOKEN").unwrap_or_default(),
            )
            .map(Some),
            _ => Err("Invalid privacy recovery mode"),
        }
    }
    async fn record(&self, event: RecoveryEvent) -> Result<String> {
        tokio::time::timeout(
            Duration::from_secs(30),
            g3_brand::client::record_recovery_event(&self.endpoint, &self.token, &event),
        )
        .await
        .map_err(|_| "ledger_unavailable")?
        .map_err(|_| "ledger_unavailable")
    }
}

pub(crate) fn enabled() -> Result<bool> {
    CONFIG
        .get()
        .map(Option::is_some)
        .ok_or("Recovery is not initialized")
}

pub(crate) async fn enqueue(db: &Surreal<Any>, owner: &crate::auth::Owner) -> Result<String> {
    let account = RecordId::parse_simple(&owner.0).map_err(|_| "Account unavailable")?;
    if account.table.as_str() != "app_user" {
        return Err("Account unavailable");
    }
    let id = uuid::Uuid::new_v4().to_string();
    let subject = g3_brand::model::digest(account.to_sql().as_bytes());
    for attempt in 0..3_u64 {
        let result = db
            .query(
                r#"
BEGIN TRANSACTION;
fn::privacy_active_owner_v1($account);
LET $fence = type::record('auth_session_subject', <string>$account);
UPSERT $fence SET account = $account, retired = retired ?? false, touched_at = time::now();
IF (SELECT VALUE retired FROM ONLY $fence) { THROW 'Account unavailable'; };
LET $existing = SELECT VALUE id FROM privacy_request WHERE subject = $subject LIMIT 1;
IF array::len($existing) = 0 {
    UPDATE $fence SET privacy_version = (privacy_version ?? 0) + 1;
    CREATE type::record('privacy_request', $request_id) SET app = 'tawny', request_id = $request_id,
        subject = $subject, subject_version = (SELECT VALUE privacy_version FROM ONLY $fence),
        account = $account, state = 'verified';
};
COMMIT TRANSACTION;
"#,
            )
            .bind(("account", account.clone()))
            .bind(("subject", subject.clone()))
            .bind(("request_id", id.clone()))
            .await
            .and_then(|response| response.check());
        if result.is_ok() {
            break;
        }
        if attempt == 2 {
            return Err("Could not save deletion request");
        }
        tokio::time::sleep(Duration::from_millis(25 * (attempt + 1))).await;
    }
    let mut ids: Vec<String> = db.query("SELECT VALUE request_id FROM privacy_request WHERE subject = $subject AND (account = $account OR (state = 'primary_verified' AND account = NONE)) LIMIT 1")
        .bind(("subject", subject)).bind(("account", account)).await.and_then(|response| response.check())
        .map_err(|_| "Request read-back failed")?.take(0).map_err(|_| "Request read-back failed")?;
    let saved = ids.pop().ok_or("Request read-back failed")?;
    uuid::Uuid::parse_str(&saved).map_err(|_| "Request read-back failed")?;
    Ok(saved)
}

// No Debug/Serialize: internal identifiers and state do not enter public logs.
#[derive(SurrealValue)]
struct Job {
    id: RecordId,
    app: String,
    request_id: String,
    subject: String,
    subject_version: i64,
    account: Option<RecordId>,
    state: String,
    recovery_digest: Option<String>,
    attempt_count: i64,
}
impl Job {
    fn event(&self) -> Result<RecoveryEvent> {
        let event = RecoveryEvent {
            schema_version: 1,
            app: self.app.clone(),
            request_id: self.request_id.clone(),
            subject: self.subject.clone(),
            subject_version: self
                .subject_version
                .try_into()
                .map_err(|_| "invalid_record")?,
            action: PrivacyKind::Delete,
            purpose: None,
            corrections: None,
        };
        event.validate().map_err(|_| "invalid_record")?;
        if self.app != "tawny"
            || uuid::Uuid::parse_str(&self.request_id).is_err()
            || self.id != RecordId::new("privacy_request", self.request_id.clone())
        {
            return Err("invalid_record");
        }
        Ok(event)
    }
}

async fn claim(db: &Surreal<Any>, lease: &str) -> Result<Option<Job>> {
    let mut ids: Vec<RecordId> = db.query("SELECT VALUE id FROM privacy_request WHERE state != 'primary_verified' AND next_attempt_at <= time::now() AND (lease_until = NONE OR lease_until < time::now()) ORDER BY id LIMIT 1")
        .await.and_then(|response| response.check()).map_err(|_| "invalid_record")?.take(0).map_err(|_| "invalid_record")?;
    let Some(id) = ids.pop() else {
        return Ok(None);
    };
    let mut rows: Vec<Job> = db.query("UPDATE $id SET lease_owner = $lease, lease_until = time::now() + 5m, attempt_count = math::min([attempt_count + 1, 20]) WHERE state != 'primary_verified' AND (lease_until = NONE OR lease_until < time::now()) RETURN AFTER")
        .bind(("id", id)).bind(("lease", lease.to_owned())).await.and_then(|response| response.check())
        .map_err(|_| "lease_lost")?.take(0).map_err(|_| "invalid_record")?;
    Ok(rows.pop())
}

pub(super) const REQUEST_FIELDS: &[&str] = &[
    "id",
    "app",
    "request_id",
    "subject",
    "subject_version",
    "account",
    "state",
    "recovery_digest",
    "created_at",
    "updated_at",
    "primary_verified_at",
    "next_attempt_at",
    "attempt_count",
    "lease_owner",
    "lease_until",
    "last_error_code",
];

fn reviewed_payload(value: &serde_json::Value) -> bool {
    value.as_object().is_some_and(|object| {
        object
            .keys()
            .all(|key| REQUEST_FIELDS.contains(&key.as_str()))
    })
}

const PURGE: &str = concat!(
    r#"
BEGIN TRANSACTION;
LET $leased = SELECT VALUE id FROM $job WHERE lease_owner = $lease AND lease_until > time::now()
    AND state = 'recovery_recorded' AND recovery_digest = $digest AND account = type::record($owner);
IF array::len($leased) != 1 { THROW 'Request lease changed'; };
LET $request = (SELECT * FROM ONLY $job);
IF array::len(array::complement(object::keys($request), $reviewed_request_fields)) != 0 { THROW 'Request fields require review'; };
LET $profile = (SELECT * FROM ONLY type::record($owner));
IF $profile = NONE { THROW 'Account requires review'; };
IF array::len(array::complement(object::keys($profile), ['id', 'display_name', 'email', 'password_hash', 'is_guest', 'created_at', 'updated_at', 'privacy_revision'])) != 0 { THROW 'Account fields require review'; };
"#,
    crate::privacy::primary::erase_primary_sql!(),
    r#"
LET $changed = (UPDATE $job SET state = 'primary_removed', updated_at = time::now()
    WHERE lease_owner = $lease AND lease_until > time::now() AND state = 'recovery_recorded' AND recovery_digest = $digest RETURN VALUE id);
IF array::len($changed) != 1 { THROW 'Request lease changed'; };
COMMIT TRANSACTION;
"#
);

pub(super) async fn verify_primary(db: &Surreal<Any>, account: &RecordId) -> Result<()> {
    let mut response = db
        .query(
            r#"
RETURN count(SELECT id FROM $account)
 + count(SELECT id FROM playlist WHERE owner = $account)
 + count(SELECT id FROM library_state WHERE owner = $account)
 + count(SELECT id FROM subscription_group WHERE owner = $account)
 + count(SELECT id FROM user_subscription WHERE owner = $account)
 + count(SELECT id FROM video_progress WHERE owner = $account)
 + count(SELECT id FROM brand_legal_receipt WHERE owner = $account)
 + count(SELECT id FROM subscription WHERE in = $account)
 + count(SELECT id FROM sessions WHERE session_account = $account);
SELECT VALUE retired FROM ONLY type::record('auth_session_subject', <string>$account);
"#,
        )
        .bind(("account", account.clone()))
        .await
        .and_then(|response| response.check())
        .map_err(|_| "primary_pending")?;
    if response
        .take::<Option<i64>>(0)
        .map_err(|_| "primary_pending")?
        != Some(0)
        || response
            .take::<Option<bool>>(1)
            .map_err(|_| "session_pending")?
            != Some(true)
    {
        return Err("primary_pending");
    }
    Ok(())
}

async fn process<L, F>(db: Arc<Surreal<Any>>, job: &Job, lease: &str, record: L) -> Result<()>
where
    L: Fn(RecoveryEvent) -> F,
    F: Future<Output = Result<String>>,
{
    let raw: Option<serde_json::Value> = db
        .select(job.id.clone())
        .await
        .map_err(|_| "invalid_record")?;
    if !raw.as_ref().is_some_and(reviewed_payload) {
        return Err("invalid_record");
    }
    let account = job.account.clone().ok_or("invalid_record")?;
    if account.table.as_str() != "app_user"
        || g3_brand::model::digest(account.to_sql().as_bytes()) != job.subject
    {
        return Err("invalid_record");
    }
    let leased: Option<i64> = db.query("RETURN count(SELECT id FROM $id WHERE lease_owner = $lease AND lease_until > time::now() AND state != 'primary_verified')")
        .bind(("id", job.id.clone())).bind(("lease", lease.to_owned())).await.and_then(|response| response.check())
        .map_err(|_| "lease_lost")?.take(0).map_err(|_| "lease_lost")?;
    if leased != Some(1) {
        return Err("lease_lost");
    }
    let event = job.event()?;
    let expected =
        g3_brand::model::digest(&serde_json::to_vec(&event).map_err(|_| "invalid_record")?);
    match &job.recovery_digest {
        Some(value) if value != &expected || job.state == "verified" => {
            return Err("invalid_record");
        }
        Some(_) => {}
        None => {
            if job.state != "verified" {
                return Err("invalid_record");
            }
            if record(event).await? != expected {
                return Err("ledger_unavailable");
            }
            let ids: Vec<RecordId> = db.query("UPDATE $id SET recovery_digest = $digest, state = 'recovery_recorded', updated_at = time::now() WHERE lease_owner = $lease AND lease_until > time::now() AND state = 'verified' RETURN VALUE id")
                .bind(("id", job.id.clone())).bind(("lease", lease.to_owned())).bind(("digest", expected.clone()))
                .await.and_then(|response| response.check()).map_err(|_| "lease_lost")?.take(0).map_err(|_| "lease_lost")?;
            if ids.len() != 1 {
                return Err("lease_lost");
            }
        }
    }
    if job.state != "primary_removed" {
        db.query(PURGE)
            .bind((
                "reviewed_request_fields",
                REQUEST_FIELDS
                    .iter()
                    .map(|field| (*field).to_owned())
                    .collect::<Vec<_>>(),
            ))
            .bind(("job", job.id.clone()))
            .bind(("lease", lease.to_owned()))
            .bind(("digest", expected.clone()))
            .bind(("owner", account.to_sql()))
            .await
            .and_then(|response| response.check())
            .map_err(|_| "review_required")?;
    }
    crate::auth::session_pool(Arc::clone(&db))
        .retire_account_sessions("sessions", &account)
        .await
        .map_err(|_| "session_pending")?;
    verify_primary(&db, &account).await?;
    let ids: Vec<RecordId> = db.query("UPDATE $id SET state = 'primary_verified', account = NONE, primary_verified_at = time::now(), updated_at = time::now(), lease_owner = NONE, lease_until = NONE, last_error_code = NONE WHERE lease_owner = $lease AND lease_until > time::now() AND state = 'primary_removed' AND recovery_digest = $digest RETURN VALUE id")
        .bind(("id", job.id.clone())).bind(("lease", lease.to_owned())).bind(("digest", expected))
        .await.and_then(|response| response.check()).map_err(|_| "lease_lost")?.take(0).map_err(|_| "lease_lost")?;
    if ids.len() != 1 {
        return Err("lease_lost");
    }
    Ok(())
}

async fn retry(db: &Surreal<Any>, job: &Job, lease: &str, error: &'static str) -> Result<()> {
    let code = match error {
        "ledger_unavailable" | "primary_pending" | "session_pending" | "review_required"
        | "lease_lost" => error,
        _ => "invalid_record",
    };
    let delay = 5_i64
        .saturating_mul(1_i64 << job.attempt_count.clamp(0, 10))
        .min(3600);
    db.query("UPDATE $id SET last_error_code = $code, next_attempt_at = time::now() + duration::from_secs($delay), lease_owner = NONE, lease_until = NONE, updated_at = time::now() WHERE lease_owner = $lease AND lease_until > time::now() AND state != 'primary_verified'")
        .bind(("id", job.id.clone())).bind(("lease", lease.to_owned())).bind(("code", code)).bind(("delay", delay))
        .await.and_then(|response| response.check()).map_err(|_| "lease_lost")?;
    Ok(())
}

async fn require_empty_when_disabled(db: &Surreal<Any>, disabled: bool) -> Result<()> {
    if disabled {
        let pending: Option<i64> = db
            .query("RETURN count(SELECT id FROM privacy_request WHERE state != 'primary_verified')")
            .await
            .and_then(|response| response.check())
            .map_err(|_| "Recovery status unavailable")?
            .take(0)
            .map_err(|_| "Recovery status unavailable")?;
        if pending != Some(0) {
            return Err("Do not disable recovery over pending requests");
        }
    }
    Ok(())
}

pub(crate) async fn start(db: Arc<Surreal<Any>>) -> Result<()> {
    let configured = Configuration::read(|key| std::env::var(key).ok())?;
    require_empty_when_disabled(&db, configured.is_none()).await?;
    let config = CONFIG.get_or_init(|| configured);
    let Some(config) = config.clone() else {
        return Ok(());
    };
    tokio::spawn(async move {
        let lease = uuid::Uuid::new_v4().to_string();
        loop {
            match claim(&db, &lease).await {
                Ok(Some(job)) => {
                    if let Err(code) =
                        process(Arc::clone(&db), &job, &lease, |event| config.record(event)).await
                    {
                        let _ = retry(&db, &job, &lease, code).await;
                        crate::telemetry::diagnostic(
                            crate::telemetry::Diagnostic::PrivacyRequestPending,
                        );
                    }
                }
                Ok(None) => {}
                Err(_) => crate::telemetry::diagnostic(
                    crate::telemetry::Diagnostic::PrivacyRequestPending,
                ),
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    });
    Ok(())
}

#[cfg(test)]
pub(crate) async fn exercise_editor_queue(db: Arc<Surreal<Any>>) {
    let accounts = crate::auth::Accounts::new(&db);
    let owner = crate::auth::Owner(accounts.create_guest().await.unwrap().id);
    let other = accounts.create_guest().await.unwrap();
    db.query("CREATE library_state SET owner = type::record($owner), queue_json = '[\"private-wire-fixture\"]';")
        .bind(("owner", owner.0.clone())).await.unwrap().check().unwrap();
    let id = enqueue(&db, &owner).await.unwrap();
    assert_eq!(enqueue(&db, &owner).await.unwrap(), id);
    assert!(accounts.delete(&owner, Some(&owner.0)).await.is_err());
    assert!(
        db.query("DELETE type::record($owner)")
            .bind(("owner", owner.0.clone()))
            .await
            .unwrap()
            .check()
            .is_err()
    );
    let job = claim(&db, "wire-worker").await.unwrap().unwrap();
    process(Arc::clone(&db), &job, "wire-worker", |event| async move {
        Ok(g3_brand::model::digest(
            &serde_json::to_vec(&event).unwrap(),
        ))
    })
    .await
    .unwrap();
    assert!(accounts.find(&owner.0).await.is_err());
    assert!(accounts.find(&other.id).await.is_ok());
    verify_primary(&db, job.account.as_ref().unwrap())
        .await
        .unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{auth::Owner, server::AppServerState};

    async fn queued() -> (AppServerState, Owner, Owner, Job) {
        let state = AppServerState::initialize().await.unwrap();
        let owner = Owner(state.accounts().create_guest().await.unwrap().id);
        let other = Owner(state.accounts().create_guest().await.unwrap().id);
        state.seed_new_account(&owner.0).await.unwrap();
        state.seed_new_account(&other.0).await.unwrap();
        state
            .set_queue(&owner.0, &["private-owner-canary".into()])
            .await
            .unwrap();
        let id = enqueue(&state.db, &owner).await.unwrap();
        assert_eq!(enqueue(&state.db, &owner).await.unwrap(), id);
        let job = claim(&state.db, "fixture").await.unwrap().unwrap();
        assert_eq!(job.request_id, id);
        (state, owner, other, job)
    }
    async fn ack(event: RecoveryEvent) -> Result<String> {
        Ok(g3_brand::model::digest(
            &serde_json::to_vec(&event).unwrap(),
        ))
    }
    async fn release(db: &Surreal<Any>, job: &Job) {
        db.query(
            "UPDATE $id SET lease_owner = NONE, lease_until = NONE, next_attempt_at = time::now();",
        )
        .bind(("id", job.id.clone()))
        .await
        .unwrap()
        .check()
        .unwrap();
    }

    #[tokio::test]
    async fn stable_intent_blocks_legacy_purge_and_an_unknown_ledger_receipt() {
        let (state, owner, other, job) = queued().await;
        assert!(
            state
                .accounts()
                .delete(&owner, Some(&owner.0))
                .await
                .is_err()
        );
        assert!(
            state
                .db
                .query("DELETE type::record($owner)")
                .bind(("owner", owner.0.clone()))
                .await
                .unwrap()
                .check()
                .is_err()
        );
        assert!(require_empty_when_disabled(&state.db, true).await.is_err());
        assert!(
            state
                .accounts()
                .register(
                    &owner,
                    Some(&owner.0),
                    "queued@example.invalid",
                    "long fixture password"
                )
                .await
                .is_err()
        );
        assert!(state.accounts().find(&owner.0).await.unwrap().is_guest);
        for change in [
            "subject_version = 2",
            "state = 'primary_verified'",
            "account = NONE",
        ] {
            assert!(
                state
                    .db
                    .query(format!("UPDATE $id SET {change}"))
                    .bind(("id", job.id.clone()))
                    .await
                    .unwrap()
                    .check()
                    .is_err()
            );
        }
        let event = job.event().unwrap();
        let event_json = serde_json::to_string(&event).unwrap();
        assert!(!event_json.contains(&owner.0) && !event_json.contains("private-owner-canary"));
        assert_eq!(event.subject_version, 1);
        assert_eq!(
            process(Arc::clone(&state.db), &job, "fixture", |_| async {
                Err("ledger_unavailable")
            })
            .await,
            Err("ledger_unavailable")
        );
        assert_eq!(
            process(Arc::clone(&state.db), &job, "fixture", |_| async {
                Ok("wrong-digest".into())
            })
            .await,
            Err("ledger_unavailable")
        );
        assert!(state.accounts().find(&owner.0).await.is_ok());
        assert!(state.accounts().find(&other.0).await.is_ok());
        retry(
            &state.db,
            &job,
            "fixture",
            "unreviewed-private-error-canary",
        )
        .await
        .unwrap();
        let code: Option<String> = state
            .db
            .query("SELECT VALUE last_error_code FROM ONLY $id")
            .bind(("id", job.id))
            .await
            .unwrap()
            .check()
            .unwrap()
            .take(0)
            .unwrap();
        assert_eq!(code.as_deref(), Some("invalid_record"));
    }

    #[tokio::test]
    async fn exact_acknowledgment_removes_only_the_owner_and_clears_the_private_reference() {
        use axum_session::DatabasePool;
        let (state, owner, other, job) = queued().await;
        let pool = crate::auth::session_pool(Arc::clone(&state.db));
        let payload = serde_json::json!({"data":{"user_auth_session_id":serde_json::to_string(owner.0.strip_prefix("app_user:").unwrap()).unwrap()}}).to_string();
        pool.store(
            "before-removal",
            &payload,
            time::OffsetDateTime::now_utc().unix_timestamp() + 3600,
            "sessions",
        )
        .await
        .unwrap();
        process(Arc::clone(&state.db), &job, "fixture", ack)
            .await
            .unwrap();
        assert!(state.accounts().find(&owner.0).await.is_err());
        assert!(state.accounts().find(&other.0).await.is_ok());
        verify_primary(&state.db, job.account.as_ref().unwrap())
            .await
            .unwrap();
        let finished: Job = state.db.select(job.id.clone()).await.unwrap().unwrap();
        assert_eq!(finished.state, "primary_verified");
        assert!(finished.account.is_none());
        assert!(finished.recovery_digest.is_some());
        assert!(
            pool.load("before-removal", "sessions")
                .await
                .unwrap()
                .is_none()
        );
        pool.store(
            "delayed-removal",
            &payload,
            time::OffsetDateTime::now_utc().unix_timestamp() + 3600,
            "sessions",
        )
        .await
        .unwrap();
        assert!(
            pool.load("delayed-removal", "sessions")
                .await
                .unwrap()
                .is_none()
        );
        assert!(state.set_queue(&owner.0, &["late".into()]).await.is_err());
        assert!(require_empty_when_disabled(&state.db, true).await.is_ok());
        assert!(claim(&state.db, "later").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn persisted_ack_and_primary_checkpoint_resume_without_recording_a_new_event() {
        for already_removed in [false, true] {
            let (state, owner, other, job) = queued().await;
            let digest = ack(job.event().unwrap()).await.unwrap();
            state
                .db
                .query("UPDATE $id SET recovery_digest = $digest, state = 'recovery_recorded'")
                .bind(("id", job.id.clone()))
                .bind(("digest", digest.clone()))
                .await
                .unwrap()
                .check()
                .unwrap();
            if already_removed {
                state
                    .db
                    .query(PURGE)
                    .bind((
                        "reviewed_request_fields",
                        REQUEST_FIELDS
                            .iter()
                            .map(|field| (*field).to_owned())
                            .collect::<Vec<_>>(),
                    ))
                    .bind(("job", job.id.clone()))
                    .bind(("lease", "fixture"))
                    .bind(("digest", digest))
                    .bind(("owner", owner.0.clone()))
                    .await
                    .unwrap()
                    .check()
                    .unwrap();
            }
            release(&state.db, &job).await;
            let restarted = claim(&state.db, "restarted").await.unwrap().unwrap();
            process(Arc::clone(&state.db), &restarted, "restarted", |_| async {
                panic!("Durable receipt must be reused")
            })
            .await
            .unwrap();
            assert!(state.accounts().find(&owner.0).await.is_err());
            assert!(state.accounts().find(&other.0).await.is_ok());
        }
    }

    #[tokio::test]
    async fn lost_leases_and_unknown_private_fields_hold_erasure() {
        let (state, owner, _, job) = queued().await;
        state
            .db
            .query("UPDATE $id SET lease_owner = 'replacement'")
            .bind(("id", job.id.clone()))
            .await
            .unwrap()
            .check()
            .unwrap();
        assert_eq!(
            process(Arc::clone(&state.db), &job, "fixture", |_| async {
                panic!("Lost worker must not contact ledger")
            })
            .await,
            Err("lease_lost")
        );
        release(&state.db, &job).await;
        let job = claim(&state.db, "fixture").await.unwrap().unwrap();
        state.db.query("DEFINE FIELD future_private_marker ON app_user TYPE option<string>; UPDATE type::record($owner) SET future_private_marker = 'provider-grant-canary';")
            .bind(("owner", owner.0.clone())).await.unwrap().check().unwrap();
        assert_eq!(
            process(Arc::clone(&state.db), &job, "fixture", ack).await,
            Err("review_required")
        );
        assert!(state.accounts().find(&owner.0).await.is_ok());
        state.db.query("DEFINE FIELD future_private_marker ON privacy_request TYPE option<string>; UPDATE $id SET future_private_marker = 'future-job-canary';")
            .bind(("id", job.id.clone())).await.unwrap().check().unwrap();
        let current: Job = state.db.select(job.id.clone()).await.unwrap().unwrap();
        assert_eq!(
            process(Arc::clone(&state.db), &current, "fixture", ack).await,
            Err("invalid_record")
        );
        assert!(state.accounts().find(&owner.0).await.is_ok());
    }

    #[tokio::test]
    async fn fields_added_during_ledger_recording_hold_the_atomic_primary_transaction() {
        let (state, owner, other, job) = queued().await;
        let db = Arc::clone(&state.db);
        let id = job.id.clone();
        let result = process(Arc::clone(&state.db), &job, "fixture", move |event| {
            let db = Arc::clone(&db);
            let id = id.clone();
            async move {
                db.query("DEFINE FIELD unreviewed_processor ON privacy_request TYPE option<string>; UPDATE $id SET unreviewed_processor = 'future-processor-canary';")
                    .bind(("id", id)).await.unwrap().check().unwrap();
                ack(event).await
            }
        }).await;
        assert_eq!(result, Err("review_required"));
        assert!(state.accounts().find(&owner.0).await.is_ok());
        assert!(state.accounts().find(&other.0).await.is_ok());
    }

    #[tokio::test]
    async fn account_download_projects_only_the_callers_minimal_request_state() {
        let (state, owner, other, job) = queued().await;
        let other_request = enqueue(&state.db, &other).await.unwrap();
        state.db.query("DEFINE FIELD future_private_marker ON privacy_request TYPE option<string>; UPDATE $id SET recovery_digest = 'ledger-secret-canary', lease_owner = 'lease-secret-canary', future_private_marker = 'future-request-canary';")
            .bind(("id", job.id.clone())).await.unwrap().check().unwrap();
        let body = super::super::server::export(
            &state.db,
            &owner,
            Some(&owner.0),
            super::super::server::Limits {
                page_size: 1,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let value: serde_json::Value = serde_json::from_str(&body).unwrap();
        let requests = value["sections"]["privacy_requests"].as_array().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0]["request_id"], job.request_id);
        assert_eq!(requests[0]["state"], "verified");
        assert_eq!(requests[0].as_object().unwrap().len(), 4);
        for excluded in [
            other_request.as_str(),
            job.subject.as_str(),
            "ledger-secret-canary",
            "lease-secret-canary",
            "future-request-canary",
        ] {
            assert!(!body.contains(excluded), "Excluded request metadata leaked");
        }
    }

    #[test]
    fn configuration_is_explicit_and_does_not_allow_leaking_credentials_in_urls() {
        assert!(Configuration::read(|_| None).unwrap().is_none());
        assert!(
            Configuration::read(|key| (key == "G3_PRIVACY_RECOVERY").then(|| "Required".into()))
                .is_err()
        );
        for endpoint in [
            "http://example.invalid/events",
            "https://user:secret@example.invalid/events",
            "https://example.invalid/other",
            "https://example.invalid/events?token=secret",
        ] {
            assert!(Configuration::new(endpoint.into(), "x".repeat(32)).is_err());
        }
        assert!(
            Configuration::new("http://privacy-ledger:8081/events".into(), "x".repeat(32)).is_ok()
        );
        assert!(
            Configuration::new("https://example.invalid/events".into(), "short".into()).is_err()
        );
    }

    #[tokio::test]
    async fn scoped_http_ledger_transport_acknowledges_the_exact_event_only() {
        use axum::{Json, Router, routing::post};
        for valid in [true, false] {
            let (state, owner, _, job) = queued().await;
            let event = job.event().unwrap();
            let expected_event = event.clone();
            let router = Router::new().route("/events", post(move |headers: axum::http::HeaderMap, Json(received): Json<RecoveryEvent>| {
                let expected_event = expected_event.clone();
                async move {
                    assert_eq!(headers["authorization"], format!("Bearer {}", "x".repeat(32)));
                    assert_eq!(received, expected_event);
                    Json(serde_json::json!({"digest": if valid { ack(received).await.unwrap() } else { "wrong".into() }}))
                }
            }));
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let task = tokio::spawn(async move {
                axum::serve(listener, router).await.unwrap();
            });
            let config =
                Configuration::new(format!("http://{address}/events"), "x".repeat(32)).unwrap();
            let result = process(Arc::clone(&state.db), &job, "fixture", |event| {
                config.record(event)
            })
            .await;
            task.abort();
            assert_eq!(result.is_ok(), valid);
            assert_eq!(state.accounts().find(&owner.0).await.is_err(), valid);
        }
    }
}
