use crate::{app::Route, state::AppState};
use dioxus::prelude::*;
use dioxus_icons::lucide::{History, House, List as ListIcon, ListVideo, Search, Settings, Users};

#[cfg(any(target_os = "android", target_os = "ios"))]
use g3_native_plugins::NativePlugins;
use g3_route_transitions::{
    ROUTE_TRANSITION_PERSISTENT_REGION_CLASS, RouteTransitionPageRegion, animated_back_or_navigate,
    animated_navigate, use_native_back_navigation_with_layer,
};
use g3_ui::{
    AdaptiveNav, BackButton, Button, ButtonFill, Color, Header, NavItem, Space, Stack, StackAlign,
    TabLayout, Text, TextVariant, use_toast,
};
use std::{cell::RefCell, rc::Rc};

use super::PersistentPlayer;

/// The gold label: the nav destination the current route belongs to.
fn section_label(route: &Route) -> &'static str {
    match route {
        Route::Subscriptions {} => "Subscriptions",
        Route::Playlists {} | Route::PlaylistDetail { .. } | Route::Downloads {} => "Playlists",
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
/// a page opened afresh. A page is rebuilt on every visit, so without this
/// every return to the feed was a return to its first screen. One kept beneath
/// a sheet is not rebuilt, but `display: none` still drops its offset.
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
const SCROLL_MEMORY_JS: &str = include_str!("../../assets/tawny_scroll_memory.js");

/// The app icon's black owl, white eyes and all, on either theme. Drawn
/// from a file because its colours are fixed rather than the theme's.
const BRAND_OWL: Asset = asset!("/assets/brand-owl.svg");

#[component]
fn BrandOwl() -> Element {
    rsx! {
        img { src: BRAND_OWL, width: "20", height: "20", alt: "" }
    }
}

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
    let router_route: Route = use_route();
    // A page kept beneath a sheet describes itself, not the sheet.
    let route = try_use_context::<BaseRoute>()
        .map(|base| base.0.borrow().clone())
        .unwrap_or(router_route);
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
                    BrandOwl {}
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

    rsx! {
        if let Some(removed) = app_state.undo_removal.read().as_ref() {
            UndoRemovalToast { key: "{removed.playlist_id}:{removed.video.id}" }
        }
    }
}

/// ToastOptions in g3-ui 0.6 cannot carry an action. Keep this action-bearing
/// toast in the app's bridge so it survives the card and page that opened it.
#[component]
fn UndoRemovalToast() -> Element {
    let mut app_state = use_context::<AppState>();
    let open = use_signal(|| true);

    rsx! {
        g3_ui::Toast {
            open,
            message: "Removed from playlist",
            duration: g3_ui::ToastDuration::Long,
            on_dismiss: move |_| app_state.undo_removal.set(None),
            action: rsx! {
                Button {
                    fill: ButtonFill::Clear,
                    size: g3_ui::ButtonSize::Sm,
                    onclick: move |_| app_state.undo_playlist_removal(),
                    "Undo"
                }
            },
        }
    }
}

#[component]
pub fn AppShell() -> Element {
    rsx! {
        super::AccountGate {
            ShellContent {}
        }
    }
}

