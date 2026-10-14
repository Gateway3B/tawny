use crate::{
    api::{get_channel_details, get_channel_media_page, get_videos},
    app::Route,
    models::{
        Channel, ChannelDetails, ChannelMediaTab, FeedFilter, SubscriptionContent, Video,
        compact_count_value, queued_playlist_id,
    },
    state::AppState,
};
use dioxus::prelude::*;
use dioxus_icons::lucide::{Check, History, ListVideo, Play, Trash2, Tv};
use g3_route_transitions::{ROUTE_TRANSITION_OVERLAY_REGION_CLASS, animated_navigate};
use g3_ui::{
    Avatar, AvatarSize, BottomSheet, Button, ButtonFill, ButtonSize, Card, Chip, Color, Content,
    EmptyState, InfiniteScroll, SegmentButton, SegmentGroup, Shelf, Space, Spinner, Stack,
    StackAlign, Text, TextTone, TextVariant,
};

use super::{FeedFilterSegments, PageHeader, VideoGrid};
use g3_cache::{update_cached, use_cached};

/// How many history entries the page lays out before asking for more.
const HISTORY_PAGE_SIZE: usize = 24;

/// Queue and History are sheets, and the sheet is the whole page - its own
/// header included - so the header rides up with the body it belongs to.
#[component]
fn AuxiliarySheet(title: String, children: Element) -> Element {
    rsx! {
        div { class: "auxiliary-cover {ROUTE_TRANSITION_OVERLAY_REGION_CLASS}",
            PageHeader { title, back_to: Route::Feed {} }
            Content { {children} }
        }
    }
}

