mod api;
mod app;
mod auth;
mod cache;
mod components;
mod data_change;
#[cfg(feature = "server")]
mod db;
#[cfg(feature = "server")]
mod health;
mod models;
#[cfg(feature = "server")]
mod server;
mod server_url;
mod state;
mod subscriptions_io;

use app::App;
// Both features re-export the same `dioxus-desktop` crate.
#[cfg(all(feature = "desktop", not(feature = "mobile")))]
use dioxus::desktop::Config as NativeConfig;
#[cfg(feature = "mobile")]
use dioxus::mobile::Config as NativeConfig;

#[cfg(feature = "server")]
fn main() {
    let _ = dotenvy::dotenv();
    dioxus::serve(|| async move {
        use std::sync::Arc;

        use axum_session::{SessionConfig, SessionLayer, SessionStore};
        use axum_session_auth::AuthConfig;
        use dioxus::server::axum::middleware::from_fn_with_state;
        use dioxus::server::axum::routing::get;
        use dioxus::server::axum::{Extension, Router};
        use g3_auth::{AuthGuard, AuthSessionLayer, SurrealSessionPool, require_session};
        use surrealdb::engine::any::Any;

        let state = server::AppServerState::initialize()
            .await
            .expect("initialize Tawny server");
        server::spawn_subscription_poller(state.clone());

        let db = Arc::clone(&state.db);
        let session_config = SessionConfig::default()
            .with_cookie_path("/")
            .with_secure(!cfg!(debug_assertions));
        let session_store = SessionStore::new(
            Some(SurrealSessionPool::new(Arc::clone(&db))),
            session_config,
        )
        .await
        .expect("create Tawny session store");
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

        Ok(dioxus::server::router(App)
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
            .layer(g3_cache::cdn_cache_guard("/api"))
            .merge(sessionless))
    });
}

#[cfg(not(feature = "server"))]
fn main() {
    g3_auth::init();
    g3_ui::init_auto_mode();
    // Before `launch`, not after: `set_server_url` keeps only its first value.
    // The cookie runtime is initialized above so the first request can restore
    // its session. See `config` for why the URL cannot come from the usual
    // persistence helper.
    server_url::install();
    #[cfg(any(feature = "desktop", feature = "mobile"))]
    dioxus::LaunchBuilder::new()
        .with_cfg(NativeConfig::new().with_custom_head(app::native_head()))
        .launch(App);
    #[cfg(not(any(feature = "desktop", feature = "mobile")))]
    dioxus::launch(App);
}
