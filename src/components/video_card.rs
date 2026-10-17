use crate::{
    app::Route,
    models::{SwipeActionKind, Video},
    state::AppState,
};
use dioxus::prelude::*;
use dioxus_icons::lucide::{
    Check, EllipsisVertical, ListPlus, Play, Rows3, Share2, Trash2, Video as VideoIcon,
};
use g3_route_transitions::animated_navigate;
use g3_ui::{
    Avatar, AvatarSize, Badge, Button, ButtonFill, ButtonSize, Card, CardVariant, Color,
    EmptyState, Grid, GridColumns, Img, Progress, Skeleton, SkeletonShape, Space, SwipeAction,
    SwipeBehavior, SwipeItem, SwipeSide, SwipeState,
};

/// The icon for a swipe action. A swipe can be set to any of five things, so
/// drawing the playlist icon for all of them told the reader the wrong one
/// four times out of five. These are the same icons the video actions sheet
/// uses for the same actions.
fn swipe_icon(kind: SwipeActionKind, size: u32) -> Element {
    match kind {
        SwipeActionKind::AddToPlaylist => rsx! { ListPlus { size } },
        SwipeActionKind::AddToQueue => rsx! { Rows3 { size } },
        SwipeActionKind::PlayNext => rsx! { Play { size } },
        SwipeActionKind::Share => rsx! { Share2 { size } },
        SwipeActionKind::MarkWatched => rsx! { Check { size } },
    }
}

/// How many cards a grid draws before the route transition that brought its
/// page in has finished. Two screens of a phone, a row or two of a wide one.
const GRID_FIRST_SCREEN: usize = 8;

/// A grid of video cards.
///
/// `videos` is a signal so a caller holding a memo passes it without copying
/// the list, and the grid re-renders only when the list changes, not every
/// time its parent does. A caller with a plain `Vec` still writes
/// `VideoGrid { videos }`.
#[component]
pub fn VideoGrid(
    videos: ReadSignal<Vec<Video>>,
    empty_message: Option<String>,
    playlist_id: Option<String>,
    shorts_layout: Option<bool>,
) -> Element {
    let shorts = shorts_layout.unwrap_or(false);
    let empty_copy =
        empty_message.unwrap_or_else(|| "Your cached library will appear here.".to_string());
    // A page slides in only once it has rendered, and a card is the costliest
    // thing on any page, so a grid draws its first screenful and the rest
    // follows once the slide has finished. That puts every list-bearing page's
    // first frame at the same small price, cached or not.
    let settled = super::use_after_route_transition();
    let limit = if settled() {
        usize::MAX
    } else {
        GRID_FIRST_SCREEN
    };
    // Cards from RSS arrive without a length, and only the feed and playlists
    // asked for it, so everywhere else the badge stayed missing. The server
    // only answers for videos this account follows or saved.
    let candidates = use_memo(move || super::duration_candidates(videos.read().iter(), usize::MAX));
    super::use_duration_hydration(candidates);
    let videos = videos.read();
    rsx! {
        if videos.is_empty() {
            EmptyState {
                title: "Nothing here yet",
                icon: rsx! { VideoIcon { size: 40 } },
                "{empty_copy}"
            }
        } else {
            // Shorts are portrait, so more of them fit across.
            Grid {
                columns: GridColumns::Fit(if shorts { 10.0 } else { 18.0 }),
                gap: Space::Lg,
                for video in videos.iter().take(limit) {
                    VideoCard { key: "{video.id}", video: video.clone(), playlist_id: playlist_id.clone(), short: shorts }
                }
            }
        }
    }
}

/// Where a [`VideoGrid`] will be, laid out the same way, while the page works
/// out what goes in it.
#[component]
pub fn VideoGridSkeleton(count: usize, shorts_layout: Option<bool>) -> Element {
    let shorts = shorts_layout.unwrap_or(false);
    rsx! {
        Grid {
            columns: GridColumns::Fit(if shorts { 10.0 } else { 18.0 }),
            gap: Space::Lg,
            for index in 0..count {
                div { key: "{index}", class: "flex flex-col gap-2",
                    Skeleton { shape: SkeletonShape::Block, class: if shorts { "aspect-[9/16] rounded-lg" } else { "aspect-video rounded-lg" } }
                    Skeleton { width: "85%" }
                    Skeleton { width: "50%" }
                }
            }
        }
    }
}

