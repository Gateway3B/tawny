use crate::{
    app::Route,
    models::{Appearance, PlatformStyle, SponsorAction, SponsorCategory, SwipeActionKind},
    state::AppState,
    subscriptions_io::{SubscriptionFormat, export_subscriptions, parse_subscription_export},
};
use dioxus::prelude::*;
use g3_route_transitions::ROUTE_TRANSITION_OVERLAY_REGION_CLASS;

use super::{AccountSettings, BackendSettings, PageHeader};
use dioxus_icons::lucide::{Captions, Download, Gauge, Languages, RotateCw, ShieldCheck, Upload};
use g3_ui::{
    Button, ButtonFill, ButtonSize, Card, Color, Content, ContentWidth, Grid, GridColumns, Input,
    InputType, Item, List, ListLines, ListVariant, SegmentButton, SegmentGroup, Select,
    SelectOption, SelectWidth, Space, Stack, Text, TextVariant, Toggle,
};

/// Hand a generated file to the browser.
///
/// The export is built in Rust, so there is no URL to link to; a Blob and a
/// synthetic anchor click is the only way to produce a real download. The
/// object URL is revoked on the next tick, once the click has been dispatched.
fn download_text_file(file_name: &str, mime_type: &str, contents: String) {
    let file_name = serde_json::to_string(file_name).unwrap_or_else(|_| "\"export.txt\"".into());
    let mime_type = serde_json::to_string(mime_type).unwrap_or_else(|_| "\"text/plain\"".into());
    let contents = serde_json::to_string(&contents).unwrap_or_else(|_| "\"\"".into());
    spawn(async move {
        let script = format!(
            r#"
            const blob = new Blob([{contents}], {{ type: {mime_type} }});
            const url = URL.createObjectURL(blob);
            const anchor = document.createElement('a');
            anchor.href = url;
            anchor.download = {file_name};
            document.body.appendChild(anchor);
            anchor.click();
            anchor.remove();
            setTimeout(() => URL.revokeObjectURL(url), 0);
            dioxus.send(true);
            "#
        );
        let mut eval = document::eval(&script);
        let _ = eval.recv::<bool>().await;
    });
}

/// The languages offered for audio dubs and for captions.
const PREFERRED_LANGUAGES: [(&str, &str); 10] = [
    ("en", "English"),
    ("es", "Spanish"),
    ("fr", "French"),
    ("de", "German"),
    ("it", "Italian"),
    ("pt", "Portuguese"),
    ("ja", "Japanese"),
    ("ko", "Korean"),
    ("zh", "Chinese"),
    ("hi", "Hindi"),
];

/// A number field that writes a parsed value back through `apply`.
#[component]
fn NumberSetting(
    label: String,
    value: Signal<String>,
    min: f64,
    max: f64,
    apply: Callback<u64>,
) -> Element {
    rsx! {
        Input {
            label,
            value,
            input_type: InputType::Number,
            min,
            max,
            onchange: move |text: String| {
                if let Ok(number) = text.parse::<u64>() {
                    apply.call(number);
                }
            },
        }
    }
}

