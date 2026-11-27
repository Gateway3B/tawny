//! The Downloads page and the watch page's Download button. The downloads
//! themselves are in `crate::downloads`.

use crate::{
    app::Route,
    downloads::{
        DownloadEntry, DownloadQuality, DownloadState, export_download, merged_entries,
        queue_downloads, remove_all_downloads, remove_download, size_label,
    },
    models::Video,
    state::AppState,
};
use dioxus::prelude::*;
use dioxus_icons::lucide::{CircleCheck, Download, FileOutput, RotateCw, Trash2, X};
use g3_route_transitions::animated_navigate;
use g3_ui::{
    Avatar, AvatarSize, Badge, BottomSheet, Button, ButtonExpand, ButtonFill, ButtonSize, Card,
    CardVariant, Color, ConfirmModal, Content, EmptyState, Grid, GridColumns, Img, Item,
    ItemDetail, List, ListLines, Progress, Space, Stack, StackAlign, Text, TextTone,
};

use super::{PageHeader, VideoGrid};

#[component]
pub fn Downloads() -> Element {
    let app_state = use_context::<AppState>();
    // Kept videos are ordinary video cards, as in a playlist: the same look,
    // and the same ⋮, whose Download row opens the sheet that exports or
    // removes them. The artwork kept with the download stands in for the
    // remote thumbnail, so the page has pictures with no connection.
    let kept = use_memo(move || {
        app_state
            .downloads
            .read()
            .iter()
            .filter(|entry| entry.state == DownloadState::Done)
            .filter_map(|entry| {
                let mut video = entry.video.clone()?;
                if let Some(thumbnail) = &entry.thumbnail {
                    video.thumbnail_url = thumbnail.clone();
                }
                Some(video)
            })
            .collect::<Vec<Video>>()
    });
    let mut remove_all_open = use_signal(|| false);
    // What the page holds, then what is waiting its turn: the queue is this
    // app's, not the page's, so it is shown from here.
    let entries = merged_entries(
        &app_state.downloads.read(),
        &app_state.download_queue.read(),
    );
    let kept_bytes = entries
        .iter()
        .filter(|entry| entry.state == DownloadState::Done)
        .map(|entry| entry.bytes)
        .sum::<u64>();
    let kept_count = entries
        .iter()
        .filter(|entry| entry.state == DownloadState::Done)
        .count();
    // Under way or failed, or kept without the video it was of: these need a
    // progress bar or a way to cancel, which a video card has no place for.
    let failed = entries
        .iter()
        .filter(|entry| entry.state == DownloadState::Failed)
        .filter_map(|entry| entry.video.clone())
        .collect::<Vec<Video>>();
    let failed_count = failed.len();
    let total = entries.len();
    let pending = entries
        .into_iter()
        .filter(|entry| entry.state != DownloadState::Done || entry.video.is_none())
        .collect::<Vec<_>>();

    rsx! {
        PageHeader { title: "Downloads", back_to: Route::Playlists {} }
        Content {
            if !(app_state.downloads_supported)() {
                EmptyState {
                    title: "Downloads are not available here",
                    icon: rsx! { Download { size: 40 } },
                    "This browser cannot keep videos offline. The Tawny app can."
                }
            } else if kept_count == 0 && pending.is_empty() {
                EmptyState {
                    title: "Nothing downloaded",
                    icon: rsx! { Download { size: 40 } },
                    "Use Download under a video, in its ⋮ menu, or with a swipe set to Download, to keep it on this device and watch it without a connection."
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
                    Stack { horizontal: true, gap: Space::Sm, align: StackAlign::Center, class: "flex-wrap",
                        if failed_count > 0 {
                            Button {
                                fill: ButtonFill::Outline,
                                size: ButtonSize::Sm,
                                start: rsx! { RotateCw { size: 15 } },
                                onclick: move |_| {
                                    let title = if failed.len() == 1 {
                                        "Retry 1 download".to_string()
                                    } else {
                                        format!("Retry {} downloads", failed.len())
                                    };
                                    app_state.open_bulk_download(title, failed.clone());
                                },
                                if failed_count == 1 { "Retry failed" } else { "Retry {failed_count} failed" }
                            }
                        }
                        Button {
                            fill: ButtonFill::Outline,
                            color: Color::Danger,
                            size: ButtonSize::Sm,
                            start: rsx! { Trash2 { size: 15 } },
                            onclick: move |_| remove_all_open.set(true),
                            "Remove all"
                        }
                    }
                    // The same grid and card shape as the kept videos below, so
                    // a row under way is as tall as its neighbours whatever its
                    // title or error says.
                    if !pending.is_empty() {
                        Grid { class: "video-grid relative content-start", columns: GridColumns::Fit(18.0),
                            for entry in pending {
                                DownloadRow { key: "{entry.video_id}", entry }
                            }
                        }
                    }
                    if !kept.read().is_empty() {
                        VideoGrid { videos: kept }
                    }
                }
            }
            ConfirmModal {
                open: remove_all_open,
                title: "Remove all downloads?",
                message: if total == 1 {
                    "1 download will be deleted from this device.".to_string()
                } else {
                    format!("{total} downloads, kept and under way, will be deleted from this device.")
                },
                confirm_label: "Remove all",
                destructive: true,
                on_confirm: move |_| {
                    remove_all_open.set(false);
                    spawn(async move {
                        remove_all_downloads(app_state).await;
                        app_state.show_toast("All downloads removed", Color::Neutral);
                    });
                },
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
    let avatar = entry
        .video
        .as_ref()
        .map(|video| video.channel_avatar_url.clone())
        .unwrap_or_default();
    let status = entry.summary();
    // Short enough for a badge on the picture; the line under the title says
    // the rest, and its tooltip the whole of a long error.
    let badge = match entry.state {
        DownloadState::Downloading => format!("{}%", (entry.progress * 100.0).floor() as u32),
        DownloadState::Queued => "Waiting".to_string(),
        DownloadState::Failed => "Failed".to_string(),
        DownloadState::Done => String::new(),
    };
    let downloading = entry.pending();
    let failed = entry.state == DownloadState::Failed;
    let open_video = entry.video.clone();
    let open_id = entry.video_id.clone();
    let remove_id = entry.video_id.clone();
    let remove_title = entry.title();
    let retry_video = entry.video.clone();
    let remove_label = if downloading {
        format!("Cancel downloading {remove_title}")
    } else {
        format!("Remove {remove_title}")
    };
    let remove = move || {
        let id = remove_id.clone();
        spawn(async move {
            remove_download(app_state, &id).await;
            app_state.show_toast(
                if downloading {
                    "Download cancelled"
                } else {
                    "Download removed"
                },
                Color::Neutral,
            );
        });
    };
    let desktop_remove = remove.clone();
    let desktop_remove_label = remove_label.clone();

    rsx! {
        div { class: "video-card-frame",
            Card {
                variant: CardVariant::Flat,
                class: "h-full [&_.g3-card-title]:text-[0.95rem]",
                title: entry.title(),
                media: rsx! {
                    div { class: "relative",
                        Img { src: artwork, alt: "", aspect_ratio: "16 / 9" }
                        if !badge.is_empty() {
                            div { class: "thumbnail-badges pointer-events-none absolute right-2 bottom-2 flex gap-1",
                                Badge { "{badge}" }
                            }
                        }
                        if downloading {
                            Progress {
                                class: "absolute inset-x-0 bottom-0",
                                value: entry.progress * 100.0,
                                max: 100.0,
                                label: "Download progress",
                            }
                        }
                        // Where a video card in a playlist has its own remove,
                        // and drawn the same, so the two pages read alike.
                        div {
                            class: "video-card-remove video-card-remove-touch absolute top-0 right-0 p-2",
                            onclick: move |event: MouseEvent| {
                                event.stop_propagation();
                                remove();
                            },
                            Button { color: Color::Neutral, aria_label: remove_label,
                                if downloading {
                                    X { size: 20 }
                                } else {
                                    Trash2 { size: 20 }
                                }
                            }
                        }
                    }
                },
                start: rsx! {
                    Avatar { name: channel.clone(), src: avatar, size: AvatarSize::Sm }
                },
                end: rsx! {
                    if failed && let Some(video) = retry_video.clone() {
                        Button {
                            fill: ButtonFill::Clear,
                            color: Color::Neutral,
                            size: ButtonSize::Sm,
                            aria_label: format!("Try downloading {remove_title} again"),
                            onclick: move |event: MouseEvent| {
                                event.stop_propagation();
                                app_state.open_download(video.clone());
                            },
                            RotateCw { size: 18 }
                        }
                    }
                },
                onclick: move |_| {
                    // A failed download is tapped to try it again: the sheet
                    // offers the qualities afresh, since the one that failed on a
                    // weak connection may be the wrong one to ask for twice.
                    if failed && let Some(video) = open_video.clone() {
                        app_state.open_download(video);
                        return;
                    }
                    // A kept video opens in the player straight away, without
                    // waiting for the watch page to ask the server about it.
                    if let Some(video) = open_video.clone() {
                        app_state.record_history(&video.id);
                        app_state.play(video);
                    }
                    spawn(animated_navigate(Route::VideoDetail { id: open_id.clone() }));
                },
                div { class: "video-card-metadata",
                    span { class: "video-card-metadata-line",
                        if channel.is_empty() { "\u{00a0}" } else { "{channel}" }
                    }
                    span { class: "video-card-metadata-line", title: "{status}", "{status}" }
                }
            }
            div { class: "video-card-remove video-card-remove-desktop",
                Button {
                    size: ButtonSize::Sm,
                    color: Color::Neutral,
                    aria_label: desktop_remove_label,
                    onclick: move |_| desktop_remove(),
                    if downloading {
                        X { size: 15 }
                    } else {
                        Trash2 { size: 15 }
                    }
                }
            }
        }
    }
}

/// Exports a kept video as a file for other apps, saying how it went.
fn spawn_export(app_state: AppState, entry: DownloadEntry) {
    spawn(async move {
        app_state.show_toast("Exporting…", Color::Neutral);
        match export_download(&entry).await {
            Ok(saved) => app_state.show_toast(saved, Color::Success),
            Err(error) => app_state.show_toast(format!("Could not export: {error}"), Color::Danger),
        }
    });
}

/// The watch page's Download button. It opens the shared [`DownloadSheet`].
#[component]
pub fn DownloadButton(video: ReadSignal<Video>) -> Element {
    let app_state = use_context::<AppState>();
    let entry = use_download_entry(use_memo(move || Some(video.read().id.clone())));

    // A live stream has no end to keep, and some browsers cannot keep anything.
    if video.read().is_live || !(app_state.downloads_supported)() {
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
        (Some(DownloadState::Queued), _) => (rsx! { Download { size: 17 } }, "Waiting".to_string()),
        _ => (rsx! { Download { size: 17 } }, "Download".to_string()),
    };

    rsx! {
        Button {
            fill: ButtonFill::Outline,
            color: if state == Some(DownloadState::Done) { Color::Success } else { Color::Neutral },
            size: ButtonSize::Sm,
            start: icon,
            onclick: move |_| app_state.open_download(video.peek().clone()),
            "{label}"
        }
    }
}

/// What this device holds of one video, if anything.
fn use_download_entry(video_id: Memo<Option<String>>) -> Memo<Option<DownloadEntry>> {
    let app_state = use_context::<AppState>();
    use_memo(move || {
        let id = video_id()?;
        let listed = app_state
            .downloads
            .read()
            .iter()
            .find(|entry| entry.video_id == id)
            .cloned();
        // Waiting outranks an earlier failure: the retry is what is happening.
        let queue = app_state.download_queue.read();
        let queued = queue.iter().find(|queued| queued.video.id == id).cloned();
        match queued {
            Some(queued) => merged_entries(listed.as_slice(), &[queued]).pop(),
            None => listed,
        }
    })
}

/// The download sheet, mounted once with the other app overlays: qualities to
/// pick from, or what is kept and a way to remove it. Opened by
/// [`AppState::open_download`].
#[component]
pub fn DownloadSheet() -> Element {
    let mut app_state = use_context::<AppState>();
    let video_id = use_memo(move || {
        app_state
            .download_video
            .read()
            .as_ref()
            .map(|video| video.id.clone())
    });
    let entry = use_download_entry(video_id);
    let start = use_callback(move |quality: DownloadQuality| {
        app_state.download_open.set(false);
        let Some(video) = app_state.download_video.peek().clone() else {
            return;
        };
        let ahead = merged_entries(
            &app_state.downloads.peek(),
            &app_state.download_queue.peek(),
        )
        .iter()
        .filter(|entry| entry.pending())
        .count();
        if queue_downloads(app_state, vec![video], quality) > 0 {
            let quality = quality.label().to_lowercase();
            app_state.show_toast(
                if ahead > 0 {
                    format!("Queued {quality}")
                } else {
                    format!("Preparing {quality}…")
                },
                Color::Neutral,
            );
        }
    });
    let remove = use_callback(move |_: ()| {
        app_state.download_open.set(false);
        let Some(id) = video_id.peek().clone() else {
            return;
        };
        let downloading = entry.peek().as_ref().is_some_and(DownloadEntry::pending);
        spawn(async move {
            remove_download(app_state, &id).await;
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
    let current = entry();

    rsx! {
        BottomSheet { open: app_state.download_open, title: "Download", class: "player-sheet", max_height: "74dvh",
            Stack { gap: Space::Md,
                if let Some(video) = &*app_state.download_video.read() {
                    Text { tone: TextTone::Secondary, class: "line-clamp-2", "{video.title}" }
                }
                match current {
                    Some(entry) if entry.state != DownloadState::Failed => rsx! {
                        Stack { gap: Space::Md,
                            Text { tone: TextTone::Secondary, "{entry.summary()}" }
                            if entry.state == DownloadState::Downloading {
                                Progress { value: entry.progress * 100.0, label: "Download progress" }
                            } else if entry.state == DownloadState::Queued {
                                Text { tone: TextTone::Secondary, "Starts when the downloads ahead of it finish." }
                            } else {
                                Text { tone: TextTone::Secondary, "Plays from this device, with or without a connection." }
                                Button {
                                    expand: ButtonExpand::Block,
                                    fill: ButtonFill::Outline,
                                    start: rsx! { FileOutput { size: 17 } },
                                    onclick: {
                                        let entry = entry.clone();
                                        move |_| {
                                            app_state.download_open.set(false);
                                            spawn_export(app_state, entry.clone());
                                        }
                                    },
                                    if entry.quality == "Audio only" { "Export audio" } else { "Export video" }
                                }
                            }
                            Button {
                                expand: ButtonExpand::Block,
                                fill: ButtonFill::Outline,
                                color: Color::Danger,
                                start: rsx! { Trash2 { size: 17 } },
                                onclick: move |_| remove(()),
                                if entry.pending() { "Cancel download" } else { "Remove download" }
                            }
                            Button {
                                expand: ButtonExpand::Block,
                                fill: ButtonFill::Clear,
                                onclick: move |_| {
                                    app_state.download_open.set(false);
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
}

/// One quality for several videos at once: a whole playlist, or every failed
/// download. Opened by [`AppState::open_bulk_download`]; they then download
/// two at a time.
#[component]
pub fn BulkDownloadSheet() -> Element {
    let mut app_state = use_context::<AppState>();
    let start = use_callback(move |quality: DownloadQuality| {
        app_state.bulk_download_open.set(false);
        let Some(bulk) = app_state.bulk_download.peek().clone() else {
            return;
        };
        let added = queue_downloads(app_state, bulk.videos, quality);
        app_state.show_toast(
            match added {
                0 => "Those videos are already downloaded or on their way".to_string(),
                1 => "Downloading 1 video".to_string(),
                count => format!("Downloading {count} videos, two at a time"),
            },
            if added == 0 {
                Color::Neutral
            } else {
                Color::Success
            },
        );
    });
    let (title, summary) = match &*app_state.bulk_download.read() {
        Some(bulk) => {
            let downloads = app_state.downloads.read();
            let kept = bulk
                .videos
                .iter()
                .filter(|video| {
                    downloads.iter().any(|entry| {
                        entry.video_id == video.id && entry.state == DownloadState::Done
                    })
                })
                .count();
            let count = bulk.videos.len();
            let videos = if count == 1 {
                "1 video".to_string()
            } else {
                format!("{count} videos")
            };
            let summary = if kept > 0 {
                format!("{videos}, {kept} already on this device")
            } else {
                videos
            };
            (bulk.title.clone(), summary)
        }
        None => (String::new(), String::new()),
    };

    rsx! {
        BottomSheet { open: app_state.bulk_download_open, title: "{title}", class: "player-sheet", max_height: "74dvh",
            Stack { gap: Space::Sm,
                Text { tone: TextTone::Secondary, "{summary}" }
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
        }
    }
}
