//! The Downloads page and the watch page's Download button. The downloads
//! themselves are in `crate::downloads`.

use crate::{
    app::Route,
    downloads::{
        DownloadEntry, DownloadQuality, DownloadState, remove_download, size_label, start_download,
    },
    models::Video,
    state::AppState,
};
use dioxus::prelude::*;
use dioxus_icons::lucide::{CircleCheck, Download, Trash2, X};
use g3_route_transitions::animated_navigate;
use g3_ui::{
    BottomSheet, Button, ButtonExpand, ButtonFill, ButtonSize, Card, Color, Content, ContentWidth,
    EmptyState, Item, ItemDetail, List, ListLines, Progress, Space, Stack, Text, TextTone,
};

use super::PageHeader;

#[component]
pub fn Downloads() -> Element {
    let app_state = use_context::<AppState>();
    let entries = app_state.downloads.read().clone();
    let kept_bytes = entries
        .iter()
        .filter(|entry| entry.state == DownloadState::Done)
        .map(|entry| entry.bytes)
        .sum::<u64>();
    let kept_count = entries
        .iter()
        .filter(|entry| entry.state == DownloadState::Done)
        .count();

    rsx! {
        PageHeader { title: "Downloads", back_to: Route::Playlists {} }
        Content { width: ContentWidth::Readable,
            if entries.is_empty() {
                EmptyState {
                    title: "Nothing downloaded",
                    icon: rsx! { Download { size: 40 } },
                    "Use Download under a video to keep it on this device and watch it without a connection."
                }
            } else {
                Stack { gap: Space::Md,
                    Text { tone: TextTone::Secondary,
                        if kept_count == 1 {
                            "1 video on this device · {size_label(kept_bytes)}"
                        } else {
                            "{kept_count} videos on this device · {size_label(kept_bytes)}"
                        }
                    }
                    for entry in entries {
                        DownloadRow { key: "{entry.video_id}", entry }
                    }
                }
            }
        }
    }
}

#[component]
fn DownloadRow(entry: DownloadEntry) -> Element {
    let app_state = use_context::<AppState>();
    let artwork = entry
        .thumbnail
        .clone()
        .or_else(|| {
            entry
                .video
                .as_ref()
                .map(|video| video.thumbnail_url.clone())
        })
        .unwrap_or_default();
    let channel = entry
        .video
        .as_ref()
        .map(|video| video.channel_name.clone())
        .unwrap_or_default();
    let subtitle = if channel.is_empty() {
        entry.summary()
    } else {
        format!("{channel} · {}", entry.summary())
    };
    let downloading = entry.state == DownloadState::Downloading;
    let open_video = entry.video.clone();
    let open_id = entry.video_id.clone();
    let remove_id = entry.video_id.clone();
    let remove_title = entry.title();

    rsx! {
        Card {
            title: entry.title(),
            subtitle,
            start: rsx! {
                img {
                    class: "w-24 shrink-0 aspect-video rounded-md object-cover",
                    src: "{artwork}",
                    alt: "",
                }
            },
            end: rsx! {
                Button {
                    fill: ButtonFill::Clear,
                    color: Color::Neutral,
                    size: ButtonSize::Sm,
                    aria_label: if downloading { format!("Cancel downloading {remove_title}") } else { format!("Remove {remove_title}") },
                    onclick: move |event: MouseEvent| {
                        event.stop_propagation();
                        let id = remove_id.clone();
                        spawn(async move {
                            remove_download(&id).await;
                            app_state.show_toast(
                                if downloading { "Download cancelled" } else { "Download removed" },
                                Color::Neutral,
                            );
                        });
                    },
                    if downloading {
                        X { size: 18 }
                    } else {
                        Trash2 { size: 18 }
                    }
                }
            },
            onclick: move |_| {
                // A kept video opens in the player straight away, without
                // waiting for the watch page to ask the server about it.
                if let Some(video) = open_video.clone() {
                    app_state.record_history(&video.id);
                    app_state.play(video);
                }
                spawn(animated_navigate(Route::VideoDetail { id: open_id.clone() }));
            },
            if downloading {
                Progress {
                    value: entry.progress * 100.0,
                    label: "Download progress",
                }
            }
        }
    }
}