#[component]
pub fn QueuePage() -> Element {
    let app_state = use_context::<AppState>();
    let queued_ids = app_state.with_viewer(|viewer| {
        viewer
            .queue
            .iter()
            .filter(|entry| queued_playlist_id(entry).is_none())
            .cloned()
            .collect::<Vec<_>>()
    });
    // Keyed by the ids, so a changed queue is a new read.
    let queued_count = queued_ids.len();
    let queued = use_cached(get_videos, (queued_ids,));
    let videos = match &*queued.read() {
        Some(Ok(videos)) => videos.clone(),
        _ => Vec::new(),
    };
    // The playlist being run is one entry in the queue, not a copy of its
    // contents, so it gets a card of its own rather than fifty. Shown above
    // the loose videos because that is where `start_playlist_run` puts it:
    // pressing play on a playlist means now, not after last week's leftovers.
    let running = app_state.playlist_run_status();
    // What pressing play actually starts, resolved exactly the way autoplay
    // resolves it: a playlist at the head of the queue plays its first shown
    // entry, not the first loose video behind it.
    let first_id = app_state.next_in_run("");
    let is_empty = queued_count == 0 && running.is_none();

    rsx! {
        AuxiliarySheet { title: "Queue",
            if is_empty {
                EmptyState {
                    title: "Your queue is empty",
                    icon: rsx! { ListVideo { size: 40 } },
                    "Use a video menu to play next or add something to the queue."
                }
            } else {
                Stack { gap: Space::Md,
                    Stack { horizontal: true, gap: Space::Sm, align: StackAlign::Center,
                        Text { variant: TextVariant::Label, class: "mr-auto", "{queued_count} queued" }
                        Button {
                            start: rsx! { Play { size: 17 } },
                            onclick: move |_| {
                                if let Some(id) = &first_id {
                                    app_state.record_history(id);
                                    spawn(animated_navigate(Route::VideoDetail { id: id.clone() }));
                                }
                            },
                            "Play all"
                        }
                        Button {
                            fill: ButtonFill::Clear,
                            color: Color::Neutral,
                            start: rsx! { Trash2 { size: 17 } },
                            onclick: move |_| {
                                app_state.clear_queue();
                                app_state.show_toast("Queue cleared", Color::Neutral);
                            },
                            "Clear"
                        }
                    }
                    if let Some(run) = running {
                        {
                            let playlist_id = run.playlist_id.clone();
                            let playlist_name = run.name.clone();
                            let stop_name = run.name.clone();
                            let total = run.total;
                            let remaining = run.remaining;
                            let order_label = run.view.sort.label(run.view.descending);
                            let filters = run.view.filter_labels();
                            let filter_text = if filters.is_empty() {
                                "no filters".to_string()
                            } else {
                                filters.join(", ")
                            };
                            let left_text = if remaining == 1 { "1 left".to_string() } else { format!("{remaining} left") };
                            // A playlist in the queue is a rule rather than a
                            // list: each time a video ends it looks at the
                            // playlist as its page is arranged *then*. So this
                            // says where the run is and which arrangement it
                            // is following.
                            let now = match (run.position, run.current.as_ref()) {
                                (Some(position), Some(current)) => {
                                    format!("Now playing {position} of {total}: {}", current.title)
                                }
                                _ => format!("Not started · {remaining} of {total} to play"),
                            };
                            // The count leads so an ellipsis on a long title
                            // never swallows it.
                            let next = match (run.up_next.as_ref(), run.position.is_some()) {
                                (Some(next), true) => Some(format!("{left_text} · Up next: {}", next.title)),
                                (Some(next), false) => Some(format!("Starts with: {}", next.title)),
                                (None, true) => Some("Last video in this order".to_string()),
                                (None, false) => None,
                            };
                            rsx! {
                                Card {
                                    title: playlist_name,
                                    subtitle: "Order: {order_label} · {filter_text}",
                                    start: rsx! { ListVideo { size: 22 } },
                                    onclick: move |_| {
                                        spawn(animated_navigate(Route::PlaylistDetail { id: playlist_id.clone() }));
                                    },
                                    end: rsx! {
                                        Button {
                                            fill: ButtonFill::Clear,
                                            size: ButtonSize::Sm,
                                            aria_label: "Stop playing this playlist",
                                            onclick: move |_| {
                                                if app_state.clear_playlist_run() {
                                                    app_state.show_toast(format!("Stopped playing {stop_name}"), Color::Neutral);
                                                }
                                            },
                                            Trash2 { size: 16 }
                                        }
                                    },
                                    Stack { gap: Space::Xs,
                                        Text { class: "line-clamp-1", "{now}" }
                                        if let Some(next) = next {
                                            Text { tone: TextTone::Secondary, class: "line-clamp-1", "{next}" }
                                        }
                                        Text { variant: TextVariant::Caption,
                                            "Follows the playlist page as it is now. Change the order or filters there and the next video is picked from the new arrangement, continuing after the one playing. If the playing video no longer matches the filters, the run starts again from the top."
                                        }
                                    }
                                }
                            }
                        }
                    }
                    VideoGrid { videos }
                }
            }
        }
    }
}

#[component]
pub fn HistoryPage() -> Element {
    let app_state = use_context::<AppState>();
    let mut visible_count = use_signal(|| HISTORY_PAGE_SIZE);
    // At most `HISTORY_LIMIT` entries, fetched together and laid out a page
    // at a time.
    let history_ids = app_state.with_viewer(|viewer| {
        viewer
            .history
            .iter()
            .map(|entry| entry.video_id.clone())
            .collect::<Vec<_>>()
    });
    let history = use_cached(get_videos, (history_ids,));
    let (videos, total) = match &*history.read() {
        Some(Ok(found)) => (
            found
                .iter()
                .take(visible_count())
                .cloned()
                .collect::<Vec<_>>(),
            found.len(),
        ),
        _ => (Vec::new(), 0),
    };
    let remaining = total.saturating_sub(videos.len());

    rsx! {
        AuxiliarySheet { title: "History",
            Stack { gap: Space::Md,
                Stack { horizontal: true, gap: Space::Sm, align: StackAlign::Center,
                    History { size: 18 }
                    Text { variant: TextVariant::Label, "{total} recent" }
                    Text { tone: TextTone::Secondary, class: "mr-auto", "Most recent first" }
                    Button {
                        fill: ButtonFill::Clear,
                        color: Color::Neutral,
                        disabled: videos.is_empty(),
                        start: rsx! { Trash2 { size: 17 } },
                        onclick: move |_| {
                            app_state.clear_history();
                            app_state.show_toast("History cleared", Color::Neutral);
                        },
                        "Clear"
                    }
                }
                VideoGrid { videos, empty_message: "Videos you open will appear here.".to_string() }
                InfiniteScroll {
                    loading: false,
                    complete: remaining == 0,
                    on_load: move |_| {
                        if remaining > 0 {
                            visible_count += HISTORY_PAGE_SIZE;
                        }
                    },
                }
            }
        }
    }
}

