use crate::{
    components::{
        AccountGate, AppOverlays, AppShell, ChannelDetail, Explore, Feed, HistoryPage,
        PlaylistDetail, Playlists, QueuePage, SettingsPage, Subscriptions, VideoDetail,
    },
    models::{Appearance, PlatformStyle},
    state::{AppState, AppStateProvider},
};
use dioxus::prelude::*;
use g3_native_plugins::NativePluginsProvider;
use g3_route_transitions::{
    Platform as TransitionPlatform, RouteTransitions, detect_platform, set_platform,
    use_browser_history_transitions,
};
use g3_ui::{AppWrapper, ComponentMode, Theme};

// Statically headed so web builds block first paint on it, the same as
// g3-ui.css - linking it only at runtime left the app rendering unstyled while
// the WASM booted. Desktop and mobile bundles do not collect statically-headed
// assets, so the runtime link below is what reaches those builds.
const TAILWIND_CSS: Asset = asset!(
    "/assets/tailwind.css",
    AssetOptions::css().with_static_head(true)
);
const SHAKA_PLAYER_JS: Asset = asset!("/node_modules/shaka-player/dist/shaka-player.compiled.js");
const TAWNY_TRANSPORT_JS: Asset = asset!("/assets/tawny_transport.js");
const TAWNY_PLAYER_CONTROLS_JS: Asset = asset!("/assets/tawny_player_controls.js");
const FAVICON_SVG: Asset = asset!("/assets/favicon.svg");