/// One swipe direction: what the gesture does, and where "Add to playlist"
/// files the video. The playlist picker only matters for that action.
#[component]
fn SwipeSetting(label: String, right: bool) -> Element {
    let mut app_state = use_context::<AppState>();
    let settings = app_state.settings();
    let (stored_action, stored_playlist) = if right {
        (
            settings.swipe_right_action,
            settings.swipe_right_playlist_id.clone(),
        )
    } else {
        (
            settings.swipe_left_action,
            settings.swipe_left_playlist_id.clone(),
        )
    };
    let action = use_signal(|| stored_action);
    let playlist = use_signal(|| stored_playlist);
    let playlists = app_state.with_viewer(|viewer| {
        viewer
            .playlists
            .iter()
            .map(|playlist| SelectOption::new(playlist.id.clone(), playlist.name.clone()))
            .collect::<Vec<_>>()
    });

    rsx! {
        Item {
            wrap: true,
            label: label.clone(),
            description: "What the gesture does",
            end: rsx! {
                Select {
                    value: action,
                    aria_label: "{label} action",
                    width: SelectWidth::Fit,
                    options: SwipeActionKind::ALL
                        .iter()
                        .map(|kind| SelectOption::new(*kind, kind.label()).trigger_label(kind.trigger_label()))
                        .collect::<Vec<_>>(),
                    onchange: move |kind: SwipeActionKind| {
                        let mut settings = app_state.settings.write();
                        if right { settings.swipe_right_action = kind } else { settings.swipe_left_action = kind }
                    },
                }
            },
        }
        if action() == SwipeActionKind::AddToPlaylist {
            Item {
                wrap: true,
                label: "Destination playlist",
                end: rsx! {
                    Select {
                        value: playlist,
                        aria_label: "{label} playlist",
                        width: SelectWidth::Fit,
                        options: playlists,
                        onchange: move |id: String| {
                            let mut settings = app_state.settings.write();
                            if right { settings.swipe_right_playlist_id = id } else { settings.swipe_left_playlist_id = id }
                        },
                    }
                },
            }
        }
    }
}

