//! Accounts and cookie-backed Axum sessions.
//!
//! First launch creates a guest account and logs it into an HTTP-only session
//! cookie. Registration promotes that same account; sign-in replaces the
//! session's user. No credential is kept in Tawny's JavaScript state.
//!
//! Sessions, the guard and the signed-in user come from `g3-auth`; this module
//! holds what is Tawny's own: the accounts themselves.

use anyhow::{Result, anyhow};
use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng},
};
use axum::{
    extract::FromRequestParts,
    http::{StatusCode, request::Parts},
};
use serde::Deserialize;
use surrealdb::{Surreal, engine::any::Any};
use surrealdb_types::SurrealValue;

use crate::models::{Account, CredentialProblem, validate_email, validate_password};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthError {
    Credential(CredentialProblem),
    InvalidLogin,
    AlreadyRegistered,
}

impl AuthError {
    pub fn message(&self) -> String {
        match self {
            Self::Credential(problem) => problem.message().to_string(),
            Self::InvalidLogin => "That email and password do not match an account.".into(),
            Self::AlreadyRegistered => "This account already has an email address.".into(),
        }
    }
}

impl std::fmt::Display for AuthError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message())
    }
}

impl std::error::Error for AuthError {}

impl From<CredentialProblem> for AuthError {
    fn from(problem: CredentialProblem) -> Self {
        Self::Credential(problem)
    }
}

#[derive(Debug, Deserialize, SurrealValue)]
struct DbAccount {
    id: String,
    display_name: String,
    email: Option<String>,
    is_guest: Option<bool>,
}

impl From<DbAccount> for Account {
    fn from(row: DbAccount) -> Self {
        Self {
            id: row.id,
            display_name: row.display_name,
            is_guest: row.is_guest.unwrap_or(row.email.is_none()),
            email: row.email,
        }
    }
}

#[derive(Debug, Deserialize, SurrealValue)]
struct DbCredential {
    id: String,
    display_name: String,
    email: Option<String>,
    password_hash: Option<String>,
}

const ACCOUNT_COLUMNS: &str = "type::string(id) AS id, display_name, email, is_guest";

// Delete relations before their source records; the shared YouTube catalogue
// belongs to the server, not to this account.
const DELETE_ACCOUNT_QUERY: &str = r#"
    BEGIN TRANSACTION;
    LET $channels = SELECT VALUE channel_id FROM user_subscription WHERE owner = type::record($owner);
    DELETE playlist_item WHERE in IN (SELECT VALUE id FROM playlist WHERE owner = type::record($owner));
    DELETE playlist WHERE owner = type::record($owner);
    DELETE subscription_group WHERE owner = type::record($owner);
    DELETE library_state WHERE owner = type::record($owner);
    DELETE video_progress WHERE owner = type::record($owner);
    DELETE user_subscription WHERE owner = type::record($owner);
    DELETE subscription WHERE in = type::record($owner);
    FOR $channel_id IN $channels {
        UPDATE channel SET subscribed = count(SELECT id FROM user_subscription WHERE channel_id = $channel_id AND subscribed = true) > 0 WHERE channel_id = $channel_id;
    };
    DELETE type::record($owner);
    COMMIT TRANSACTION;
"#;

fn hash_password(password: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|error| anyhow!("could not hash password: {error}"))
}

