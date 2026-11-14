mod api;

mod app;

mod auth;

mod cache;

mod components;

mod data_change;

mod downloads;

#[cfg(feature = "server")]
mod db;

#[cfg(feature = "server")]
mod health;

mod library_move;

mod library_sync;
// The relay is Android's; the server build compiles it only to test it.
#[cfg(any(target_os = "android", all(test, feature = "server")))]
mod local_relay;

#[cfg(target_os = "android")]
mod local_streams;

mod models;

mod playback_resolver;

#[cfg(feature = "server")]
mod server;

#[cfg(feature = "server")]
mod server_router;

mod device_backend;

mod server_url;

mod state;

mod subscriptions_io;

#[cfg(feature = "server")]
mod telemetry;

#[cfg(any(feature = "desktop", feature = "mobile"))]
mod updater;

mod youtube_links;

use app::App;
// Both features re-export the same `dioxus-desktop` crate.
#[cfg(all(feature = "desktop", not(feature = "mobile")))]
use dioxus::desktop::Config as NativeConfig;

#[cfg(feature = "mobile")]
use dioxus::mobile::Config as NativeConfig;

#[cfg(feature = "server")]
fn main() {
    let _ = dotenvy::dotenv();
    telemetry::init();
    dioxus::serve(|| async move {
        let state = server::AppServerState::initialize()
            .await
            .unwrap_or_else(|_| panic!("Tawny initialization failed; check configuration and private database migration status."));
        server::spawn_subscription_poller(state.clone());
        server_router::router(state).await
    });
}

#[cfg(not(feature = "server"))]
fn main() {
    g3_auth::init();
    g3_ui::init_auto_mode();
    // Before `launch`: the first render already calls server functions. The
    // cookie runtime is initialized above so the first request can restore its
    // session. See `server_url` for why the URL cannot come from the usual
    // persistence helper.
    server_url::install();

    // Before launch too, so Settings finds the updater ready.
    #[cfg(any(feature = "desktop", feature = "mobile"))]
    updater::configure();

    #[cfg(any(feature = "desktop", feature = "mobile"))]
    let config = NativeConfig::new().with_custom_head(app::native_head());

    // The window needs its own icon, including when run without dx's packager.
    // Embedding the ICO also makes Cargo relink when its Windows resource changes.
    #[cfg(all(feature = "desktop", not(feature = "mobile")))]
    let config = config.with_icon(
        dioxus::desktop::icon_from_memory(include_bytes!("../assets/icon/icon.ico"))
            .expect("the bundled Tawny icon is valid"),
    );

    #[cfg(all(feature = "desktop", not(feature = "mobile"), target_os = "windows"))]
    let config = config.with_menu(None);

    #[cfg(any(feature = "desktop", feature = "mobile"))]
    dioxus::LaunchBuilder::new().with_cfg(config).launch(App);

    #[cfg(not(any(feature = "desktop", feature = "mobile")))]
    dioxus::launch(App);
}
