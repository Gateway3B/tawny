use crate::{app::Route, state::AppState};
use dioxus::prelude::*;
use dioxus_icons::lucide::{
    History, House, List as ListIcon, ListVideo, Play, Search, Settings, Users,
};
#[cfg(target_os = "android")]
use g3_native_plugins::NativePlugins;
use g3_route_transitions::{
    ROUTE_TRANSITION_PERSISTENT_REGION_CLASS, RouteTransitionPageRegion, animated_back_or_navigate,
    animated_navigate, use_native_back_navigation_with_layer,
};
use g3_ui::{
    AdaptiveNav, BackButton, Button, ButtonFill, Color, Header, NavItem, Space, Stack, StackAlign,
    TabLayout, Text, TextVariant, use_toast,
};

use super::PersistentPlayer;

/// The gold label: the nav destination the current route belongs to.
fn section_label(route: &Route) -> &'static str {
    match route {
        Route::Subscriptions {} => "Subscriptions",
        Route::Playlists {} | Route::PlaylistDetail { .. } => "Playlists",
        Route::Explore {} => "Search",
        _ => "Feed",
    }
}

/// Installs the shared native-Back/router integration. Tawny keeps only its
/// app-specific overlay priority on the standard DOM events; the library owns
/// the actual Dioxus pop and transition handshake.
#[component]
fn NativeBackCoordinator() -> Element {
    let app_state = use_context::<AppState>();
    // Every open g3 sheet counts, including the ones a page keeps in a local
    // signal - comments, captions, audio, a channel's description. Listing app
    // state by hand missed those, so Back at a history-less root fell through
    // to Android and closed the app instead of the sheet. Share is a modal.
    let overlay_open = use_memo(move || g3_ui::open_sheet_count() > 0 || (app_state.share_open)());
    use_native_back_navigation_with_layer::<Route>(overlay_open);

    #[cfg(target_os = "android")]
    let _native_back_behavior = use_hook(|| {
        document::eval(
            r#"
            const stateKey = Symbol.for('tawny.native-back-ui');
            window[stateKey]?.dispose?.();

            const onNativeBack = async (event) => {
                if (document.fullscreenElement || document.webkitFullscreenElement) {
                    event.preventDefault();
                    if (window.__tawnyFullscreenBackPending) return;
                    window.__tawnyFullscreenBackPending = true;
                    document.querySelector('[data-player-native-orientation-unlock]')?.click();
                    const exit = document.exitFullscreen || document.webkitExitFullscreen;
                    if (exit) {
                        try { await exit.call(document); } catch (_) {}
                    }
                    await new Promise((resolve) => requestAnimationFrame(resolve));
                    document.querySelector('[data-player-minimize]')?.click();
                    setTimeout(() => { window.__tawnyFullscreenBackPending = false; }, 350);
                    return;
                }

                const options = document.querySelector('[data-player-options-menu]:not([hidden])');
                if (options) {
                    event.preventDefault();
                    document.querySelector('[data-player-action="settings"]')?.click();
                    return;
                }

                // The topmost open sheet, whatever its backdrop. The comments
                // sheet has no scrim to click, so every g3 sheet carries a
                // hidden dismiss control and Back uses that instead.
                const dismiss = [
                    ...document.querySelectorAll('[data-g3-sheet-dismiss]'),
                ].pop();
                if (dismiss) {
                    event.preventDefault();
                    dismiss.click();
                    return;
                }

                const modal = document.querySelector('.g3-modal-overlay[data-state="open"]');
                if (modal) {
                    event.preventDefault();
                    modal.click();
                    return;
                }

            };

            window.addEventListener('g3nativeback', onNativeBack);
            window[stateKey] = {
                dispose() {
                    window.removeEventListener('g3nativeback', onNativeBack);
                },
            };
            "#,
        )
    });

    rsx! {}
}

