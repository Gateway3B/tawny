use crate::state::{AppState, GroupPickerChannel, PlaylistSave};
use dioxus::prelude::*;
use dioxus_icons::lucide::{Check, Copy, ListPlus, Play, Plus, Rows3, Share2, Tags, Trash2};
use g3_ui::{
    BottomSheet, Button, ButtonExpand, ButtonFill, Color, Img, Input, Item, List, ListLines,
    ListVariant, Modal, Space, Stack, StackAlign, Text, TextTone, TextVariant, Toggle,
};

fn youtube_share_url(video_id: &str, with_timestamp: bool, timestamp_seconds: u64) -> String {
    let mut url = format!("https://www.youtube.com/watch?v={video_id}");
    if with_timestamp && timestamp_seconds > 0 {
        url.push_str(&format!("&t={timestamp_seconds}s"));
    }
    url
}

fn timestamp_label(seconds: u64) -> String {
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    let seconds = seconds % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

#[component]
pub fn AppOverlays() -> Element {
    let mut app_state = use_context::<AppState>();
    let mut create_playlist_open = use_signal(|| false);
    let mut new_playlist_name = use_signal(String::new);
    let mut new_playlist_video = use_signal(|| None::<crate::models::Video>);
    let mut create_group_open = use_signal(|| false);
    let mut new_group_name = use_signal(String::new);
    let mut new_group_channel = use_signal(|| None::<GroupPickerChannel>);
    #[cfg(any(
        target_arch = "wasm32",
        target_os = "android",
        target_os = "ios",
        target_os = "macos"
    ))]
    let mut plugins = use_context::<g3_native_plugins::NativePlugins>();
    let target = app_state.playlist_picker_video();
    let action_target = (app_state.video_actions_video)();
    let group_target = (app_state.group_picker_channel)();
    let picker_groups = if (app_state.group_picker_open)() {
        app_state.with_viewer(|viewer| viewer.subscription_groups.clone())
    } else {
        Vec::new()
    };
    let share_target = (app_state.share_video)();
    let share_url = share_target
        .as_ref()
        .map(|video| {
            youtube_share_url(
                &video.id,
                (app_state.share_with_timestamp)(),
                (app_state.share_timestamp_seconds)(),
            )
        })
        .unwrap_or_default();
    let share_action_url = share_url.clone();
    let share_text_target = share_target.clone();
    let copy_action_url = share_url.clone();

    rsx! {
        // As tall as its rows: the default cap left the last one or two
        // behind a scroll on a phone.
        BottomSheet { open: app_state.video_actions_open, aria_label: "Video actions",
            max_height: "calc(100dvh - 6rem)",
            if let Some(video) = &action_target {
                {
                    let play_next_id = video.id.clone();
                    let queue_id = video.id.clone();
                    let save_video = video.clone();
                    let watched_video = video.clone();
                    let share_video = video.clone();
                    let remove_video = video.clone();
                    let group_channel_id = video.channel_id.clone();
                    let group_channel_name = video.channel_name.clone();
                    // Groups only sort the feed, which only holds followed
                    // channels, so the row is offered for those alone.
                    let follows_channel = app_state.follows(&video.channel_id);
                    let channel_groups = app_state.group_names_for(&video.channel_id);
                    // Only offered when the sheet was opened from a playlist
                    // the video is still in.
                    let remove_from = (app_state.video_actions_playlist)().and_then(|playlist_id| {
                        app_state.with_viewer(|viewer| {
                            viewer
                                .playlists
                                .iter()
                                .find(|playlist| playlist.id == playlist_id && playlist.video_ids.contains(&video.id))
                                .map(|playlist| (playlist.id.clone(), playlist.name.clone()))
                        })
                    });
                    let is_queued = app_state.with_viewer(|viewer| viewer.queue.iter().any(|id| id == &video.id));
                    let is_watched = video.watched;
                    rsx! {
                        Stack { gap: Space::Md,
                            Stack { horizontal: true, gap: Space::Md, align: StackAlign::Center,
                                Img { src: video.thumbnail_url.clone(), alt: "", aspect_ratio: "16 / 9", class: "w-28 shrink-0 rounded-lg" }
                                Stack { gap: Space::Xs, class: "min-w-0",
                                    Text { variant: TextVariant::Label, class: "line-clamp-2", "{video.title}" }
                                    Text { tone: TextTone::Secondary, "{video.channel_name}" }
                                }
                            }
                            List { variant: ListVariant::Raised, lines: ListLines::Inset,
                                Item {
                                    start: rsx! { Play { size: 18 } },
                                    label: "Play next",
                                    onclick: move |_| {
                                        let message = app_state.add_to_queue(&play_next_id, true);
                                        app_state.video_actions_open.set(false);
                                        app_state.show_toast(message, Color::Success);
                                    },
                                }
                                Item {
                                    start: rsx! { Rows3 { size: 18 } },
                                    label: if is_queued { "Remove from queue" } else { "Add to queue" },
                                    onclick: move |_| {
                                        let message = if is_queued {
                                            app_state.remove_from_queue(&queue_id);
                                            "Removed from queue".to_string()
                                        } else {
                                            app_state.add_to_queue(&queue_id, false)
                                        };
                                        app_state.video_actions_open.set(false);
                                        app_state.show_toast(message, Color::Neutral);
                                    },
                                }
                                Item {
                                    start: rsx! { ListPlus { size: 18 } },
                                    label: "Save to playlist",
                                    onclick: move |_| {
                                        app_state.video_actions_open.set(false);
                                        app_state.playlist_picker_video.set(Some(save_video.clone()));
                                        app_state.playlist_picker_open.set(true);
                                    },
                                }
                                Item {
                                    start: rsx! { Check { size: 18 } },
                                    label: if is_watched { "Mark unwatched" } else { "Mark watched" },
                                    onclick: move |_| {
                                        app_state.mark_watched(&watched_video, !is_watched);
                                        app_state.video_actions_open.set(false);
                                        app_state.show_toast(
                                            if is_watched { "Marked unwatched" } else { "Marked watched" },
                                            Color::Neutral,
                                        );
                                    },
                                }
                                Item {
                                    start: rsx! { Share2 { size: 18 } },
                                    label: "Share",
                                    onclick: move |_| {
                                        app_state.video_actions_open.set(false);
                                        app_state.open_share(share_video.clone());
                                    },
                                }
                                if follows_channel {
                                    Item {
                                        start: rsx! { Tags { size: 18 } },
                                        label: "Channel groups",
                                        description: if channel_groups.is_empty() {
                                            "Not in a group".to_string()
                                        } else {
                                            channel_groups.join(", ")
                                        },
                                        onclick: move |_| {
                                            app_state.video_actions_open.set(false);
                                            app_state.open_group_picker(&group_channel_id, &group_channel_name);
                                        },
                                    }
                                }
                                if let Some((playlist_id, playlist_name)) = remove_from {
                                    Item {
                                        start: rsx! {
                                            span { class: "flex text-[var(--g3-color-danger)]", Trash2 { size: 18 } }
                                        },
                                        label: "Remove from playlist",
                                        aria_label: "Remove from {playlist_name}",
                                        onclick: move |_| {
                                            app_state.video_actions_open.set(false);
                                            app_state.remove_from_playlist(&remove_video, &playlist_id);
                                        },
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        BottomSheet { open: app_state.playlist_picker_open, title: "Save to playlist",
            Stack { gap: Space::Md,
                if let Some(video) = &target {
                    Text { tone: TextTone::Secondary, class: "line-clamp-2", "{video.title}" }
                }
                List { variant: ListVariant::Raised, lines: ListLines::Inset,
                    for playlist in app_state.with_viewer(|viewer| viewer.playlists.clone()) {
                        {
                            let picked = target.clone();
                            let video_id = target.as_ref().map(|video| video.id.clone()).unwrap_or_default();
                            let playlist_id = playlist.id.clone();
                            let playlist_name = playlist.name.clone();
                            let already_saved = playlist.video_ids.iter().any(|id| id == &video_id);
                            rsx! {
                                Item {
                                    key: "{playlist.id}",
                                    checked: already_saved,
                                    label: playlist.name,
                                    description: format!("{} videos", playlist.video_ids.len()),
                                    onclick: move |_| {
                                        if already_saved {
                                            if let Some(video) = picked.as_ref() {
                                                app_state.remove_from_playlist(video, &playlist_id);
                                            }
                                        } else if let Some(saved) = picked.as_ref().and_then(|video| app_state.add_to_playlist(video, &playlist_id)) {
                                            match saved {
                                                PlaylistSave::Saved(name) => {
                                                    app_state.show_toast(format!("Added to {name}"), Color::Success)
                                                }
                                                PlaylistSave::AlreadyThere(name) => {
                                                    app_state.show_toast(format!("Already in {name}"), Color::Warning)
                                                }
                                            }
                                        } else {
                                            app_state.show_toast(format!("Could not open {playlist_name}"), Color::Danger);
                                        }
                                        app_state.playlist_picker_open.set(false);
                                    },
                                }
                            }
                        }
                    }
                }
                Button {
                    fill: ButtonFill::Outline,
                    color: Color::Neutral,
                    expand: ButtonExpand::Block,
                    start: rsx! { Plus { size: 18 } },
                    onclick: move |_| {
                        new_playlist_name.set(String::new());
                        // The picker steps aside rather than staying up under
                        // the dialog, where the dialog was hidden by it. The
                        // video is kept here: the picker's own copy is not
                        // something to rely on once the sheet has closed.
                        new_playlist_video.set((app_state.playlist_picker_video)());
                        app_state.playlist_picker_open.set(false);
                        create_playlist_open.set(true);
                    },
                    "New playlist"
                }
            }
        }
        BottomSheet { open: app_state.group_picker_open, title: "Channel groups",
            Stack { gap: Space::Md,
                if let Some(channel) = &group_target {
                    Text { tone: TextTone::Secondary, class: "line-clamp-2", "{channel.name}" }
                }
                if picker_groups.is_empty() {
                    Text { tone: TextTone::Secondary, "No groups yet. A group filters your feed to just its channels." }
                } else {
                    List { variant: ListVariant::Raised, lines: ListLines::Inset,
                        for group in picker_groups {
                            {
                                let channel_id = group_target.as_ref().map(|channel| channel.id.clone()).unwrap_or_default();
                                let member = group.channel_ids.iter().any(|id| id == &channel_id);
                                let group_id = group.id.clone();
                                rsx! {
                                    Item {
                                        key: "{group.id}",
                                        checked: member,
                                        label: group.name,
                                        description: super::subscriptions::channel_count(group.channel_ids.len()),
                                        // Stays open: a channel can sit in several groups.
                                        onclick: move |_| app_state.set_channel_in_group(&group_id, &channel_id, !member),
                                    }
                                }
                            }
                        }
                    }
                }
                Stack { horizontal: true, gap: Space::Sm,
                    Button {
                        fill: ButtonFill::Outline,
                        color: Color::Neutral,
                        class: "flex-1",
                        start: rsx! { Plus { size: 18 } },
                        onclick: move |_| {
                            new_group_name.set(String::new());
                            // Steps aside for the dialog, as the playlist
                            // picker does, keeping its own copy of the channel.
                            new_group_channel.set((app_state.group_picker_channel)());
                            app_state.group_picker_open.set(false);
                            create_group_open.set(true);
                        },
                        "New group"
                    }
                    Button {
                        class: "flex-1",
                        onclick: move |_| app_state.group_picker_open.set(false),
                        "Done"
                    }
                }
            }
        }
        Modal {
            open: create_group_open,
            title: "New group",
            actions: rsx! {
                Button {
                    fill: ButtonFill::Clear,
                    color: Color::Neutral,
                    onclick: move |_| {
                        create_group_open.set(false);
                        app_state.group_picker_open.set(true);
                    },
                    "Cancel"
                }
                Button {
                    disabled: new_group_name().trim().is_empty(),
                    onclick: move |_| {
                        let name = new_group_name().trim().to_string();
                        if name.is_empty() { return; }
                        create_group_open.set(false);
                        let channel = new_group_channel.peek().clone();
                        let channel_ids = channel.iter().map(|channel| channel.id.clone()).collect();
                        app_state.create_subscription_group(name.clone(), channel_ids);
                        match channel {
                            Some(channel) => app_state.show_toast(format!("Added {} to {name}", channel.name), Color::Success),
                            None => app_state.show_toast(format!("Created {name}"), Color::Success),
                        }
                    },
                    "Create"
                }
            },
            Input { label: "Group name", value: new_group_name, placeholder: "Documentaries", autofocus: true }
        }
        Modal {
            open: create_playlist_open,
            title: "New playlist",
            actions: rsx! {
                Button {
                    fill: ButtonFill::Clear,
                    color: Color::Neutral,
                    onclick: move |_| create_playlist_open.set(false),
                    "Cancel"
                }
                Button {
                    disabled: new_playlist_name().trim().is_empty(),
                    onclick: move |_| {
                        let name = new_playlist_name().trim().to_string();
                        if name.is_empty() { return; }
                        create_playlist_open.set(false);
                        match new_playlist_video.peek().as_ref() {
                            Some(video) => {
                                app_state.create_playlist_with(name.clone(), video);
                                app_state.show_toast(format!("Created {name} and saved video"), Color::Success);
                            }
                            None => {
                                app_state.create_playlist(name.clone());
                                app_state.show_toast(format!("Created {name}"), Color::Success);
                            }
                        }
                    },
                    "Create"
                }
            },
            Input { label: "Playlist name", value: new_playlist_name, autofocus: true }
        }
        Modal {
            open: app_state.share_open,
            title: "Share video",
            actions: rsx! {
                Button {
                    fill: ButtonFill::Clear,
                    onclick: move |_| app_state.share_open.set(false),
                    "Cancel"
                }
                Button {
                    start: rsx! { Share2 { size: 17 } },
                    onclick: move |_| {
                        let link = share_action_url.clone();
                        let text = share_text_target
                            .as_ref()
                            .map(|video| format!("{}\n{}", video.title, link))
                            .unwrap_or(link);
                        cfg_if::cfg_if! {
                            if #[cfg(any(target_arch = "wasm32", target_os = "android", target_os = "ios", target_os = "macos"))] {
                                if let Err(error) = plugins.clipboard.write().share(text) {
                                    app_state.show_toast(error, Color::Warning);
                                }
                            } else {
                                let _ = text;
                                app_state.show_toast("Sharing is not available on this platform", Color::Warning);
                            }
                        }
                        app_state.share_open.set(false);
                    },
                    "Share"
                }
            },
            Stack { gap: Space::Md,
                if let Some(video) = &share_target {
                    Stack { gap: Space::Xs,
                        Text { variant: TextVariant::Label, "{video.title}" }
                        Text { tone: TextTone::Secondary, class: "break-all font-mono text-xs", "{share_url}" }
                    }
                }
                Toggle {
                    checked: app_state.share_with_timestamp,
                    label: "Start at {timestamp_label((app_state.share_timestamp_seconds)())}",
                    helper: "Add the current playback position to the link",
                }
                Button {
                    fill: ButtonFill::Outline,
                    color: Color::Neutral,
                    expand: ButtonExpand::Block,
                    start: rsx! { Copy { size: 17 } },
                    onclick: move |_| {
                        let link = copy_action_url.clone();
                        cfg_if::cfg_if! {
                            if #[cfg(any(target_arch = "wasm32", target_os = "android", target_os = "ios", target_os = "macos"))] {
                                match plugins.clipboard.write().copy_to_clipboard(link) {
                                    Ok(()) => app_state.show_toast("Link copied", Color::Success),
                                    Err(error) => app_state.show_toast(error, Color::Warning),
                                }
                            } else {
                                let _ = link;
                                app_state.show_toast("Copy is not available on this platform", Color::Warning);
                            }
                        }
                    },
                    "Copy link"
                }
            }
        }
    }
}