/// Stylesheet links for the native WebView's initial `index.html`.
///
/// Statically-headed assets only reach the web build. On desktop and mobile
/// every stylesheet was linked at runtime, after the first render had already
/// reached the WebView, so the app painted unstyled until they loaded. Links in
/// the served head block first paint the way the web build's do. The runtime
/// links stay: the native bundles collect assets from them.
#[cfg(any(feature = "desktop", feature = "mobile"))]
pub fn native_head() -> String {
    [
        g3_ui::UI_CSS,
        g3_route_transitions::ROUTE_TRANSITIONS_CSS,
        TAILWIND_CSS,
    ]
    .iter()
    .map(|href| format!(r#"<link rel="stylesheet" href="{href}">"#))
    .collect()
}

/// Primary destinations are stable roots. Pages reached from a card push above
/// them; the watch page and the three header destinations present as sheets.
///
/// Every page is `#[public]`: a first-time visitor may open any of them, and
/// the client signs them in as a guest from there. The data each page shows
/// comes from server functions, which the guard does protect.

#[derive(Debug, Clone, Routable, PartialEq, RouteTransitions, g3_auth::PublicRoutes)]
#[rustfmt::skip]
pub enum Route {
    #[layout(AppShell)]
        #[redirect("/:..segments", |segments: Vec<String>| { let _ = segments; Route::Feed {} })]
        #[transition(layer = stack_root)]
        #[public]
        #[route("/")]
        Feed {},
        #[transition(layer = stack_root)]
        #[public]
        #[route("/subscriptions")]
        Subscriptions {},
        #[transition(layer = stack_root)]
        #[public]
        #[route("/playlists")]
        Playlists {},
        #[transition(layer = stack_root)]
        #[public]
        #[route("/explore")]
        Explore {},
        // Queue, History, and Settings are peers: each one's header offers the
        // other two, so a lateral move between them slides in the order the header
        // lists them instead of taking the unrelated-peer fade.
        #[transition(layer = sheet, forward_to = (HistoryPage, SettingsPage))]
        #[public]
        #[route("/queue")]
        QueuePage {},
        #[transition(layer = sheet, forward_to = SettingsPage)]
        #[public]
        #[route("/history")]
        HistoryPage {},
        // A new item in an active run replaces the current watch entry.  Back
        // then dismisses the player to the page beneath it instead of walking
        // back through every video autoplay (or the queue controls) visited.
        // Opening a video from Queue or History hands that sheet off to this
        // one: it rises like any video and takes the header sheet's place in
        // history, so minimizing lands on the page beneath both instead of
        // reopening the list the video was picked from.
        #[transition(layer = sheet, history = replace, handoff_from = (QueuePage, HistoryPage))]
        #[public]
        #[route("/watch/:id")]
        VideoDetail { id: String },
        #[transition(layer = stack_page)]
        #[public]
        #[route("/playlists/:id")]
        PlaylistDetail { id: String },
        #[transition(layer = sheet)]
        #[public]
        #[route("/settings")]
        SettingsPage {},
        #[transition(layer = stack_page)]
        #[public]
        #[route("/channel/:id")]
        ChannelDetail { id: String },
}

fn tawny_theme(appearance: Appearance) -> Theme {
    match appearance {
        // Each token becomes a CSS `light-dark()` pair, so the browser follows
        // the system setting by itself, with no script and no re-render.
        Appearance::Auto => Theme::adaptive(
            tawny_theme(Appearance::Light),
            tawny_theme(Appearance::Dark),
        ),
        Appearance::Dark => Theme {
            accent: "#f5a524".into(),
            accent_secondary: "#f5a524".into(),
            // Amber is light, so what sits on it is dark.
            on_accent: "#1a1206".into(),
            text: "#f8fafc".into(),
            text_secondary: "#a8b3bf".into(),
            text_tertiary: "#9aa6b2".into(),
            bg: "#0b0f14".into(),
            bg_secondary: "#10161e".into(),
            card: "#141b24".into(),
            surface: "#18212c".into(),
            control: "rgba(255, 255, 255, 0.09)".into(),
            border: "rgba(255, 255, 255, 0.09)".into(),
            shadow: "rgba(0, 0, 0, 0.42)".into(),
            success: "#5bc987".into(),
            warning: "#f5a524".into(),
            on_warning: "#1a1206".into(),
            danger: "#ff6b6b".into(),
            color_scheme: "dark".into(),
            ..Theme::default()
        },
        Appearance::Light => Theme {
            // Tawny in daylight: warm parchment and weathered-sage layers,
            // not a white system theme with an orange accent. Keep every
            // elevation visibly distinct, as the g3 playground themes do.
            accent: "#a9530b".into(),
            accent_secondary: "#a9530b".into(),
            on_accent: "#ffffff".into(),
            text: "#342a21".into(),
            text_secondary: "#725f4d".into(),
            text_tertiary: "#725f4d".into(),
            bg: "#d8c5a8".into(),
            bg_secondary: "#cbb595".into(),
            card: "#eadcc5".into(),
            surface: "#e3d2b8".into(),
            control: "#d4bea0".into(),
            border: "#bea27f".into(),
            shadow: "rgba(70, 45, 25, 0.24)".into(),
            success: "#178451".into(),
            warning: "#a9530b".into(),
            on_warning: "#ffffff".into(),
            danger: "#c93b45".into(),
            color_scheme: "light".into(),
            ..Theme::default()
        },
    }
}

#[component]
pub fn App() -> Element {
    rsx! {
        // Declared rather than left to the browser, which otherwise probes
        // /favicon.ico on every load and takes a 404 for it.
        document::Link { rel: "icon", r#type: "image/svg+xml", href: FAVICON_SVG }
        // Tawny has its own dark theme, and Dark Reader leaks on the player:
        // each write to the seek bar's inline custom properties makes it wrap
        // its own previous rewrite in another `--darkreader-bgimg--` layer, and
        // a long-lived tab grew past 2 GB of those strings.
        document::Meta { name: "darkreader-lock" }
        document::Link { rel: "stylesheet", href: TAILWIND_CSS }
        document::Script { src: SHAKA_PLAYER_JS }
        document::Script { src: TAWNY_TRANSPORT_JS }
        document::Script { src: TAWNY_PLAYER_CONTROLS_JS }
        NativePluginsProvider {
            AppStateProvider {
                ThemedApp {}
            }
        }
    }
}

/// Whether the system is in dark mode, where the page cannot tell by itself.
///
/// Android's WebView reports the app's own theme to `prefers-color-scheme`,
/// and the one `dx` generates is light, so Auto stayed light on a phone set to
/// dark. The system is asked directly there. Everywhere else the page's media
/// query is right, and Auto's `light-dark()` pairs follow it.
fn system_dark() -> Option<bool> {
    #[cfg(target_os = "android")]
    {
        let mut plugins = try_consume_context::<g3_native_plugins::NativePlugins>()?;
        plugins.appearance.with_mut(|appearance| appearance.dark())
    }
    #[cfg(not(target_os = "android"))]
    None
}

/// Auto, settled to the system's answer where the page could not read it.
fn resolved_appearance(appearance: Appearance, system_dark: Option<bool>) -> Appearance {
    match (appearance, system_dark) {
        (Appearance::Auto, Some(true)) => Appearance::Dark,
        (Appearance::Auto, Some(false)) => Appearance::Light,
        (chosen, _) => chosen,
    }
}

#[component]
fn ThemedApp() -> Element {
    let app_state = use_context::<AppState>();
    let settings = app_state.settings();
    // Once: changing the system theme recreates the Android Activity, which
    // starts the app over.
    let system_dark = use_hook(system_dark);
    let appearance = resolved_appearance(settings.appearance, system_dark);
    use_browser_history_transitions::<Route>();

    // g3-ui and the transition library each keep their own notion of platform,
    // and both have to agree or the components render one language while the
    // navigation animates in the other. `Auto` leaves each library on its own
    // detection, which is the right answer for a packaged build.
    let component_mode = match settings.platform_style {
        PlatformStyle::Auto => None,
        PlatformStyle::Ios => Some(ComponentMode::Ios),
        PlatformStyle::Material => Some(ComponentMode::Md),
    };
    // Read from the signal, not the `settings` copy above. Settings hydrate from
    // local storage after the first render, and an effect that reads no signal
    // runs exactly once: it applied the default and never saw the saved style,
    // so the components switched to iOS while every transition stayed Material.
    use_effect(move || match app_state.settings.read().platform_style {
        PlatformStyle::Auto => set_platform(detect_platform()),
        PlatformStyle::Ios => set_platform(TransitionPlatform::Ios),
        PlatformStyle::Material => set_platform(TransitionPlatform::Material),
    });

    // View-transition snapshots are painted on the document element, outside
    // the wrapper that carries the theme variables, so the stylesheet's
    // `--g3-color-bg` lookup missed and fell back to near-white. Mirroring the
    // real background onto the root keeps transitions on the theme's colour.
    use_effect(move || {
        // From the signal for the same reason as the platform above: a copy
        // taken at render time pinned the root to the default dark background,
        // so light mode kept a dark page wherever the root showed through.
        let chosen = app_state.settings.read().appearance;
        let background = tawny_theme(resolved_appearance(chosen, system_dark))
            .bg
            .clone();
        spawn(async move {
            let script = format!(
                "document.documentElement.style.setProperty('--route-transition-bg', {background:?});\
                 document.documentElement.style.setProperty('--g3-color-bg', {background:?});\
                 dioxus.send(true);"
            );
            let mut eval = document::eval(&script);
            let _ = eval.recv::<bool>().await;
        });
    });

    // Card times are shown in the viewer's zone. The effect reads no signal, so
    // it asks once; cards drawn before the answer lands show UTC until their
    // next render.
    use_effect(move || {
        spawn(async move {
            let mut eval = document::eval("dioxus.send(-new Date().getTimezoneOffset());");
            if let Ok(minutes) = eval.recv::<i32>().await {
                crate::models::set_local_utc_offset_minutes(minutes);
            }
        });
    });

    rsx! {
        AppWrapper {
            theme: tawny_theme(appearance),
            mode: component_mode,
            text_selection: false,
            // Routed covers name their own overlay region. Naming the wrapper
            // as well would produce duplicate `overlay` snapshots and make the
            // browser abort every sheet/player transition.
            route_transition_overlay: false,
            // Inside the wrapper so the setup screen is themed. A component
            // adds no DOM node of its own, so passing children through leaves
            // the snapshot structure untouched.
            AccountGate {
                Router::<Route> {}
            }
            AppOverlays {}
        }
    }
}

#[cfg(test)]
mod transition_tests {
    use super::*;
    use g3_route_transitions::NavigationTransition;

    /// Queue, History, and Settings all reach each other from the header, so a
    /// lateral move is a push in header order rather than the fallback fade.
    #[test]
    fn auxiliary_peers_slide_in_header_order() {
        let queue = Route::QueuePage {};
        let history = Route::HistoryPage {};
        let settings = Route::SettingsPage {};

        assert_eq!(queue.transition_to(&history), NavigationTransition::Forward);
        assert_eq!(
            history.transition_to(&settings),
            NavigationTransition::Forward
        );
        assert_eq!(
            queue.transition_to(&settings),
            NavigationTransition::Forward
        );

        assert_eq!(
            history.transition_to(&queue),
            NavigationTransition::Backward
        );
        assert_eq!(
            settings.transition_to(&history),
            NavigationTransition::Backward
        );
        assert_eq!(
            settings.transition_to(&queue),
            NavigationTransition::Backward
        );
    }

    /// Reached from a header action rather than a tab, the three present as
    /// sheets over whichever destination launched them. The forward edges that
    /// order them against each other must not steal that.
    #[test]
    fn auxiliary_pages_present_as_sheets_over_a_root() {
        let feed = Route::Feed {};
        let queue = Route::QueuePage {};

        assert_eq!(
            feed.transition_to(&queue),
            NavigationTransition::PresentSheet
        );
        assert_eq!(
            queue.transition_to(&feed),
            NavigationTransition::DismissSheet
        );
        assert_eq!(queue.transition_back(), NavigationTransition::DismissSheet);
    }

    /// Opening a channel from the watch sheet dismisses the sheet. A `forward`
    /// edge would read better going in, but it is bidirectional: it would also
    /// turn channel-to-watch into a push, and opening a video has to cover.
    #[test]
    fn opening_a_video_from_a_channel_still_covers() {
        let channel = Route::ChannelDetail {
            id: "channel".into(),
        };
        let watch = Route::VideoDetail { id: "video".into() };

        assert_eq!(
            channel.transition_to(&watch),
            NavigationTransition::PresentSheet
        );
        assert_eq!(
            watch.transition_to(&channel),
            NavigationTransition::DismissSheet
        );
    }

    #[test]
    fn primary_destinations_cross_fade() {
        assert_eq!(
            Route::Feed {}.transition_to(&Route::Subscriptions {}),
            NavigationTransition::CrossFade
        );
    }

    #[test]
    fn detail_pages_push_and_pop() {
        let root = Route::Playlists {};
        let detail = Route::PlaylistDetail {
            id: "playlist:test".into(),
        };

        assert_eq!(root.transition_to(&detail), NavigationTransition::Forward);
        assert_eq!(detail.transition_back(), NavigationTransition::Backward);
    }

    #[test]
    fn player_covers_and_uncovers_the_current_page() {
        let root = Route::Feed {};
        let player = Route::VideoDetail { id: "video".into() };

        assert_eq!(
            root.transition_to(&player),
            NavigationTransition::PresentSheet
        );
        assert_eq!(player.transition_back(), NavigationTransition::DismissSheet);
    }

    #[test]
    fn opening_a_video_from_a_header_sheet_replaces_that_sheet() {
        let watch = Route::VideoDetail { id: "video".into() };
        for sheet in [Route::QueuePage {}, Route::HistoryPage {}] {
            assert_eq!(
                sheet.transition_to(&watch),
                NavigationTransition::PresentSheet
            );
            assert!(sheet.replaces_history(&watch));
        }
        assert!(!Route::Feed {}.replaces_history(&watch));
        assert_eq!(watch.transition_back(), NavigationTransition::DismissSheet);
    }

    #[test]
    fn advancing_between_videos_reuses_the_watch_history_entry() {
        let first = Route::VideoDetail { id: "first".into() };
        let next = Route::VideoDetail { id: "next".into() };

        assert!(first.replaces_history(&next));
        assert!(!Route::Feed {}.replaces_history(&next));
    }
}

#[cfg(test)]
mod appearance_tests {
    use super::*;

    #[test]
    fn auto_follows_the_system_where_the_page_cannot_see_it() {
        assert_eq!(
            resolved_appearance(Appearance::Auto, Some(true)),
            Appearance::Dark
        );
        assert_eq!(
            resolved_appearance(Appearance::Auto, Some(false)),
            Appearance::Light
        );
        // No answer: the page's own media query decides.
        assert_eq!(
            resolved_appearance(Appearance::Auto, None),
            Appearance::Auto
        );
        // A theme the viewer picked is never overridden.
        assert_eq!(
            resolved_appearance(Appearance::Light, Some(true)),
            Appearance::Light
        );
        assert_eq!(
            resolved_appearance(Appearance::Dark, Some(false)),
            Appearance::Dark
        );
    }
}