/// Who the channel is, and the viewer's relationship to it. Its own component
/// so its hooks run on every render, not only once the channel has loaded.
#[component]
fn ChannelHero(channel: ReadSignal<Channel>) -> Element {
    let app_state = use_context::<AppState>();
    let mut description_open = use_signal(|| false);
    // The viewer's stored preference for this channel.
    let stored = use_memo(move || {
        let channel = channel.read();
        app_state
            .with_viewer(|viewer| {
                viewer
                    .follows(&channel.id)
                    .map(|followed| followed.subscription_content)
            })
            .unwrap_or(channel.subscription_content)
    });
    // Mirrors it for the control, which writes the viewer's pick at once.
    let mut content = use_signal(|| *stored.peek());
    use_effect(move || content.set(stored()));
    let channel = channel();
    let toggle_channel = channel.clone();
    let content_channel_id = channel.id.clone();
    // The viewer is the truth about follows: it shows a change at once.
    let followed = app_state.with_viewer(|viewer| viewer.follows(&channel.id).cloned());
    let is_subscribed = followed.is_some();
    let handle = channel.handle.trim();
    let subscribers = channel.subscriber_count.trim();
    let meta = match (handle.is_empty(), subscribers.is_empty()) {
        (false, false) => format!("{handle} · {subscribers} subscribers"),
        (false, true) => handle.to_string(),
        (true, false) => format!("{subscribers} subscribers"),
        (true, true) => String::new(),
    };

    rsx! {
        Card {
            media: channel.banner_url.clone().map(|banner_url| rsx! {
                div {
                    class: "aspect-[6/1] bg-cover bg-center",
                    style: "background-image: url('{banner_url}')",
                }
            }),
            Stack { gap: Space::Md,
                Stack { horizontal: true, gap: Space::Md, align: StackAlign::Center,
                    Avatar { name: channel.name.clone(), src: channel.avatar_url.clone(), size: AvatarSize::Lg }
                    Stack { gap: Space::Xs, class: "min-w-0",
                        Text { variant: TextVariant::Title, class: "line-clamp-2", "{channel.name}" }
                        if !meta.is_empty() {
                            Text { tone: TextTone::Secondary, "{meta}" }
                        }
                    }
                }
                Stack { horizontal: true, gap: Space::Sm, wrap: true,
                    Button {
                        fill: if is_subscribed { ButtonFill::Outline } else { ButtonFill::Solid },
                        color: if is_subscribed { Color::Neutral } else { Color::Accent },
                        "aria-pressed": if is_subscribed { "true" } else { "false" },
                        start: is_subscribed.then(|| rsx! { Check { size: 16 } }),
                        onclick: move |_| {
                            let now_subscribed = app_state.toggle_subscription_for(&toggle_channel);
                            {
                                app_state.show_toast(
                                    if now_subscribed { "Subscribed" } else { "Unsubscribed" },
                                    Color::Neutral,
                                );
                            }
                        },
                        if is_subscribed { "Subscribed" } else { "Subscribe" }
                    }
                    // Behind a button: a long channel description pushed the
                    // videos off the first screen.
                    if !channel.description.is_empty() {
                        Button {
                            fill: ButtonFill::Outline,
                            color: Color::Neutral,
                            onclick: move |_| description_open.set(true),
                            "About"
                        }
                    }
                }
                // Only meaningful once subscribed: this narrows what reaches
                // the feed, it does not change the channel page.
                if is_subscribed {
                    SegmentGroup {
                        value: content,
                        label: "Feed shows",
                        onchange: move |picked: SubscriptionContent| {
                            if app_state.set_subscription_content(&content_channel_id, picked) {
                                app_state.show_toast(
                                    format!("Feed shows {}", picked.label().to_lowercase()),
                                    Color::Neutral,
                                );
                            }
                        },
                        SegmentButton { value: SubscriptionContent::All, "Both" }
                        SegmentButton { value: SubscriptionContent::Videos, "Videos" }
                        SegmentButton { value: SubscriptionContent::Shorts, "Shorts" }
                    }
                }
            }
        }
        BottomSheet { open: description_open, title: "About",
            Text { class: "whitespace-pre-line", "{channel.description}" }
        }
    }
}

