//! The server's HTTP surface: the app and its server functions behind the
//! session layers, and the few routes that answer without a session.
//!
//! Shared by the deployed server (`main`) and the backend that runs inside the
//! app on the viewer's own device (`device_backend`), so both answer the same
//! requests the same way.

use std::sync::Arc;

use axum_session::{SessionConfig, SessionLayer, SessionStore};
use axum_session_auth::AuthConfig;
use dioxus::server::axum::middleware::{from_fn, from_fn_with_state};
use dioxus::server::axum::routing::get;
use dioxus::server::axum::{Extension, Router};
use dioxus::server::{DioxusRouterExt, FullstackState};
use g3_auth::{AuthGuard, AuthSessionLayer, require_session};
use surrealdb::engine::any::Any;

use crate::{App, app, auth, health, server, telemetry};

/// How long a session on a device backend lasts without use.
///
/// A device holds one person's library behind a guest session, and a session
/// that lapsed would lock them out of it: the data would still be on the
/// phone, owned by an account nothing can sign in to. A server's guest can
/// register to protect theirs; a device has no one else to sign in as.
const DEVICE_SESSION_DAYS: i64 = 3650;

pub async fn router(state: server::AppServerState) -> anyhow::Result<Router> {
    let db = Arc::clone(&state.db);
    let mut session_config = SessionConfig::default()
        .with_cookie_path("/")
        .with_secure(auth::secure_cookies());
    if server::on_device().is_some() {
        let lifetime = chrono::Duration::days(DEVICE_SESSION_DAYS);
        session_config = session_config
            .with_lifetime(lifetime)
            .with_max_lifetime(lifetime)
            .with_max_age(Some(lifetime));
    }
    let session_store =
        SessionStore::new(Some(auth::session_pool(Arc::clone(&db))), session_config).await?;
    // Every page is public: the client signs a first-time visitor in as a
    // guest from whichever page they opened. Server functions are not,
    // unless marked `#[g3_auth::public]`.
    let guard = AuthGuard::for_routes(app::Route::Feed {});
    let rate_limits = auth::account_rate_limits().shared();

    // Routes that answer without a session, outside the session layers.
    // Probes should not allocate a viewer session; YouTube's hub has no
    // cookie; and a proxy token is itself the capability, which a native
    // media element fetches without the app's cookie.
    let sessionless = Router::new()
        .route("/api/v1/health", get(health::health))
        .route(
            "/api/v1/playback/proxy/{token}",
            get(server::playback_proxy).options(server::playback_proxy_options),
        )
        .route(
            "/api/v1/websub/youtube",
            get(server::youtube_websub_verify).post(server::youtube_websub_notification),
        )
        .layer(Extension(state.clone()));

    let app = if server::on_device().is_some() {
        // The API alone. The app renders its own pages, and there is no
        // `public/` folder beside an app's executable for the asset routes,
        // which panic when it is missing.
        Router::new()
            .register_server_functions()
            .with_state(FullstackState::headless())
    } else {
        dioxus::server::router(App)
    };

    Ok(app
        .layer(from_fn_with_state(
            rate_limits,
            g3_auth::rate_limit::enforce,
        ))
        .layer(Extension(state))
        .layer(Extension(Arc::clone(&db)))
        .layer(from_fn_with_state(
            guard,
            require_session::<auth::AppUser, Any>,
        ))
        .layer(
            AuthSessionLayer::<auth::AppUser, Any>::new(Some(db))
                .with_config(AuthConfig::<String>::default().set_cache(false)),
        )
        .layer(SessionLayer::new(session_store))
        .layer(from_fn(auth::private_account_responses))
        .layer(g3_cache::cdn_cache_guard("/api"))
        .merge(sessionless)
        .layer(from_fn(telemetry::request_complete)))
}
