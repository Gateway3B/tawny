use dioxus::prelude::*;
use dioxus_icons::lucide::{
    Captions, ChevronLeft, ChevronRight, Maximize2, Minimize2, PanelTopClose, PanelTopOpen, Pause,
    PictureInPicture, Play, Settings2, ToggleLeft, ToggleRight, Volume2, VolumeX,
};
use g3_route_transitions::{animated_back_or_navigate, animated_navigate};

use super::NativePlayerBridges;
use crate::{app::Route, state::AppState};

/// Controls subscribe to presentation state without owning the media element.
#[component]
pub(super) fn PlayerControls(expanded: bool, on_advance: EventHandler<String>) -> Element {
    let app_state = use_context::<AppState>();
    // Progress changes do not change the controls' video identity or labels.
    let control_video = use_memo(move || {
        app_state.active_video.read().as_ref().map(|video| {
            (
                video.id.clone(),
                video.title.clone(),
                video.is_short,
                video.audio_only,
            )
        })
    });
    let Some((video_id, playback_title, is_short, audio_only_active)) = control_video() else {
        return rsx! {};
    };

    let audio_only_video_id = video_id.clone();
    let mut captions_enabled = app_state.captions_enabled;
    let captions_for_toggle = app_state.active_captions;
    let mut chapters_sheet = app_state.chapters_sheet_open;
    let current_speed = app_state.settings.read().speed_for(is_short);
    let theater_mode = app_state.settings.read().theater_mode;
    let mut settings_for_theater = app_state.settings;
    let autoplay_enabled = app_state.settings.read().autoplay_for(is_short);
    let autoplay_video_id = video_id.clone();
    let mut autoplay_settings = app_state.settings;
    // A run is anything with somewhere to go: a queue ahead, a playlist being
    // run, a video stepped back from, or any combination. Each direction is
    // independently rendered below, so an unavailable side is absent rather than
    // a permanent disabled button.
    //
    // Asked of the run rather than of the queue's length: a queue holding only a
    // spent playlist marker has nothing left to play, and offering a Next that
    // does nothing is worse than not offering one.
    let has_next_queued = app_state.has_next_in_run(&video_id);
    let can_step_back = app_state.can_step_back_in_run(&video_id);
    // Naming the playlist is the only thing that tells a viewer these arrows
    // walk the list they pressed play on rather than a queue they hand-built.
    let run_name = app_state
        .running_playlist_name()
        .unwrap_or_else(|| "queue".to_string());
    let next_label = format!("Next in {run_name}");
    let previous_label = format!("Previous in {run_name}");
    let step_forward_id = video_id.clone();
    let step_back_id = video_id.clone();

    rsx! {
        div {
            class: "tawny-player-controls controls-visible",
            role: "group",
            aria_label: "Video controls",
            div { class: "player-controls-scrim" }
            div { class: "player-controls-top",
                // Same affordance as every other page: this leaves
                // the video, it does not shrink it in place.
                button {
                    r#type: "button",
                    class: "player-control-button player-minimize-button",
                    aria_label: "Back",
                    title: "Back",
                    // Deliberately the same call the swipe-down
                    // makes: popping history instead picks its
                    // animation from whatever came before, which
                    // is not always the page under the sheet.
                    "data-player-action": "back",
                    ChevronLeft { size: 26 }
                }
                strong { class: "player-overlay-title", "{playback_title}" }
                div { class: "player-controls-top-actions",
                    // Autoplay is a per-sitting decision as often
                    // as a preference, so it is reachable without
                    // leaving the video. It writes the same setting
                    // the settings page does, split by kind: a run
                    // of Shorts and a long video are not the same
                    // choice.
                    button {
                        r#type: "button",
                        class: if autoplay_enabled {
                            "player-control-button player-autoplay-button is-on"
                        } else {
                            "player-control-button player-autoplay-button"
                        },
                        aria_label: if is_short { "Autoplay Shorts" } else { "Autoplay videos" },
                        title: if autoplay_enabled { "Autoplay is on" } else { "Autoplay is off" },
                        aria_pressed: autoplay_enabled.to_string(),
                        onclick: move |event: Event<MouseData>| {
                            // Root owns media-surface taps. This is
                            // an independent state control, never a
                            // request to toggle playback too.
                            event.stop_propagation();
                            let enabled = autoplay_settings().autoplay_for(is_short);
                            autoplay_settings.write().set_autoplay_for(is_short, !enabled);
                        },
                        // A switch rather than a glyph: this button
                        // reports a state as much as it invites a
                        // tap, and the list icon it used to carry
                        // said "queue" - which is the thing beside
                        // it in the header.
                        if autoplay_enabled {
                            ToggleRight { size: 23 }
                        } else {
                            ToggleLeft { size: 23 }
                        }
                    }
                    // Clicked by the player JS when a video ends.
                    // Advancing is decided here because the queue
                    // and the setting both live in Rust.
                    button {
                        r#type: "button",
                        class: "player-caption-state-bridge",
                        tabindex: "-1",
                        aria_hidden: "true",
                        "data-player-autoplay-next": "",
                        // Read by the JS at the end of a video, so a
                        // stage about to move on shows the next one
                        // loading rather than its own paused controls.
                        "data-has-next": has_next_queued.to_string(),
                        onclick: move |_| {
                            let finished = autoplay_video_id.clone();
                            if let Some(next) = app_state.take_next_queued(&finished) {
                                on_advance.call(next);
                            }
                        },
                    }
                    button {
                        r#type: "button",
                        class: "player-control-button",
                        aria_label: "Picture in picture",
                        title: "Picture in picture",
                        "data-player-action": "pip",
                        PictureInPicture { size: 21 }
                    }
                    button {
                        r#type: "button",
                        class: "player-control-button",
                        aria_label: "Player settings",
                        title: "Player settings",
                        "data-player-action": "settings",
                        Settings2 { size: 22 }
                    }
                }
            }
            // Seeking has no buttons here on purpose - it is a
            // double-tap on either side of the video, and the arrow
            // keys on a keyboard - but moving through a queue is not
            // seeking, and there is no gesture for it. The pair only
            // appears when there is a run to move through.
            div { class: "player-controls-center",
                if can_step_back {
                    button {
                        r#type: "button",
                        class: "player-control-button player-step-button player-step-previous",
                        aria_label: "{previous_label}",
                        title: "{previous_label}",
                        onclick: move |_| {
                            let current = step_back_id.clone();
                            spawn(async move {
                                let Some(previous) = app_state.step_back_in_run(&current) else {
                                    return;
                                };
                                animated_navigate(Route::VideoDetail { id: previous }).await;
                            });
                        },
                        ChevronLeft { size: 27 }
                    }
                }
                button {
                    r#type: "button",
                    class: "player-control-button player-play-button",
                    aria_label: "Play",
                    title: "Play or pause",
                    "data-player-action": "toggle",
                    span { class: "player-play-icon", Play { size: 38, fill: "currentColor" } }
                    span { class: "player-pause-icon", Pause { size: 38, fill: "currentColor" } }
                }
                if has_next_queued {
                    button {
                        r#type: "button",
                        class: "player-control-button player-step-button player-step-next",
                        aria_label: "{next_label}",
                        title: "{next_label}",
                        onclick: move |_| {
                            let current = step_forward_id.clone();
                            if let Some(next) = app_state.take_next_queued(&current) {
                                on_advance.call(next);
                            }
                        },
                        ChevronRight { size: 27 }
                    }
                }
            }
            div { class: "player-options-menu", "data-player-options-menu": "", hidden: true,
                div { class: "player-options-section",
                    strong { "Playback speed" }
                    div { class: "player-option-grid",
                        for speed in [0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0] {
                            button {
                                r#type: "button",
                                class: "player-option-button",
                                "data-player-speed": "{speed}",
                                "{speed}×"
                            }
                        }
                    }
                }
                div { class: "player-options-section",
                    strong { "Audio only" }
                    div { class: "player-option-grid",
                        button {
                            r#type: "button",
                            class: if audio_only_active { "player-option-button selected" } else { "player-option-button" },
                            onclick: move |_| {
                                let active = app_state.active_video.peek().clone();
                                if let Some(video) = active.filter(|video| video.id == audio_only_video_id) {
                                    app_state.set_audio_only(&video, !audio_only_active);
                                }
                            },
                            if audio_only_active { "On" } else { "Off" }
                        }
                    }
                }
                div { class: "player-options-section",
                    strong { "Quality" }
                    div { class: "player-option-grid", "data-player-quality-options": "",
                        button { r#type: "button", class: "player-option-button selected", "Auto" }
                    }
                }
            }
            // Announces an automatic skip. Without it a video that
            // jumps forward on its own reads as a fault.
            div {
                class: "player-sponsor-notice",
                "data-player-sponsor-notice": "",
                role: "status",
                aria_live: "polite",
            }
            div { class: "player-controls-bottom",
                div { class: "player-control-row",
                    span { class: "player-time",
                        span { "data-player-current-time": "", "0:00" }
                        span { class: "player-time-separator", " / " }
                        span { "data-player-duration": "", "0:00" }
                        // Current chapter, to the right of the clock; tapping it
                        // opens the chapter list rather than making the user scrub.
                    button {
                        r#type: "button",
                        class: "player-chapter-label",
                        "data-player-chapter-label": "",
                        "data-player-action": "chapters",
                        hidden: true,
                        span { class: "player-chapter-title", "data-player-chapter-title": "" }
                        ChevronRight { class: "player-chapter-chevron", size: 15 }
                    }
                    }
                    span { class: "player-control-spacer" }
                    button {
                        r#type: "button",
                        class: "player-control-button player-mute-button",
                        aria_label: "Mute",
                        title: "Mute",
                        "data-player-action": "mute",
                        span { class: "player-volume-icon", Volume2 { size: 20 } }
                        span { class: "player-muted-icon", VolumeX { size: 20 } }
                    }
                    button {
                        r#type: "button",
                        class: "player-control-button player-captions-button",
                        aria_label: "Captions",
                        title: "Captions",
                        "data-player-action": "captions",
                        Captions { size: 21 }
                    }
                    button {
                        r#type: "button",
                        class: "player-caption-state-bridge",
                        tabindex: "-1",
                        aria_hidden: "true",
                        "data-player-chapters-open": "",
                        onclick: move |_| chapters_sheet.set(true),
                    }
                    NativePlayerBridges { title: playback_title.clone() }
                    button {
                        r#type: "button",
                        class: "player-caption-state-bridge",
                        tabindex: "-1",
                        aria_hidden: "true",
                        "data-player-minimize": "",
                        // Minimizing dismisses the watch sheet, so it has to pop
                        // history rather than push Feed: pushing sent anyone who
                        // opened the video from Subscriptions or a playlist to the
                        // wrong page, and left the watch route ahead in history.
                        // Feed is only the deep-link fallback.
                        onclick: move |_| { spawn(animated_back_or_navigate(Route::Feed {})); },
                    }
                    button {
                        r#type: "button",
                        class: "player-caption-state-bridge",
                        tabindex: "-1",
                        aria_hidden: "true",
                        "data-player-caption-toggle": "",
                        onclick: move |_| {
                            if !captions_for_toggle().is_empty() {
                                captions_enabled.toggle();
                            }
                        },
                    }
                    button {
                        r#type: "button",
                        class: "player-control-button player-speed-control",
                        aria_label: "Playback speed",
                        title: "Change playback speed",
                        "data-player-action": "speed",
                        span { "data-player-speed-label": "", "{current_speed}×" }
                    }
                    if expanded {
                        button {
                            r#type: "button",
                            class: "player-control-button player-theater-button",
                            aria_label: if theater_mode { "Exit theater mode" } else { "Enter theater mode" },
                            title: if theater_mode { "Exit theater mode" } else { "Theater mode" },
                            aria_pressed: theater_mode.to_string(),
                            onclick: move |event: Event<MouseData>| {
                                event.stop_propagation();
                                settings_for_theater.write().theater_mode = !theater_mode;
                            },
                            if theater_mode {
                                PanelTopClose { size: 21 }
                            } else {
                                PanelTopOpen { size: 21 }
                            }
                        }
                    }
                    // Both icons ship; CSS shows whichever matches
                    // the current state, the same way play/pause does.
                    button {
                        r#type: "button",
                        class: "player-control-button player-fullscreen-button",
                        aria_label: "Enter fullscreen",
                        title: "Fullscreen",
                        "data-player-action": "fullscreen",
                        span { class: "player-fullscreen-enter-icon", Maximize2 { size: 21 } }
                        span { class: "player-fullscreen-exit-icon", Minimize2 { size: 21 } }
                    }
                }
                div { class: "player-timeline",
                    div {
                        class: "player-seek-preview",
                        "data-player-seek-preview": "",
                        hidden: true,
                        div { class: "player-seek-preview-image", "data-player-preview-image": "" }
                        div { class: "player-seek-preview-copy",
                            strong { "data-player-preview-chapter": "" }
                            span { "data-player-preview-time": "", "0:00" }
                        }
                    }
                    input {
                        r#type: "range",
                        class: "player-progress",
                        min: "0",
                        max: "10000",
                        step: "1",
                        value: "0",
                        aria_label: "Video progress",
                        "data-player-progress": "",
                    }
                }
            }
        }
    }
}
