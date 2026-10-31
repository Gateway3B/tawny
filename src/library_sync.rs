//! Bringing this device up to date with changes made on another.
//!
//! Screens show cached answers, refetched when they open and when `g3-cache`
//! sees the app regain focus. That missed the way Tawny is used: a phone left
//! open on a playlist while a video is removed from it on the computer is
//! never refocused, and on Android a loaded video keeps the WebView reporting
//! itself visible in the background, so coming back to the app is not a focus
//! event at all. The playlist stayed stale until it was left and reopened.
//!
//! So the device asks the server what this account has changed
//! ([`get_library_changes`], a few counters held in memory) whenever it comes
//! back into view, and every few seconds while it is in view, and refetches
//! only the kinds of data that moved. A position saved elsewhere also brings a
//! paused player up to date (see [`catch_up_playback`]).

use crate::{
    api::get_library_changes,
    components::catch_up_playback,
    data_change::{DataChange, invalidate},
    models::{LibraryChange, LibraryChanges},
    state::AppState,
};
use dioxus::prelude::*;

/// Sends whenever the device should check, coalescing bursts: coming back to
/// the app fires visibility, focus and the native resume event together.
const CHECK_WHEN_JS: &str = r#"
let waiting = null;
const check = () => {
    if (waiting) return;
    waiting = setTimeout(() => { waiting = null; dioxus.send(true); }, 300);
};
document.addEventListener('visibilitychange', () => {
    if (document.visibilityState === 'visible') check();
});
window.addEventListener('focus', check);
window.addEventListener('online', check);
window.addEventListener('pageshow', check);
// Android, coming back with a video loaded: see the module docs.
window.addEventListener('tawnynativeplaybackresume', check);
// The phone that never left the screen. Timers stop in a hidden tab anyway;
// this keeps a visible one from asking while nobody is looking at it.
setInterval(() => { if (document.visibilityState === 'visible') check(); }, __EVERY_MS__);
check();
await new Promise(() => {});
"#;

/// How often a device in view asks. The answer is a few numbers from memory.
const CHECK_EVERY_MS: u32 = 15_000;

/// Mounted once, beneath [`AppState`]'s provider.
#[component]
pub fn LibrarySync() -> Element {
    let app_state = use_context::<AppState>();
    // An effect, so only the client runs it, and one that reads no signal,
    // so it runs once.
    use_effect(move || {
        spawn(watch(app_state));
    });

    rsx! {}
}

async fn watch(app_state: AppState) {
    let mut checks =
        document::eval(&CHECK_WHEN_JS.replace("__EVERY_MS__", &CHECK_EVERY_MS.to_string()));
    let mut last: Option<LibraryChanges> = None;
    while checks.recv::<bool>().await.is_ok() {
        // Signed out, offline or failing: try again at the next check.
        let Ok(now) = get_library_changes().await else {
            continue;
        };
        // The first answer is only a baseline: the screens fetched just now.
        let Some(before) = last.replace(now) else {
            continue;
        };
        for change in now.changed_since(before) {
            apply(app_state, change).await;
        }
    }
}

/// Changes this device made itself come back here too, a few seconds later.
/// Refetching for them costs a request and no redraw: a cached read that comes
/// back the same does not re-render.
async fn apply(app_state: AppState, change: LibraryChange) {
    match change {
        LibraryChange::Subscriptions => invalidate(DataChange::Subscriptions),
        LibraryChange::Groups => invalidate(DataChange::Groups),
        LibraryChange::Playlists => invalidate(DataChange::Playlists),
        LibraryChange::Queue => invalidate(DataChange::Queue),
        LibraryChange::History => invalidate(DataChange::History),
        LibraryChange::Watched => invalidate(DataChange::Watched),
        LibraryChange::Progress => catch_up_playback(app_state).await,
    }
}
