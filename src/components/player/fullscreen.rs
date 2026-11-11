use dioxus::prelude::*;

fn set_window_fullscreen(enabled: bool) {
    #[cfg(all(feature = "desktop", not(feature = "mobile")))]
    dioxus::desktop::window().set_fullscreen(enabled);

    #[cfg(not(all(feature = "desktop", not(feature = "mobile"))))]
    let _ = enabled;
}

/// The WebView's fullscreen only fills its client area. Follow the player's
/// fullscreen events at the window level as well, restoring the desktop when
/// fullscreen ends or the controls are removed.
#[component]
pub(super) fn NativeFullscreenBridge() -> Element {
    use_drop(|| set_window_fullscreen(false));

    rsx! {
        button {
            r#type: "button",
            class: "player-caption-state-bridge",
            tabindex: "-1",
            aria_hidden: "true",
            "data-player-native-system-bars-hide": "",
            onclick: move |_| set_window_fullscreen(true),
        }
        button {
            r#type: "button",
            class: "player-caption-state-bridge",
            tabindex: "-1",
            aria_hidden: "true",
            "data-player-native-system-bars-show": "",
            onclick: move |_| set_window_fullscreen(false),
        }
    }
}