fn password_matches(password: &str, encoded: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(encoded) else {
        return false;
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

pub struct Accounts<'db> {
    db: &'db Surreal<Any>,
}

impl<'db> Accounts<'db> {
    pub fn new(db: &'db Surreal<Any>) -> Self {
        Self { db }
    }

    pub async fn delete(&self, account_id: &str) -> Result<()> {
        self.db
            .query(DELETE_ACCOUNT_QUERY)
            .bind(("owner", account_id.to_string()))
            .await?
            .check()?;
        Ok(())
    }

    pub async fn create_guest(&self) -> Result<Account> {
        let created: Vec<DbAccount> = self
            .db
            .query(format!(
                r#"CREATE app_user SET
                    display_name = "Guest", email = NONE, password_hash = NONE, is_guest = true
                RETURN {ACCOUNT_COLUMNS}"#
            ))
            .await?
            .check()?
            .take(0)?;
        created
            .into_iter()
            .next()
            .map(Into::into)
            .ok_or_else(|| anyhow!("guest account was not created"))
    }

    pub async fn register(&self, account_id: &str, email: &str, password: &str) -> Result<Account> {
        let account = self.find(account_id).await?;
        if !account.is_guest || account.email.is_some() {
            return Err(AuthError::AlreadyRegistered.into());
        }
        let email = validate_email(email).map_err(AuthError::from)?;
        validate_password(password).map_err(AuthError::from)?;
        if self.email_is_taken(&email).await? {
            return Err(AuthError::Credential(CredentialProblem::EmailTaken).into());
        }

        let password = password.to_string();
        let password_hash = tokio::task::spawn_blocking(move || hash_password(&password)).await??;
        let updated: Vec<DbAccount> = self
            .db
            .query(format!(
                r#"UPDATE type::record($id) SET
                    email = $email, password_hash = $password_hash, display_name = $email,
                    is_guest = false, updated_at = time::now()
                RETURN {ACCOUNT_COLUMNS}"#
            ))
            .bind(("id", account.id))
            .bind(("email", email))
            .bind(("password_hash", password_hash))
            .await?
            .check()
            .map_err(|_| AuthError::Credential(CredentialProblem::EmailTaken))?
            .take(0)?;
        updated
            .into_iter()
            .next()
            .map(Into::into)
            .ok_or_else(|| anyhow!("account row vanished during sign-up"))
    }

    pub async fn sign_in(&self, email: &str, password: &str) -> Result<Account> {
        let candidates: Vec<DbCredential> = self
            .db
            .query(format!(
                "SELECT {ACCOUNT_COLUMNS}, password_hash FROM app_user WHERE email = $email LIMIT 1"
            ))
            .bind(("email", email.trim().to_lowercase()))
            .await?
            .check()?
            .take(0)?;
        let Some(candidate) = candidates.into_iter().next() else {
            return Err(AuthError::InvalidLogin.into());
        };
        let Some(encoded) = candidate.password_hash else {
            return Err(AuthError::InvalidLogin.into());
        };
        let password = password.to_string();
        if !tokio::task::spawn_blocking(move || password_matches(&password, &encoded)).await? {
            return Err(AuthError::InvalidLogin.into());
        }
        Ok(Account {
            id: candidate.id,
            display_name: candidate.display_name,
            email: candidate.email,
            is_guest: false,
        })
    }

    pub async fn find(&self, account_id: &str) -> Result<Account> {
        let accounts: Vec<DbAccount> = self
            .db
            .query(format!(
                "SELECT {ACCOUNT_COLUMNS} FROM app_user WHERE id = type::record($id) LIMIT 1"
            ))
            .bind(("id", account_id.to_string()))
            .await?
            .check()?
            .take(0)?;
        accounts
            .into_iter()
            .next()
            .map(Into::into)
            .ok_or_else(|| anyhow!("account not found"))
    }

    async fn email_is_taken(&self, email: &str) -> Result<bool> {
        let existing: Vec<DbAccount> = self
            .db
            .query(format!(
                "SELECT {ACCOUNT_COLUMNS} FROM app_user WHERE email = $email LIMIT 1"
            ))
            .bind(("email", email.to_string()))
            .await?
            .check()?
            .take(0)?;
        Ok(!existing.is_empty())
    }
}

/// Tawny's account table, as g3-auth sees it: sessions resolve to an
/// `app_user` row, named by its `display_name`.
pub enum AppUser {}

impl g3_auth::AuthUser for AppUser {
    const TABLE: &'static str = "app_user";
}

/// The database, the auth session and the resolved user, for server functions
/// that sign in or out.
pub type SessionContext = g3_auth::SessionContext<AppUser, Any>;
pub type AuthSession = g3_auth::AuthSession<AppUser, Any>;

pub fn account_rate_limits() -> g3_auth::rate_limit::RateLimits {
    use g3_auth::rate_limit::{KeyBy, Limit, RateLimits};
    RateLimits::new()
        .client_ip_header_from_env("CLIENT_IP_HEADER")
        .limit(Limit::per_hour(
            &["/api/v1/auth/guest"],
            KeyBy::ClientIp,
            10,
        ))
        .limit(Limit::per_hour(
            &["/api/v1/auth/register"],
            KeyBy::ClientIp,
            10,
        ))
        .limit(Limit::per_minute(
            &["/api/v1/auth/sign-in"],
            KeyBy::ClientIp,
            5,
        ))
        // Dioxus does not expose the peer address. This bounds work even when
        // no trusted proxy header exists or an attacker rotates cookies.
        .limit(Limit::per_minute(
            &[
                "/api/v1/auth/guest",
                "/api/v1/auth/register",
                "/api/v1/auth/sign-in",
            ],
            KeyBy::Global,
            60,
        ))
}

/// The signed-in account as the `app_user:<key>` record id that every
/// owner-scoped query binds with `type::record($owner)`.
///
/// The guard already refused a request without a session before it got here;
/// this still refuses one, because a server function called during
/// server-side rendering runs without any middleware.
pub struct Owner(pub String);

impl<S> FromRequestParts<S> for Owner
where
    S: Send + Sync,
{
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(
        parts: &mut Parts,
        _state: &S,
    ) -> std::result::Result<Self, Self::Rejection> {
        let auth_session = parts.extensions.get::<AuthSession>().ok_or((
            StatusCode::INTERNAL_SERVER_ERROR,
            "auth session middleware is missing",
        ))?;
        auth_session
            .current_user
            .as_ref()
            .filter(|user| !user.anonymous)
            .map(|user| {
                Self(format!(
                    "{}:{}",
                    <AppUser as g3_auth::AuthUser>::TABLE,
                    user.id
                ))
            })
            .ok_or((
                StatusCode::UNAUTHORIZED,
                "your session has expired; please sign in again",
            ))
    }
}

/// Log `account` into this session and keep it past the browser closing.
///
/// g3-auth keys a session by the record's key alone (`k3j2h1`, not
/// `app_user:k3j2h1`); `database/presync.surql` rewrites sessions written in
/// the old form, so devices signed in before the change stay signed in.
pub fn sign_in_session(session: &AuthSession, account: &Account) {
    session.login_user(record_key(&account.id).to_string());
    session.remember_user(true);
}

fn record_key(id: &str) -> &str {
    id.strip_prefix("app_user:").unwrap_or(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deletion_covers_every_owner_table_in_the_schema() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("database/schema");
        for entry in std::fs::read_dir(root).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("surql") {
                continue;
            }
            let schema = std::fs::read_to_string(&path).unwrap();
            if schema.contains("DEFINE FIELD IF NOT EXISTS owner ON") {
                let table = path.file_stem().unwrap().to_str().unwrap();
                assert!(
                    DELETE_ACCOUNT_QUERY.contains(&format!("DELETE {table} WHERE owner")),
                    "{table} needs account deletion"
                );
            }
        }
    }

    #[tokio::test]
    async fn account_endpoints_return_429_at_their_separate_limits() {
        use axum::{body::Body, http::Request, routing::post};
        use tower::ServiceExt;
        let app = account_rate_limits().client_ip_header("x-client-ip").apply(
            axum::Router::new()
                .route("/api/v1/auth/guest", post(|| async { "ok" }))
                .route("/api/v1/auth/register", post(|| async { "ok" }))
                .route("/api/v1/auth/sign-in", post(|| async { "ok" })),
        );
        for (path, allowed) in [
            ("/api/v1/auth/guest", 10),
            ("/api/v1/auth/register", 10),
            ("/api/v1/auth/sign-in", 5),
        ] {
            for attempt in 0..=allowed {
                let request = Request::builder()
                    .method("POST")
                    .uri(path)
                    .header("x-client-ip", "198.51.100.1")
                    .header("cookie", format!("session={attempt}"))
                    .body(Body::empty())
                    .unwrap();
                let response = app.clone().oneshot(request).await.unwrap();
                assert_eq!(
                    response.status(),
                    if attempt < allowed {
                        StatusCode::OK
                    } else {
                        StatusCode::TOO_MANY_REQUESTS
                    },
                    "{path}: {attempt}"
                );
                if attempt == allowed {
                    assert!(response.headers().contains_key("retry-after"));
                }
            }
        }
    }

    #[tokio::test]
    async fn deleting_an_account_removes_its_library_and_revokes_other_devices() {
        use axum_session_auth::Authentication;
        let state = crate::server::AppServerState::initialize().await.unwrap();
        let first = state.accounts().create_guest().await.unwrap();
        let second = state.accounts().create_guest().await.unwrap();
        for account in [&first, &second] {
            state.seed_new_account(&account.id).await.unwrap();
            state.set_queue(&account.id, &["one".into()]).await.unwrap();
            state.db.query("CREATE video_progress SET owner = type::record($owner), video_id = 'one'; CREATE user_subscription SET owner = type::record($owner), channel_id = 'UCtest'; CREATE subscription_group SET owner = type::record($owner), group_id = 'group', name = 'Group';")
                .bind(("owner", account.id.clone())).await.unwrap().check().unwrap();
            state.db.query("LET $playlist = array::first(SELECT VALUE id FROM playlist WHERE owner = type::record($owner)); RELATE $playlist->playlist_item->video:example SET position = 0;")
                .bind(("owner", account.id.clone())).await.unwrap().check().unwrap();
        }
        state.db.query("CREATE channel:shared SET channel_id = 'UCtest', name = 'Shared', handle = '@shared', subscribed = true; CREATE channel:exclusive SET channel_id = 'UCexclusive', name = 'Exclusive', handle = '@exclusive', subscribed = true; CREATE user_subscription SET owner = type::record($owner), channel_id = 'UCexclusive';")
            .bind(("owner", first.id.clone())).await.unwrap().check().unwrap();
        state.accounts().delete(&first.id).await.unwrap();
        let mut response = state.db.query("SELECT VALUE subscribed FROM ONLY channel:shared; SELECT VALUE subscribed FROM ONLY channel:exclusive; RETURN count(SELECT id FROM playlist_item);")
            .await.unwrap().check().unwrap();
        assert!(response.take::<Option<bool>>(0).unwrap().unwrap());
        assert!(!response.take::<Option<bool>>(1).unwrap().unwrap());
        assert_eq!(response.take::<Option<i64>>(2).unwrap(), Some(1));
        assert!(state.accounts().find(&first.id).await.is_err());
        assert!(
            <g3_auth::SessionUser<AppUser> as Authentication<
                g3_auth::SessionUser<AppUser>,
                String,
                std::sync::Arc<Surreal<Any>>,
            >>::load_user(record_key(&first.id).into(), Some(&state.db))
            .await
            .is_err()
        );
        assert!(state.accounts().find(&second.id).await.is_ok());
        for table in [
            "playlist",
            "subscription_group",
            "library_state",
            "video_progress",
            "user_subscription",
        ] {
            for (account, expected_empty) in [(&first, true), (&second, false)] {
                let ids: Vec<surrealdb_types::RecordId> = state
                    .db
                    .query(format!(
                        "SELECT VALUE id FROM {table} WHERE owner = type::record($owner)"
                    ))
                    .bind(("owner", account.id.clone()))
                    .await
                    .unwrap()
                    .check()
                    .unwrap()
                    .take(0)
                    .unwrap();
                assert_eq!(ids.is_empty(), expected_empty, "{table}: {}", account.id);
            }
        }
    }

    #[test]
    fn a_password_verifies_only_against_itself() {
        let encoded = hash_password("correct horse battery").expect("hash");
        assert!(password_matches("correct horse battery", &encoded));
        assert!(!password_matches("Correct horse battery", &encoded));
    }

    #[test]
    fn the_same_password_hashes_differently_each_time() {
        assert_ne!(
            hash_password("correct horse battery").expect("hash"),
            hash_password("correct horse battery").expect("hash")
        );
    }

    #[test]
    fn a_corrupt_hash_is_refused_rather_than_panicking() {
        assert!(!password_matches("anything", "not-a-phc-string"));
    }

    #[test]
    fn a_sign_in_failure_does_not_reveal_whether_the_account_exists() {
        let message = AuthError::InvalidLogin.message().to_lowercase();
        assert!(!message.contains("not found"));
    }

    /// Opening an endpoint to signed-out callers is a reviewed change: it
    /// shows up here, not only as an attribute somewhere in `api/`.
    #[test]
    fn only_the_sign_in_endpoints_are_public() {
        assert_eq!(
            g3_auth::public_endpoints(),
            [
                "/api/v1/auth/guest",
                "/api/v1/auth/sign-in",
                "/api/v1/auth/sign-out"
            ]
        );
    }

    #[test]
    fn every_page_is_public_and_no_api_path_is() {
        use g3_auth::PublicRoutes;

        use crate::app::Route;

        for page in [
            "/",
            "/subscriptions",
            "/watch/abc",
            "/playlists/x",
            "/channel/UC1",
        ] {
            assert!(Route::is_public_path(page), "{page} should be public");
        }
        // The catch-all redirect parses every path as `Feed`; the guard must
        // not read that as "public".
        assert!(!Route::is_public_path("/api/v1/library"));
    }

    #[test]
    fn sessions_are_keyed_by_the_bare_record_key() {
        assert_eq!(record_key("app_user:k3j2h1"), "k3j2h1");
        assert_eq!(record_key("k3j2h1"), "k3j2h1");
    }

    /// A device signed in before g3-auth holds a session naming its account
    /// as `app_user:<key>`. The pre-sync rewrite keeps it signed in.
    #[tokio::test]
    async fn an_old_session_is_rewritten_to_the_bare_key() {
        let db = surrealdb::engine::any::connect("mem://").await.unwrap();
        db.use_ns("test").use_db("test").await.unwrap();
        let old = r#"{"id":"s1","data":{"user_auth_session_id":"\"app_user:k3j2h1\""}}"#;
        db.query("CREATE sessions:s1 SET sessionstore = $old, sessionid = 's1'")
            .bind(("old", old))
            .await
            .unwrap()
            .check()
            .unwrap();

        for _ in 0..2 {
            // Twice: it runs on every start.
            db.query(include_str!("../../database/presync.surql"))
                .await
                .unwrap()
                .check()
                .unwrap();
        }

        let stored: Option<String> = db
            .query("SELECT VALUE sessionstore FROM ONLY sessions:s1")
            .await
            .unwrap()
            .take(0)
            .unwrap();
        assert_eq!(
            stored.as_deref(),
            Some(r#"{"id":"s1","data":{"user_auth_session_id":"\"k3j2h1\""}}"#)
        );
    }
}