#[component]
pub fn SettingsPage() -> Element {
    let mut app_state = use_context::<AppState>();

    let settings = app_state.settings();
    let autoplay = use_signal(|| settings.autoplay);
    let shorts_autoplay = use_signal(|| settings.shorts_autoplay);
    let audio_language = use_signal(|| settings.preferred_audio_language.clone());
    let caption_language = use_signal(|| settings.preferred_caption_language.clone());
    let prefer_sabr = use_signal(|| settings.prefer_sabr);
    let appearance = use_signal(|| settings.appearance);
    let platform_style = use_signal(|| settings.platform_style);
    let hide_watched = use_signal(|| settings.hide_watched);
    let auto_landscape = use_signal(|| settings.auto_landscape_fullscreen);
    let video_short_minutes = use_signal(|| (settings.video_short_max_seconds / 60).to_string());
    let video_medium_minutes = use_signal(|| (settings.video_medium_max_seconds / 60).to_string());
    let shorts_short_seconds = use_signal(|| settings.shorts_short_max_seconds.to_string());
    let shorts_medium_seconds = use_signal(|| settings.shorts_medium_max_seconds.to_string());

    rsx! {
        // One surface, one snapshot: the header rides up with the body it
        // belongs to instead of morphing in place while the page slides
        // underneath it.
        div { class: "auxiliary-cover {ROUTE_TRANSITION_OVERLAY_REGION_CLASS}",
            PageHeader { title: "Settings".to_string(), back_to: Route::Feed {} }
            Content { width: ContentWidth::Readable,
                Stack { gap: Space::Lg,
                    // First, deliberately. Which server you talk to and who you
                    // are on it decide what every other setting below is even
                    // describing.
                    AccountSettings {}
                    BackendSettings {}
                    Card { title: "g3tech",
                        Button { fill: ButtonFill::Clear,
                            onclick: move |_| { spawn(g3_route_transitions::animated_navigate(Route::AboutG3tech {})); },
                            "About g3tech and other apps"
                        }
                    }
                    SponsorBlockSettingsCard {}

                    Card { title: "Swipe actions",
                        List { variant: ListVariant::Filled, lines: ListLines::Inset,
                            SwipeSetting { label: "Swipe right", right: true }
                            SwipeSetting { label: "Swipe left", right: false }
                        }
                    }

                    Card { title: "Playback",
                        List { variant: ListVariant::Filled, lines: ListLines::Inset,
                            Item {
                                wrap: true,
                                start: rsx! { Gauge { size: 19 } },
                                label: "Video speed",
                                description: "Remembered for long-form video",
                                metadata: format!("{}×", settings.playback_speed),
                            }
                            Item {
                                wrap: true,
                                start: rsx! { Gauge { size: 19 } },
                                label: "Shorts speed",
                                description: "Remembered separately from video",
                                metadata: format!("{}×", settings.shorts_playback_speed),
                            }
                            Item {
                                wrap: true,
                                label: "Autoplay videos",
                                description: "Continue with the next queued video",
                                end: rsx! {
                                    Toggle {
                                        checked: autoplay,
                                        aria_label: "Autoplay videos",
                                        onchange: move |enabled| app_state.settings.write().autoplay = enabled,
                                    }
                                },
                            }
                            Item {
                                wrap: true,
                                label: "Autoplay Shorts",
                                description: "Remembered separately from video",
                                end: rsx! {
                                    Toggle {
                                        checked: shorts_autoplay,
                                        aria_label: "Autoplay Shorts",
                                        onchange: move |enabled| app_state.settings.write().shorts_autoplay = enabled,
                                    }
                                },
                            }
                            Item {
                                wrap: true,
                                start: rsx! { Languages { size: 19 } },
                                label: "Audio language",
                                description: "Use this dub when a video offers one",
                                end: rsx! {
                                    Select {
                                        value: audio_language,
                                        aria_label: "Audio language",
                                        width: SelectWidth::Fit,
                                        options: PREFERRED_LANGUAGES
                                            .iter()
                                            .map(|(code, label)| SelectOption::new(code.to_string(), *label))
                                            .collect::<Vec<_>>(),
                                        onchange: move |code: String| app_state.settings.write().preferred_audio_language = code,
                                    }
                                },
                            }
                            Item {
                                wrap: true,
                                start: rsx! { Captions { size: 19 } },
                                label: "Captions language",
                                description: "Shown when a video has them",
                                end: rsx! {
                                    Select {
                                        value: caption_language,
                                        aria_label: "Captions language",
                                        width: SelectWidth::Fit,
                                        options: PREFERRED_LANGUAGES
                                            .iter()
                                            .map(|(code, label)| SelectOption::new(code.to_string(), *label))
                                            .collect::<Vec<_>>(),
                                        onchange: move |code: String| {
                                            app_state.settings.write().preferred_caption_language = code;
                                            app_state.select_preferred_caption();
                                        },
                                    }
                                },
                            }
                            Item {
                                wrap: true,
                                start: rsx! { RotateCw { size: 19 } },
                                label: "Rotate horizontal fullscreen video",
                                description: "Switch phones to landscape automatically",
                                end: rsx! {
                                    Toggle {
                                        checked: auto_landscape,
                                        aria_label: "Rotate horizontal fullscreen video",
                                        onchange: move |enabled| app_state.settings.write().auto_landscape_fullscreen = enabled,
                                    }
                                },
                            }
                            Item {
                                wrap: true,
                                start: rsx! { ShieldCheck { size: 19 } },
                                label: "Prefer SABR",
                                description: "Use adaptive streaming when the stream supports it",
                                end: rsx! {
                                    Toggle {
                                        checked: prefer_sabr,
                                        aria_label: "Prefer SABR",
                                        onchange: move |enabled| app_state.settings.write().prefer_sabr = enabled,
                                    }
                                },
                            }
                        }
                    }

                    Card { title: "Appearance & feed",
                        Stack { gap: Space::Md,
                            SegmentGroup {
                                value: platform_style,
                                label: "Style",
                                onchange: move |picked: PlatformStyle| app_state.settings.write().platform_style = picked,
                                SegmentButton { value: PlatformStyle::Auto, "Auto" }
                                SegmentButton { value: PlatformStyle::Ios, "iOS" }
                                SegmentButton { value: PlatformStyle::Material, "Material" }
                            }
                            SegmentGroup {
                                value: appearance,
                                label: "Theme",
                                onchange: move |picked: Appearance| app_state.settings.write().appearance = picked,
                                SegmentButton { value: Appearance::Auto, "Auto" }
                                SegmentButton { value: Appearance::Dark, "Dark" }
                                SegmentButton { value: Appearance::Light, "Light" }
                            }
                            Toggle {
                                checked: hide_watched,
                                label: "Hide watched videos",
                                helper: "Keep the feed focused on what is new",
                                onchange: move |hidden| app_state.settings.write().hide_watched = hidden,
                            }
                        }
                    }

                    Card {
                        title: "Duration filters",
                        subtitle: "Set where Short ends and Medium begins for each kind of upload.",
                        Grid { columns: GridColumns::Fit(14.0),
                            NumberSetting {
                                label: "Video · Short max (minutes)",
                                value: video_short_minutes,
                                min: 1.0,
                                max: 240.0,
                                apply: move |minutes: u64| {
                                    let mut settings = app_state.settings.write();
                                    settings.video_short_max_seconds = minutes * 60;
                                    settings.video_medium_max_seconds = settings.video_medium_max_seconds.max((minutes + 1) * 60);
                                },
                            }
                            NumberSetting {
                                label: "Video · Medium max (minutes)",
                                value: video_medium_minutes,
                                min: 2.0,
                                max: 600.0,
                                apply: move |minutes: u64| {
                                    let mut settings = app_state.settings.write();
                                    settings.video_medium_max_seconds = (minutes * 60).max(settings.video_short_max_seconds + 60);
                                },
                            }
                            NumberSetting {
                                label: "Shorts · Short max (seconds)",
                                value: shorts_short_seconds,
                                min: 5.0,
                                max: 170.0,
                                apply: move |seconds: u64| {
                                    let mut settings = app_state.settings.write();
                                    settings.shorts_short_max_seconds = seconds;
                                    settings.shorts_medium_max_seconds = settings.shorts_medium_max_seconds.max(seconds + 1);
                                },
                            }
                            NumberSetting {
                                label: "Shorts · Medium max (seconds)",
                                value: shorts_medium_seconds,
                                min: 6.0,
                                max: 180.0,
                                apply: move |seconds: u64| {
                                    let mut settings = app_state.settings.write();
                                    settings.shorts_medium_max_seconds = seconds.max(settings.shorts_short_max_seconds + 1);
                                },
                            }
                        }
                    }

                    SubscriptionTransferCard {}

                }
            }
        }
    }
}