#[component]
fn ShellContent() -> Element {
    super::mouse_navigation::use_mouse_navigation::<Route>();
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
    let is_sheet = is_sheet_route(&route);
    // The page a sheet was opened over stays mounted beneath it, hidden, so
    // dismissing the sheet reveals it instead of rebuilding it: on a phone
    // that rebuild was a ~30 KB edit batch and ~100 ms before the sheet could
    // start to fall. A plain cell, not a signal: the shell re-renders on every
    // route change anyway, and this only carries the answer to the next one.
    let remembered = use_hook(|| Rc::new(RefCell::new(None::<Route>)));
    let base_route = page_beneath(&route, remembered.borrow().clone());
    remembered.borrow_mut().clone_from(&base_route);

    rsx! {
        NativeMediaCoordinator {}
        NativeBackCoordinator {}
        crate::youtube_links::YoutubeLinkReceiver {}
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
            // Always in this slot and always rendered the same way, so moving
            // between the page and a sheet over it never remounts the page.
            if let Some(base) = base_route {
                BasePage { route: base, covered: is_sheet }
            }
            if is_sheet {
                Outlet::<Route> {}
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
                    selected: matches!(route, Route::Playlists {} | Route::PlaylistDetail { .. } | Route::Downloads {}),
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

/// The route a page in the base slot belongs to. Beneath a sheet the router's
/// current route is the sheet, so the page's own header reads this instead.
/// Not reactive: the header follows the router through `use_route`, so it
/// re-renders on every route change, after `BasePage` has updated this.
#[derive(Clone)]
struct BaseRoute(Rc<RefCell<Route>>);

/// The last non-sheet route, rendered in place of the router's outlet.
///
/// Beneath a sheet it is `display: none`: the browser drops its boxes, so it
/// is never captured as a second `page` snapshot and costs no layout or paint,
/// but its DOM and every component's state stay. Dismissing the sheet only
/// removes the sheet and the class. Scroll offsets do not survive
/// `display: none`; `ScrollMemory` puts them back as it does after any return.
#[component]
fn BasePage(route: Route, covered: bool) -> Element {
    let current = use_context_provider(|| BaseRoute(Rc::new(RefCell::new(route.clone()))));
    *current.0.borrow_mut() = route.clone();
    let class = covered.then(|| "tawny-covered-page".to_string());

    rsx! {
        // The page's own header and body, captured as one image, so the bar
        // slides with the page it belongs to. Level 1: the leaf beneath the
        // `AppShell` layout, which is what the outlet here used to render.
        RouteTransitionPageRegion { class, {route.render(1)} }
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

/// The routes declared `layer = sheet`: they cover the page instead of
/// replacing it.
fn is_sheet_route(route: &Route) -> bool {
    matches!(route, Route::VideoDetail { .. }) || is_auxiliary_route(route)
}

/// The page shown in the base slot: the current route when it is a page,
/// and under a sheet the page it was opened over. Empty when a sheet was the
/// first thing opened; Back then builds whatever it lands on, as before.
fn page_beneath(route: &Route, previous: Option<Route>) -> Option<Route> {
    if is_sheet_route(route) {
        previous
    } else {
        Some(route.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn walk(routes: &[Route]) -> Option<Route> {
        routes
            .iter()
            .fold(None, |previous, route| page_beneath(route, previous))
    }

    #[test]
    fn a_sheet_keeps_the_page_it_was_opened_over() {
        let watch = Route::VideoDetail { id: "video".into() };
        assert_eq!(walk(&[Route::Explore {}, watch]), Some(Route::Explore {}));
    }

    /// Moving between sheets, or from Queue into the player, never changes
    /// what is underneath: minimizing still lands on the page that opened them.
    #[test]
    fn sheets_over_sheets_keep_the_first_page() {
        let watch = Route::VideoDetail { id: "video".into() };
        let next = Route::VideoDetail { id: "next".into() };
        assert_eq!(
            walk(&[
                Route::Feed {},
                Route::QueuePage {},
                Route::SettingsPage {},
                watch,
                next,
            ]),
            Some(Route::Feed {})
        );
    }

    #[test]
    fn a_page_replaces_the_one_kept() {
        let watch = Route::VideoDetail { id: "video".into() };
        let channel = Route::ChannelDetail {
            id: "channel".into(),
        };
        assert_eq!(
            walk(&[Route::Feed {}, watch, channel.clone()]),
            Some(channel)
        );
    }

    #[test]
    fn a_sheet_opened_first_has_nothing_beneath_it() {
        assert_eq!(walk(&[Route::VideoDetail { id: "video".into() }]), None);
        assert_eq!(walk(&[Route::SettingsPage {}]), None);
    }
}