#[component]
pub fn VideoCard(
    video: Video,
    /// Set when this card is one entry of a playlist. Opening it then hands the
    /// playlist to the queue as a run, so autoplay and the player's arrows walk
    /// the playlist rather than stopping at this one video. The card, and its
    /// menu, also offer to remove the video from it.
    playlist_id: Option<String>,
    /// Draw the thumbnail portrait, for Shorts.
    short: Option<bool>,
) -> Element {
    let mut app_state = use_context::<AppState>();
    let start_action = app_state.swipe_action_label(true);
    let end_action = app_state.swipe_action_label(false);
    let start_kind = app_state.swipe_action_kind(true);
    let end_kind = app_state.swipe_action_kind(false);
    let channel_avatar_url = video.channel_avatar_url.clone();
    let progress = video.progress_percent();
    let stats = video.stats_label();

    let open = {
        let video = video.clone();
        let playlist_id = playlist_id.clone();
        move |_| {
            // History is recorded by the watch page, not here. On the History
            // page, recording moves this card to the top, which rebuilds it and
            // cancels a navigation spawned from its scope.
            app_state.play(video.clone());
            if let Some(playlist_id) = &playlist_id {
                app_state.start_playlist_run(playlist_id);
            }
            spawn(animated_navigate(Route::VideoDetail {
                id: video.id.clone(),
            }));
        }
    };
    let open_channel = {
        let id = video.channel_id.clone();
        move |_| {
            spawn(animated_navigate(Route::ChannelDetail { id: id.clone() }));
        }
    };
    let open_menu = {
        let video = video.clone();
        let playlist_id = playlist_id.clone();
        move |_| {
            app_state.video_actions_video.set(Some(video.clone()));
            app_state.video_actions_playlist.set(playlist_id.clone());
            app_state.video_actions_open.set(true);
        }
    };
    let remove = playlist_id.clone().map(|playlist_id| {
        let video_id = video.id.clone();
        move |_| {
            if app_state.remove_from_playlist(&video_id, &playlist_id) {
                app_state.show_toast("Removed from playlist", Color::Neutral);
            }
        }
    });
    let desktop_remove = remove.clone();
    let start_video = video.clone();
    let end_video = video.clone();
    let desktop_start_video = video.clone();
    let desktop_end_video = video.clone();
    let swipe_video = video.clone();
    let short = short.unwrap_or(false);

    rsx! {
        // A swipe files the video into one of two playlists. On a mouse the
        // gesture gives way to full-height thumbnail edge actions. Those
        // desktop controls are separate from the swipe layer so they remain
        // real, clickable buttons while the touch actions stay hidden.
        div {
            class: if short {
                "video-card-frame video-card-short h-full rounded-[inherit]"
            } else {
                "video-card-frame h-full rounded-[inherit]"
            },
            SwipeItem {
                class: "video-card-swipe h-full rounded-[inherit]",
                start_behavior: SwipeBehavior::Activate,
                end_behavior: SwipeBehavior::Activate,
                mouse_swipe: false,
                start_actions: rsx! {
                    SwipeAction {
                        color: Color::Accent,
                        aria_label: "{start_action}",
                        onclick: move |_| app_state.run_swipe_action(&start_video, true),
                        {swipe_icon(start_kind, 26)}
                    }
                },
                end_actions: rsx! {
                    SwipeAction {
                        aria_label: "{end_action}",
                        onclick: move |_| app_state.run_swipe_action(&end_video, false),
                        {swipe_icon(end_kind, 26)}
                    }
                },
                on_activate: move |swipe: SwipeState| {
                    app_state.run_swipe_action(&swipe_video, swipe.side == SwipeSide::Start);
                },
                Card {
                    variant: CardVariant::Flat,
                    class: "h-full [&_.g3-card-title]:text-[0.95rem]",
                    title: video.title.clone(),
                    onclick: open,
                    media: rsx! {
                        div { class: "relative",
                            Img {
                                src: video.thumbnail_url.clone(),
                                alt: "",
                                aspect_ratio: if short { "9 / 16" } else { "16 / 9" },
                            }
                        // Listings that come from a flat playlist carry no
                        // runtime. No badge is honest; "0:00" is not.
                        div { class: "thumbnail-badges pointer-events-none absolute right-2 bottom-2 flex gap-1",
                            if video.watched {
                                Badge { Check { size: 12 } "Watched" }
                            }
                            if video.is_live {
                                Badge { color: Color::Danger, "LIVE" }
                            } else if video.duration_seconds > 0 {
                                Badge { "{video.duration_label()}" }
                            }
                        }
                        if progress > 0.0 {
                            Progress {
                                class: "absolute inset-x-0 bottom-0",
                                value: progress,
                                max: 100.0,
                                label: "Watched {progress:.0}%",
                            }
                        }
                            // Rides along with the card as it is swiped.
                            if let Some(remove) = remove {
                                div { class: "video-card-remove video-card-remove-touch absolute top-2 right-2",
                                    Button {
                                        size: ButtonSize::Sm,
                                        color: Color::Neutral,
                                        aria_label: "Remove from playlist",
                                        onclick: remove,
                                        Trash2 { size: 15 }
                                    }
                                }
                            }
                        }
                    },
                    start: rsx! {
                        button {
                            r#type: "button",
                            class: "rounded-full",
                            aria_label: "Open {video.channel_name}",
                            onclick: open_channel,
                            Avatar {
                                name: video.channel_name.clone(),
                                src: channel_avatar_url,
                                size: AvatarSize::Sm,
                            }
                        }
                    },
                    end: rsx! {
                        Button {
                            fill: ButtonFill::Clear,
                            size: ButtonSize::Sm,
                            aria_label: "Video actions",
                            onclick: open_menu,
                            EllipsisVertical { size: 20 }
                        }
                    },
                    div { class: "video-card-metadata",
                        span { class: "video-card-metadata-line",
                            if video.channel_name.is_empty() { "\u{00a0}" } else { "{video.channel_name}" }
                        }
                        span { class: "video-card-metadata-line", "{stats}" }
                    }
                }
            }
            button {
                r#type: "button",
                class: "video-card-edge-action video-card-edge-action-start",
                aria_label: "{start_action}",
                onclick: move |_| app_state.run_swipe_action(&desktop_start_video, true),
                {swipe_icon(start_kind, 24)}
            }
            button {
                r#type: "button",
                class: "video-card-edge-action video-card-edge-action-end",
                aria_label: "{end_action}",
                onclick: move |_| app_state.run_swipe_action(&desktop_end_video, false),
                {swipe_icon(end_kind, 24)}
            }
            // On a mouse the end edge action covers the thumbnail's right
            // side, and the swipe layer underneath can't be lifted over it,
            // so this copy sits beside the edge actions, above them.
            if let Some(remove) = desktop_remove {
                div { class: "video-card-remove video-card-remove-desktop",
                    Button {
                        size: ButtonSize::Sm,
                        color: Color::Neutral,
                        aria_label: "Remove from playlist",
                        onclick: remove,
                        Trash2 { size: 15 }
                    }
                }
            }
        }
    }
}