/// Import from, and export to, other YouTube clients.
#[component]
fn SubscriptionTransferCard() -> Element {
    let app_state = use_context::<AppState>();
    rsx! {
        Card {
            title: "Subscriptions",
            subtitle: "Import a list from Google Takeout, Piped, LibreTube, or any OPML feed reader. Tawny's own format is the only one that also carries your per-channel Videos/Shorts choice and your groups.",
            Stack { gap: Space::Md,
                List { variant: ListVariant::Filled,
                    Item {
                        wrap: true,
                        start: rsx! { Upload { size: 18 } },
                        label: "Import subscriptions",
                        description: "Choose a .json, .csv, or .opml export",
                        onclick: move |_| {
                            spawn(async move {
                                let mut open = document::eval(
                                    "document.getElementById('tawny-subscription-import')?.click(); dioxus.send(true);",
                                );
                                let _ = open.recv::<bool>().await;
                            });
                        },
                    }
                }
                // Driven by the row above rather than shown directly: a bare
                // file input cannot be styled to match the list.
                input {
                    id: "tawny-subscription-import",
                    r#type: "file",
                    accept: ".json,.csv,.opml,.xml,application/json,text/csv,text/xml",
                    class: "sr-only",
                    onchange: move |event: FormEvent| {
                        async move {
                            let Some(file) = event.files().first().cloned() else { return };
                            let Ok(contents) = file.read_string().await else {
                                app_state.show_toast("Could not read that file", Color::Danger);
                                return;
                            };
                            match parse_subscription_export(&contents) {
                                Ok(parsed) => {
                                    let format = parsed.format;
                                    let summary = app_state.import_subscriptions(parsed);
                                    let color = if summary.changed() { Color::Success } else { Color::Neutral };
                                    app_state.show_toast(
                                        format!("{} \u{2014} {}", format.label(), summary.message()),
                                        color,
                                    );
                                }
                                Err(error) => {
                                    app_state.show_toast(format!("Import failed: {error}"), Color::Danger);
                                }
                            }
                        }
                    },
                }
                Text { variant: TextVariant::Overline, "Export" }
                Stack { horizontal: true, wrap: true, gap: Space::Sm,
                    for format in [
                        SubscriptionFormat::Tawny,
                        SubscriptionFormat::Piped,
                        SubscriptionFormat::TakeoutCsv,
                        SubscriptionFormat::Opml,
                    ] {
                        Button {
                            key: "{format.file_name()}",
                            fill: ButtonFill::Outline,
                            color: Color::Neutral,
                            size: ButtonSize::Sm,
                            start: rsx! { Download { size: 16 } },
                            onclick: move |_| {
                                let viewer = app_state.viewer.peek().as_ref().and_then(|viewer| viewer.as_ref().ok()).cloned().unwrap_or_default();
                                let subscribed = viewer.subscriptions.len();
                                if subscribed == 0 {
                                    app_state.show_toast("No subscriptions to export", Color::Neutral);
                                    return;
                                }
                                let contents = export_subscriptions(
                                    format,
                                    &viewer.subscriptions,
                                    &viewer.subscription_groups,
                                );
                                download_text_file(format.file_name(), format.mime_type(), contents);
                                app_state.show_toast(
                                    format!("Exported {subscribed} subscriptions as {}", format.label()),
                                    Color::Success,
                                );
                            },
                            "{format.label()}"
                        }
                    }
                }
            }
        }
    }
}