/// How a channel page orders its uploads.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum ChannelSort {
    #[default]
    Newest,
    Oldest,
    Popular,
    Unpopular,
}

impl ChannelSort {
    const ALL: [Self; 2] = [Self::Newest, Self::Popular];

    fn picked(self, option: Self) -> Self {
        match (self, option) {
            (Self::Newest, Self::Newest) => Self::Oldest,
            (Self::Popular, Self::Popular) => Self::Unpopular,
            (_, option) => option,
        }
    }

    fn family(self) -> Self {
        match self {
            Self::Oldest => Self::Newest,
            Self::Unpopular => Self::Popular,
            other => other,
        }
    }

    fn upstream(self) -> &'static str {
        match self {
            Self::Newest => "newest",
            Self::Oldest => "oldest",
            Self::Popular => "popular",
            Self::Unpopular => "unpopular",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Newest => "Newest",
            Self::Oldest => "Oldest",
            Self::Popular => "Popular",
            Self::Unpopular => "Least popular",
        }
    }
}

/// A channel's uploads on `tab` (every tab together for `None`): the answer's
/// plus the extra pages loaded since, in `sort` order, each once. Server pages
/// already follow that order; uploads with no view count stay last either way.
fn channel_videos(
    details: &ChannelDetails,
    tab: Option<ChannelMediaTab>,
    sort: ChannelSort,
    extra_videos: &[Video],
    extra_shorts: &[Video],
    extra_live: &[Video],
) -> Vec<Video> {
    let lists: Vec<&[Video]> = match tab {
        Some(ChannelMediaTab::Videos) => vec![&details.videos.videos, extra_videos],
        Some(ChannelMediaTab::Shorts) => vec![&details.shorts.videos, extra_shorts],
        Some(ChannelMediaTab::Live) => vec![&details.live.videos, extra_live],
        None => vec![
            &details.videos.videos,
            &details.shorts.videos,
            &details.live.videos,
            extra_videos,
            extra_shorts,
            extra_live,
        ],
    };
    let mut videos = lists.concat();
    let mut seen_ids = std::collections::HashSet::new();
    videos.retain(|video| seen_ids.insert(video.id.clone()));
    let views = |video: &Video| compact_count_value(&video.view_count);
    match sort {
        ChannelSort::Newest => {
            videos.sort_by_cached_key(|video| std::cmp::Reverse(video.published_epoch()))
        }
        ChannelSort::Oldest => videos.sort_by_cached_key(|video| {
            let epoch = video.published_epoch();
            (epoch == 0, epoch)
        }),
        ChannelSort::Popular => videos.sort_by(|a, b| {
            let (a_views, b_views) = (views(a), views(b));
            match (a_views, b_views) {
                (Some(a), Some(b)) => b.total_cmp(&a),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            }
            .then_with(|| b.published_epoch().cmp(&a.published_epoch()))
        }),
        ChannelSort::Unpopular => videos.sort_by(|a, b| {
            match (views(a), views(b)) {
                (Some(a), Some(b)) => a.total_cmp(&b),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            }
            .then_with(|| b.published_epoch().cmp(&a.published_epoch()))
        }),
    }
    videos
}

/// The channel tab a feed filter picks; `None` is every tab together.
fn media_tab(filter: FeedFilter) -> Option<ChannelMediaTab> {
    match filter {
        FeedFilter::All => None,
        FeedFilter::Videos => Some(ChannelMediaTab::Videos),
        FeedFilter::Shorts => Some(ChannelMediaTab::Shorts),
        FeedFilter::Live => Some(ChannelMediaTab::Live),
    }
}