/// The watch page's Download button, and the sheet it opens: qualities to
/// pick from, or what is kept and a way to remove it.
#[component]
pub fn DownloadButton(video: ReadSignal<Video>) -> Element {
    let app_state = use_context::<AppState>();
    let mut open = use_signal(|| false);
    let entry = use_memo(move || {
        let id = video.read().id.clone();
        app_state
            .downloads
            .read()
            .iter()
            .find(|entry| entry.video_id == id)
            .cloned()
    });
    let start = use_callback(move |quality: DownloadQuality| {
        open.set(false);
        let video = video.peek().clone();
        spawn(async move {
            app_state.show_toast(
                format!("Preparing {}…", quality.label().to_lowercase()),
                Color::Neutral,
            );
            if let Err(error) = start_download(app_state, video, quality).await {
                app_state.show_toast(format!("Could not download: {error}"), Color::Danger);
            }
        });
    });
    let remove = use_callback(move |_: ()| {
        open.set(false);
        let id = video.peek().id.clone();
        let downloading = entry
            .peek()
            .as_ref()
            .is_some_and(|entry| entry.state == DownloadState::Downloading);
        spawn(async move {
            remove_download(&id).await;
            app_state.show_toast(
                if downloading {
                    "Download cancelled"
                } else {
                    "Download removed"
                },
                Color::Neutral,
            );
        });
    });

    // A live stream has no end to keep.
    if video.read().is_live {
        return rsx! {};
    }
    let current = entry();
    let state = current.as_ref().map(|entry| entry.state);
    let (icon, label) = match (state, current.as_ref()) {
        (Some(DownloadState::Done), _) => {
            (rsx! { CircleCheck { size: 17 } }, "Downloaded".to_string())
        }
        (Some(DownloadState::Downloading), Some(entry)) => (
            rsx! { Download { size: 17 } },
            format!("{}%", (entry.progress * 100.0).floor() as u32),
        ),
        _ => (rsx! { Download { size: 17 } }, "Download".to_string()),
    };

    rsx! {
        Button {
            fill: ButtonFill::Outline,
            color: if state == Some(DownloadState::Done) { Color::Success } else { Color::Neutral },
            size: ButtonSize::Sm,
            start: icon,
            onclick: move |_| open.set(true),
            "{label}"
        }

        BottomSheet { open, title: "Download", class: "player-sheet", max_height: "74dvh",
            match current {
                Some(entry) if entry.state != DownloadState::Failed => rsx! {
                    Stack { gap: Space::Md,
                        Text { tone: TextTone::Secondary, "{entry.summary()}" }
                        if entry.state == DownloadState::Downloading {
                            Progress { value: entry.progress * 100.0, label: "Download progress" }
                        } else {
                            Text { tone: TextTone::Secondary, "Plays from this device, with or without a connection." }
                        }
                        Button {
                            expand: ButtonExpand::Block,
                            fill: ButtonFill::Outline,
                            color: Color::Danger,
                            start: rsx! { Trash2 { size: 17 } },
                            onclick: move |_| remove(()),
                            if entry.state == DownloadState::Downloading { "Cancel download" } else { "Remove download" }
                        }
                        Button {
                            expand: ButtonExpand::Block,
                            fill: ButtonFill::Clear,
                            onclick: move |_| {
                                open.set(false);
                                spawn(animated_navigate(Route::Downloads {}));
                            },
                            "All downloads"
                        }
                    }
                },
                failed => rsx! {
                    Stack { gap: Space::Sm,
                        if let Some(failed) = failed {
                            Text { color: Color::Danger, "{failed.summary()}" }
                        }
                        List { lines: ListLines::Inset,
                            for quality in DownloadQuality::ALL {
                                Item {
                                    key: "{quality.label()}",
                                    label: quality.label(),
                                    description: quality.description(),
                                    detail: ItemDetail::Show,
                                    onclick: move |_| start(quality),
                                }
                            }
                        }
                    }
                },
            }
        }
    }
}