/// Per-category SponsorBlock behaviour.
///
/// One row per category rather than a single on/off, because the categories are
/// not equivalent: a sponsor read is never wanted, while an intro often is the
/// video. The defaults mirror the browser extension, so someone who already uses
/// it finds Tawny behaving the way they expect.
#[component]
fn SponsorBlockSettingsCard() -> Element {
    let mut app_state = use_context::<AppState>();
    let sponsor = app_state.settings.read().sponsor_block.clone();
    let enabled_value = sponsor.enabled;
    let enabled = use_signal(|| enabled_value);
    let notify = use_signal(|| sponsor.notify_on_skip);

    rsx! {
        Card { title: "SponsorBlock",
            Stack { gap: Space::Md,
                Toggle {
                    checked: enabled,
                    label: "Use SponsorBlock",
                    helper: "Segments marked by the community",
                    onchange: move |value| app_state.settings.write().sponsor_block.enabled = value,
                }
                if enabled_value {
                    Toggle {
                        checked: notify,
                        label: "Announce skips",
                        helper: "So a jump does not look like a fault",
                        onchange: move |value| app_state.settings.write().sponsor_block.notify_on_skip = value,
                    }
                    // Rows rather than one select each: the explanations need to
                    // wrap, and they are the context for choosing an action.
                    List { variant: ListVariant::Filled, lines: ListLines::Inset,
                        for category in SponsorCategory::ALL {
                            SponsorCategoryRow { key: "{category.api_name()}", category }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn SponsorCategoryRow(category: SponsorCategory) -> Element {
    let mut app_state = use_context::<AppState>();
    let value = use_signal(|| app_state.settings.peek().sponsor_block.action_for(category));
    rsx! {
        Item {
            label: category.label(),
            description: category.description(),
            wrap: true,
            // Matches the timeline colour of the category.
            start: rsx! {
                span {
                    class: "inline-block size-3.5 rounded",
                    style: "background: {category.color()}; box-shadow: inset 0 0 0 1px rgb(0 0 0 / 0.35);",
                    aria_hidden: "true",
                }
            },
            end: rsx! {
                Select {
                    value,
                    aria_label: "{category.label()}",
                    width: SelectWidth::Fit,
                    options: SponsorAction::ALL
                        .iter()
                        .map(|action| SelectOption::new(*action, action.label()).trigger_label(action.trigger_label()))
                        .collect::<Vec<_>>(),
                    onchange: move |action: SponsorAction| {
                        app_state.settings.write().sponsor_block.set_action(category, action);
                    },
                }
            },
        }
    }
}