fn continuation_requests(
    selected: Option<ChannelMediaTab>,
    tokens: [Option<String>; 3],
) -> Vec<(ChannelMediaTab, String)> {
    [
        ChannelMediaTab::Videos,
        ChannelMediaTab::Shorts,
        ChannelMediaTab::Live,
    ]
    .into_iter()
    .zip(tokens)
    .filter_map(|(tab, token)| {
        selected
            .is_none_or(|selected| selected == tab)
            .then(|| token.map(|token| (tab, token)))
            .flatten()
    })
    .collect()
}

#[cfg(test)]
mod channel_tests {
    use super::*;

    fn video(id: &str, date: &str, views: &str) -> Video {
        Video {
            id: id.into(),
            title: id.into(),
            channel_id: "channel".into(),
            channel_name: "Channel".into(),
            thumbnail_url: String::new(),
            published_at: date.into(),
            duration_seconds: 60,
            view_count: views.into(),
            progress_seconds: 0,
            watched: false,
            is_live: false,
            is_short: false,
            audio_only: false,
            channel_avatar_url: None,
        }
    }

    #[test]
    fn tapping_sort_chips_reverses_the_active_family() {
        assert!(ChannelSort::Newest.picked(ChannelSort::Newest) == ChannelSort::Oldest);
        assert!(ChannelSort::Oldest.picked(ChannelSort::Newest) == ChannelSort::Newest);
        assert!(ChannelSort::Popular.picked(ChannelSort::Popular) == ChannelSort::Unpopular);
        assert!(ChannelSort::Unpopular.picked(ChannelSort::Popular) == ChannelSort::Popular);
        assert!(ChannelSort::Oldest.picked(ChannelSort::Popular) == ChannelSort::Popular);
    }

    #[test]
    fn all_uploads_continue_every_non_exhausted_tab() {
        let tokens = [Some("videos".into()), None, Some("live".into())];
        assert_eq!(
            continuation_requests(None, tokens.clone()),
            vec![
                (ChannelMediaTab::Videos, "videos".into()),
                (ChannelMediaTab::Live, "live".into()),
            ]
        );
        assert!(continuation_requests(Some(ChannelMediaTab::Shorts), tokens.clone()).is_empty());
        assert_eq!(
            continuation_requests(Some(ChannelMediaTab::Live), tokens),
            vec![(ChannelMediaTab::Live, "live".into()),]
        );
    }

    #[test]
    fn channel_orders_merge_pages_deduplicate_and_keep_unknown_counts_last() {
        let page = |tab, videos| crate::models::ChannelMediaPage {
            channel_id: "channel".into(),
            tab,
            videos,
            next_page: None,
            source: "Test".into(),
        };
        let details = ChannelDetails {
            channel: Channel {
                id: "channel".into(),
                name: "Channel".into(),
                handle: String::new(),
                avatar_url: None,
                subscriber_count: String::new(),
                subscribed: false,
                description: String::new(),
                banner_url: None,
                subscription_content: SubscriptionContent::All,
            },
            videos: page(
                ChannelMediaTab::Videos,
                vec![
                    video("a", "2026-01-01", "9.5K views"),
                    video("b", "2026-03-01", "1.2M views"),
                ],
            ),
            shorts: page(
                ChannelMediaTab::Shorts,
                vec![video("c", "2026-02-01", "88 views")],
            ),
            live: page(ChannelMediaTab::Live, vec![video("unknown", "", "")]),
            remote_available: true,
        };
        let extra = vec![
            video("a", "2026-01-01", "9.5K views"),
            video("d", "2025-01-01", "2K views"),
        ];
        let ids = |sort| {
            channel_videos(&details, None, sort, &extra, &[], &[])
                .into_iter()
                .map(|video| video.id)
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(ChannelSort::Newest), ["b", "c", "a", "d", "unknown"]);
        assert_eq!(ids(ChannelSort::Oldest), ["d", "a", "c", "b", "unknown"]);
        assert_eq!(ids(ChannelSort::Popular), ["b", "a", "d", "c", "unknown"]);
        assert_eq!(ids(ChannelSort::Unpopular), ["c", "d", "a", "b", "unknown"]);
    }
}