#[cfg(any(target_os = "android", target_os = "ios"))]
#[component]
fn NativeMediaCoordinator() -> Element {
    let mut plugins = use_context::<NativePlugins>();
    let app_state = use_context::<AppState>();
    use_hook(move || {
        let _ = plugins.media.write().prepare();
    });
    // Pausing keeps the system media controls so they can resume playback;
    // closing the player is what takes them away.
    //
    // Opening a video also claims playback at once, before there is a media
    // element to play. The host pauses the WebView when the screen goes off,
    // and it only keeps it running for playback it knows about: a video still
    // resolving when the screen turned off never rendered, so it never began,
    // and waited for the next unlock. Keyed on the id so the metadata filling
    // in does not re-claim playback that was paused since.
    let active_id = use_memo(move || app_state.active_video().map(|video| video.id));
    use_effect(move || match active_id() {
        None => {
            let _ = plugins.media.write().clear_playback();
        }
        Some(_) => {
            let title = app_state
                .active_video
                .peek()
                .as_ref()
                .map(|video| video.title.clone())
                .unwrap_or_default();
            let _ = plugins.media.write().set_playback_active(true, title);
        }
    });
    rsx! {}
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
#[component]
fn NativeMediaCoordinator() -> Element {
    rsx! {}
}
/// Remembers how far each page was scrolled, and puts it back when the page
/// returns: by Back, by closing the player over it, or by tapping its tab.
///
/// A page that is pushed from another starts at the top, as it should; so does
/// a page opened afresh. The pages are rebuilt on every visit, so without this
/// every return to the feed was a return to its first screen.
///
/// Mounted once per route, keyed by it, so arriving is what runs it. The work
/// is JS because the scroll area is a DOM element the framework does not own,
/// and the offset has to be set during the transition, before the new page is
/// captured, or the slide would end on the top and then jump.
#[component]
fn ScrollMemory(path: String) -> Element {
    // An effect, not a hook: the server renders this too, and has no page to
    // scroll. It reads no signal, so it runs once, which is the arrival.
    use_effect(move || {
        let path = serde_json::to_string(&path).unwrap_or_default();
        document::eval(&SCROLL_MEMORY_JS.replace("__PATH__", &path));
    });
    rsx! {}
}

/// Tab roots keep their place when switched to; everything else keeps it only
/// when it is returned to. The watch page is never restored: opening a video
/// starts it at the top, and the details reset it there on purpose.
const SCROLL_MEMORY_JS: &str = r#"
const path = __PATH__;
const key = Symbol.for('tawny.scroll-memory');
if (!window[key]) {
    const saved = new Map();
    let current = null;
    let token = 0;
    let restoring = false;
    const scroller = () => document.querySelector('.g3-content-scroll');
    document.addEventListener('scroll', (event) => {
        const element = event.target;
        if (restoring || !current || !(element instanceof Element)) return;
        if (!element.matches('.g3-content-scroll')) return;
        if (document.documentElement.dataset.routeTransition) return;
        saved.set(current, element.scrollTop);
    }, true);
    // Whoever scrolls for themselves while a position is being put back wins.
    const yield_to_user = () => { token += 1; restoring = false; };
    for (const name of ['wheel', 'touchstart', 'pointerdown', 'keydown']) {
        document.addEventListener(name, yield_to_user, { capture: true, passive: true });
    }
    window[key] = {
        arrive(next, restore) {
            current = next;
            const mine = ++token;
            restoring = false;
            const target = restore ? (saved.get(next) || 0) : 0;
            if (!restore) saved.delete(next);
            if (!target) return;
            restoring = true;
            const started = Date.now();
            const step = () => {
                if (mine !== token) return;
                const element = scroller();
                const elapsed = Date.now() - started;
                if (element) {
                    if (Math.abs(element.scrollTop - target) > 1) element.scrollTop = target;
                    // Lists arrive in stages, so a page too short to hold the
                    // offset yet is given a moment to grow.
                    if (Math.abs(element.scrollTop - target) <= 1 && elapsed > 250) {
                        restoring = false;
                        return;
                    }
                }
                if (elapsed > 2500) {
                    if (element) saved.set(next, element.scrollTop);
                    restoring = false;
                    return;
                }
                setTimeout(step, 50);
            };
            step();
        },
    };
}
const roots = ['/', '/subscriptions', '/playlists', '/explore'];
const transition = document.documentElement.dataset.routeTransition;
const returning = transition === 'backward' || transition === 'dismiss-sheet';
const restore = !path.startsWith('/watch/') && (returning || roots.includes(path));
window[key].arrive(path, restore);
"#;

/// The bar every page renders for itself, so it travels with the page it
/// describes rather than staying put while the page slides out from under it.
#[component]
pub fn PageHeader(
    /// Blank for pages that lead with their own title block, such as a channel.
    title: Option<String>,
    /// Supplied by pages that were navigated into: renders Back in place of the
    /// brand lockup, and is the destination used when the page was deep-linked
    /// and has no history to pop.
    back_to: Option<Route>,
    /// Optional segmented control rendered under the bar.
    toolbar: Option<Element>,
    /// Page-specific content beside the header's trailing controls.
    end_slot: Option<Element>,
) -> Element {
    let route: Route = use_route();
    let app_state = use_context::<AppState>();
    // All three are one group of peers presented over the page that launched
    // them. Once any of them is up, offering the group again from inside
    // itself is just chrome the sheet has to carry.
    let show_global_actions = !is_auxiliary_route(&route);

    let start = match back_to {
        Some(home) => rsx! {
            // Popping keeps the two back affordances agreeing; the route is
            // only a fallback for a deep link with nothing to pop.
            BackButton {
                onclick: move |_| {
                    spawn(animated_back_or_navigate(home.clone()));
                },
            }
        },
        None => rsx! {
            Stack { horizontal: true, gap: Space::Sm, align: StackAlign::Center, class: "px-2",
                span {
                    class: "grid size-7 place-items-center rounded-lg",
                    style: "background: var(--g3-color-accent); color: var(--g3-color-on-accent);",
                    Play { size: 15, fill: "currentColor" }
                }
                // A tab's root page has no title in the bar, so the section
                // name beside the brand mark is the page's heading.
                h1 { class: "m-0 flex",
                    Text { variant: TextVariant::Overline, color: Color::Accent, "{section_label(&route)}" }
                }
            }
        },
    };

    rsx! {
        Header {
            title: title.unwrap_or_default(),
            start,
            end: rsx! {
                if let Some(end_slot) = end_slot {
                    {end_slot}
                }
                if show_global_actions {
                    Button {
                        fill: ButtonFill::Clear,
                        aria_label: format!("Queue, {} videos", app_state.with_viewer(|viewer| viewer.queue.len())),
                        onclick: move |_| { spawn(animated_navigate(Route::QueuePage {})); },
                        ListVideo { size: 20 }
                    }
                    Button {
                        fill: ButtonFill::Clear,
                        aria_label: "History",
                        onclick: move |_| { spawn(animated_navigate(Route::HistoryPage {})); },
                        History { size: 20 }
                    }
                    Button {
                        fill: ButtonFill::Clear,
                        aria_label: "Settings",
                        onclick: move |_| { spawn(animated_navigate(Route::SettingsPage {})); },
                        Settings { size: 20 }
                    }
                }
            },
            toolbar,
        }
    }
}

/// Hands g3-ui's toast queue to the app state, which is provided above the
/// `AppWrapper` that owns the queue.
#[component]
fn ToasterBridge() -> Element {
    let mut app_state = use_context::<AppState>();
    let toaster = use_toast();
    use_hook(move || app_state.toaster.set(Some(toaster)));
    rsx! {}
}

#[component]
pub fn AppShell() -> Element {
    let route: Route = use_route();
    let mut app_state = use_context::<AppState>();

    let route_path = route.to_string();
    let player_expanded = matches!(route, Route::VideoDetail { .. });
    let on_feed = matches!(route, Route::Feed {});
    let is_auxiliary = is_auxiliary_route(&route);
    // Queue, History and Settings are sheets presented over whatever launched
    // them, and the mini bar belongs to the page underneath. Leaving it on top
    // covers the bottom of a sheet that is already scrolling under it.
    let has_mini_player = app_state.active_video().is_some() && !player_expanded && !is_auxiliary;
    let mut shell_class = "tawny-shell".to_string();
    if player_expanded {
        shell_class.push_str(" tawny-shell-cover");
    }
    if has_mini_player {
        shell_class.push_str(" has-mini-player");
    }
    if is_auxiliary {
        shell_class.push_str(" tawny-shell-auxiliary");
    }
    // A sheet route carries the overlay region itself and therefore does not
    // render a page region as well.
    let is_sheet = player_expanded || is_auxiliary;

    rsx! {
        NativeMediaCoordinator {}
        NativeBackCoordinator {}
        ToasterBridge {}
        // Keyed by the route so arriving at a page is a mount.
        {rsx! { ScrollMemory { key: "{route_path}", path: route_path.clone() } }}
        // The tab layout is the base region a sheet covers. Nothing may wrap
        // it in another snapshot region: a named descendant is lifted out of
        // its ancestor, so the outer region would contain only the lifted
        // base and paint as a bare background behind the sheet.
        TabLayout { class: shell_class,
            // Mounted here rather than inside a page so playback survives
            // navigation. Minimized it is fixed and out of flow; expanded it is
            // in flow above the watch page's own body.
            PersistentPlayer { expanded: player_expanded }
            if is_sheet {
                Outlet::<Route> {}
            } else {
                // The page's own header and body, captured as one image, so
                // the bar slides with the page it belongs to.
                RouteTransitionPageRegion { Outlet::<Route> {} }
            }
            // Always rendered: the wide-layout rail is permanent chrome, and
            // only the compact bottom bar gets out of a sheet's way. Hiding it
            // in CSS keeps that a layout decision rather than a routing one.
            AdaptiveNav {
                class: ROUTE_TRANSITION_PERSISTENT_REGION_CLASS,
                aria_label: "Primary navigation",
                NavItem {
                    label: "Feed",
                    selected: on_feed,
                    icon: rsx! { House { size: 22 } },
                    onclick: move |_| {
                        if on_feed {
                            app_state.feed_reselected += 1;
                        } else {
                            spawn(animated_navigate(Route::Feed {}));
                        }
                    },
                }
                NavItem {
                    label: "Playlists",
                    selected: matches!(route, Route::Playlists {} | Route::PlaylistDetail { .. }),
                    icon: rsx! { ListIcon { size: 22 } },
                    onclick: move |_| { spawn(animated_navigate(Route::Playlists {})); },
                }
                NavItem {
                    label: "Search",
                    selected: matches!(route, Route::Explore {}),
                    icon: rsx! { Search { size: 22 } },
                    onclick: move |_| { spawn(animated_navigate(Route::Explore {})); },
                }
                NavItem {
                    label: "Subscriptions",
                    selected: matches!(route, Route::Subscriptions {} | Route::ChannelDetail { .. }),
                    icon: rsx! { Users { size: 22 } },
                    onclick: move |_| { spawn(animated_navigate(Route::Subscriptions {})); },
                }
            }
        }
    }
}

/// Queue, History, and Settings: reached from the header rather than a tab, and
/// presented as a sheet over whatever launched them.
fn is_auxiliary_route(route: &Route) -> bool {
    matches!(
        route,
        Route::SettingsPage {} | Route::QueuePage {} | Route::HistoryPage {}
    )
}