#[component]
pub fn ChannelDetail(id: String) -> Element {
    let app_state = use_context::<AppState>();

    // Owned by the header segmented control.
    let tab = app_state.channel_tab;
    let mut sort = use_signal(ChannelSort::default);
    let mut extra_videos = use_signal(Vec::<Video>::new);
    let mut extra_shorts = use_signal(Vec::<Video>::new);
    let mut extra_live = use_signal(Vec::<Video>::new);
    let mut videos_next = use_signal(|| None::<String>);
    let mut shorts_next = use_signal(|| None::<String>);
    let mut live_next = use_signal(|| None::<String>);
    let mut page_loading = use_signal(|| false);
    let mut previous_tab = use_signal(|| *tab.peek());
    use_effect(move || {
        let current = tab();
        if previous_tab() != current {
            previous_tab.set(current);
            spawn(async move {
                let mut eval = document::eval(
                    "document.querySelector('.g3-content-scroll')?.scrollTo({ top: 0, behavior: 'instant' }); dioxus.send(true);",
                );
                let _ = eval.recv::<bool>().await;
            });
        }
    });

    // Cached per channel: reopening one shows it at once and refreshes
    // behind it. The server falls back to what it has cached when YouTube
    // cannot be reached.
    let upstream_order = use_memo(move || sort().upstream().to_string());
    let details = use_cached(get_channel_details, (id.clone(), upstream_order()));
    let channel_metadata = use_memo(move || match &*details.read() {
        Some(Ok(details)) => Some(details.channel.clone()),
        _ => None,
    });
    use_effect(move || {
        if let Some(fresh) = channel_metadata() {
            update_cached(crate::api::get_viewer, (), move |viewer| {
                if let Some(channel) = viewer
                    .subscriptions
                    .iter_mut()
                    .find(|channel| channel.id == fresh.id)
                {
                    channel.subscriber_count = fresh.subscriber_count.clone();
                }
            });
        }
    });
    // Each answer restarts the extra pages from its own continuations.
    let tokens = use_memo(move || match &*details.read() {
        Some(Ok(details)) => Some((
            details.videos.next_page.clone(),
            details.shorts.next_page.clone(),
            details.live.next_page.clone(),
            upstream_order(),
        )),
        _ => None,
    });
    use_effect(move || {
        if let Some((videos, shorts, live, _)) = tokens() {
            extra_videos.set(Vec::new());
            extra_shorts.set(Vec::new());
            extra_live.set(Vec::new());
            videos_next.set(videos);
            shorts_next.set(shorts);
            live_next.set(live);
        }
    });
    // The tab's uploads, newest first: the answer's plus the extra pages.
    // Worked out when one of those or the tab changes, not on every render.
    let videos = use_memo(move || {
        let details = details.read();
        let Some(Ok(details)) = details.as_ref() else {
            return Vec::new();
        };
        channel_videos(
            details,
            media_tab(tab()),
            sort(),
            &extra_videos.read(),
            &extra_shorts.read(),
            &extra_live.read(),
        )
    });
    // Made once, reading the tab, token and channel when they run. Closures
    // rebuilt every render and passed as props (pull to refresh, the infinite
    // scroll) were re-pointed after the previous one was dropped, and a tab
    // change panicked in `Callback::__point_to`.
    // Pull to refresh: refetch, keeping the channel on screen meanwhile.
    let refresh = use_callback(move |_: ()| details.refresh());
    let load_more = use_callback(move |_: ()| {
        if *page_loading.peek() || details.pending() {
            return;
        }
        let selected_tab = media_tab(*tab.peek());
        let requests = continuation_requests(
            selected_tab,
            [
                videos_next.peek().clone(),
                shorts_next.peek().clone(),
                live_next.peek().clone(),
            ],
        );
        if requests.is_empty() {
            return;
        }
        let requested_order = upstream_order.peek().clone();
        let channel_id = match &*details.peek() {
            Some(Ok(details)) => details.channel.id.clone(),
            _ => return,
        };
        page_loading.set(true);
        spawn(async move {
            for (selected_tab, token) in requests {
                let kind = match selected_tab {
                    ChannelMediaTab::Shorts => "shorts",
                    ChannelMediaTab::Live => "live",
                    ChannelMediaTab::Videos => "videos",
                };
                let result =
                    get_channel_media_page(channel_id.clone(), kind.into(), token.clone()).await;
                // A sort change or refresh replaces the continuation this request belongs to.
                let current_token = match selected_tab {
                    ChannelMediaTab::Videos => videos_next.peek().clone(),
                    ChannelMediaTab::Shorts => shorts_next.peek().clone(),
                    ChannelMediaTab::Live => live_next.peek().clone(),
                };
                if *upstream_order.peek() != requested_order
                    || current_token.as_ref() != Some(&token)
                {
                    break;
                }
                match result {
                    Ok(page) => match selected_tab {
                        ChannelMediaTab::Videos => {
                            extra_videos.write().extend(page.videos);
                            videos_next.set(page.next_page);
                        }
                        ChannelMediaTab::Shorts => {
                            extra_shorts.write().extend(page.videos);
                            shorts_next.set(page.next_page);
                        }
                        ChannelMediaTab::Live => {
                            extra_live.write().extend(page.videos);
                            live_next.set(page.next_page);
                        }
                    },
                    Err(_) => app_state.show_toast("Could not load the next page", Color::Warning),
                }
            }
            page_loading.set(false);
        });
    });

    let channel = match &*details.read() {
        Some(Ok(details)) => Some(details.channel.clone()),
        _ => None,
    };
    let Some(channel) = channel else {
        let failed = matches!(&*details.read(), Some(Err(_)));
        // Still renders the bar, so a channel that fails to load keeps its back
        // affordance instead of stranding the viewer.
        return rsx! {
            PageHeader { title: String::new(), back_to: Route::Subscriptions {} }
            Content {
                if failed {
                    EmptyState {
                        title: "Channel unavailable",
                        color: Color::Danger,
                        icon: rsx! { Tv { size: 40 } },
                        "This channel could not be loaded from YouTube or the server's cache."
                    }
                } else {
                    Spinner { center: true }
                }
            }
        };
    };

    let selected_tab = media_tab(tab());
    let has_next_page = match selected_tab {
        Some(ChannelMediaTab::Videos) => videos_next.read().is_some(),
        Some(ChannelMediaTab::Shorts) => shorts_next.read().is_some(),
        Some(ChannelMediaTab::Live) => live_next.read().is_some(),
        None => {
            videos_next.read().is_some()
                || shorts_next.read().is_some()
                || live_next.read().is_some()
        }
    };

    rsx! {
        PageHeader {
            // Deliberately untitled: the page leads with the channel name and
            // avatar, and repeating it in the bar pushed the actions off the
            // edge on a long name.
            title: String::new(),
            back_to: Route::Subscriptions {},
            toolbar: rsx! { FeedFilterSegments { value: tab } },
        }
        // Pull down to refresh replaces the button that used to sit in the
        // toolbar next to the segments.
        Content { on_refresh: refresh, refreshing: details.pending(),
            Stack { gap: Space::Lg,
                ChannelHero { channel }
                Shelf { aria_label: "Sort by", gap: Space::Sm,
                    for option in ChannelSort::ALL {
                        Chip {
                            key: "{option.label()}",
                            selected: sort().family() == option,
                            onclick: move |_| { let current = *sort.peek(); sort.set(current.picked(option)); },
                            if sort().family() == option { "{sort().label()}" } else { "{option.label()}" }
                        }
                    }
                }
                VideoGrid {
                    videos,
                    empty_message: match selected_tab {
                        Some(ChannelMediaTab::Shorts) => "No Shorts were returned for this channel.".to_string(),
                        Some(ChannelMediaTab::Live) => "No livestreams were returned for this channel.".to_string(),
                        Some(ChannelMediaTab::Videos) => "No videos were returned for this channel.".to_string(),
                        None => "No uploads were returned for this channel.".to_string(),
                    }
                }
                InfiniteScroll {
                    loading: page_loading() || details.pending(),
                    complete: !has_next_page,
                    on_load: load_more,
                }
            }
        }
    }
}
