use crate::{
    api::{get_feed_page, refresh_subscription_feed},
    models::{DurationFilter, FeedFilter, FeedGroup, FeedQuery},
    state::AppState,
};
use dioxus::prelude::*;
use g3_cache::{invalidate_cached, use_cached};
use g3_ui::{
    Chip, Color, Content, Divider, DividerOrientation, InfiniteScroll, SegmentButton, SegmentGroup,
    Shelf, ShellSize, Space, Stack, Text, TextTone,
};

use super::{PageHeader, VideoGrid, VideoGridSkeleton, use_duration_hydration};

/// The segmented control that picks what kind of upload a list shows. Shared
/// by the feed and a channel page.
#[component]
pub fn FeedFilterSegments(value: Signal<FeedFilter>) -> Element {
    rsx! {
        SegmentGroup { value, aria_label: "Show",
            for filter in FeedFilter::ALL {
                SegmentButton { key: "{filter.label()}", value: filter, "{filter.label()}" }
            }
        }
    }
}

#[component]
pub fn Feed() -> Element {
    let mut app_state = use_context::<AppState>();
    let mut selected_group = app_state.feed_group;
    let mut selected_duration = app_state.feed_duration;
    // How many pages are on screen. Each is its own cached read, so asking
    // for the next one appends it without redrawing the ones above.
    let mut pages = app_state.feed_pages;
    // Whether the last page on screen said there is more.
    let has_more = use_signal(|| false);
    let groups =
        use_memo(move || app_state.with_viewer(|viewer| viewer.subscription_groups.clone()));
    // The question every page asks. A memo, so the pages share it rather than
    // each taking a copy on every render, and so a change is one event.
    let query = use_memo(move || {
        let settings = app_state.settings.read();
        FeedQuery {
            group: selected_group(),
            kind: (app_state.feed_filter)(),
            duration: selected_duration(),
            hide_watched: settings.hide_watched,
            thresholds: (&*settings).into(),
        }
    });
    // A different question starts again at one page. The first run is not a
    // different question: the filters and the pages are the ones this screen
    // was left with.
    let mut asked = use_signal(|| query.peek().clone());
    use_effect(move || {
        let current = query.read().clone();
        if *asked.peek() != current {
            asked.set(current);
            pages.set(1);
        }
    });

    let mut refresh = move || {
        app_state.syncing.set(true);
        spawn(async move {
            match refresh_subscription_feed().await {
                Ok(refresh) => {
                    invalidate_cached(get_feed_page);
                    let message = if refresh.imported == 0 {
                        "No new videos".to_string()
                    } else if refresh.failed_channels == 0 {
                        format!("Feed refreshed from {}", refresh.sources.join(" + "))
                    } else {
                        "Feed refreshed".to_string()
                    };
                    app_state.show_toast(message, Color::Success);
                }
                Err(_) => {
                    app_state.show_toast("Offline — showing your cached feed", Color::Warning)
                }
            }
            app_state.syncing.set(false);
        });
    };
    // Tapping the Feed tab again while here. A phone pulls down to refresh,
    // so the tap only goes back to the top; the wide layout has no pull, so
    // it refreshes as well, from the top where the new uploads land.
    let shell_size = use_hook(try_consume_context::<Signal<ShellSize>>);
    // Taps counted before this feed mounted were answered by an earlier one.
    let mut answered = use_signal(|| *app_state.feed_reselected.peek());
    use_effect(move || {
        let taps = (app_state.feed_reselected)();
        if taps == *answered.peek() {
            return;
        }
        answered.set(taps);
        spawn(async move {
            let mut eval = document::eval(
                "window[Symbol.for('tawny.scroll-memory')]?.release?.(); document.querySelector('.g3-content-scroll')?.scrollTo({ top: 0, behavior: 'smooth' }); dioxus.send(true);",
            );
            let _ = eval.recv::<bool>().await;
        });
        let wide = shell_size.is_some_and(|size| size.peek().is_wide());
        if wide && !*app_state.syncing.peek() {
            refresh();
        }
    });

    let load_next_page = move |_| {
        if has_more() {
            pages += 1;
        }
    };
    let page_count = pages();

    rsx! {
        PageHeader {
            toolbar: rsx! { FeedFilterSegments { value: app_state.feed_filter } },
        }

        Content {
            // Pull down to refresh, replacing the button that sat in the intro row.
            on_refresh: move |_| refresh(),
            refreshing: app_state.syncing(),
            Stack { gap: Space::Md,
                // Length and group filters share one strip rather than stacking
                // two thin bands above the grid.
                Shelf { aria_label: "Filters", gap: Space::Sm,
                    for duration in DurationFilter::ALL {
                        Chip {
                            key: "{duration.label()}",
                            selected: selected_duration() == Some(duration),
                            onclick: move |_| {
                                selected_duration.set(
                                    if selected_duration() == Some(duration) { None } else { Some(duration) },
                                );
                            },
                            "{duration.label()}"
                        }
                    }
                    Divider { orientation: DividerOrientation::Vertical }
                    Chip {
                        selected: selected_group() == FeedGroup::All,
                        onclick: move |_| selected_group.set(FeedGroup::All),
                        "All channels"
                    }
                    for group in groups.read().iter() {
                        {
                            let group_id = FeedGroup::Group(group.id.clone());
                            let selected = selected_group() == group_id;
                            rsx! {
                                Chip {
                                    key: "{group.id}",
                                    selected,
                                    onclick: move |_| selected_group.set(group_id.clone()),
                                    "{group.name}"
                                }
                            }
                        }
                    }
                    Chip {
                        selected: selected_group() == FeedGroup::Ungrouped,
                        onclick: move |_| selected_group.set(FeedGroup::Ungrouped),
                        "Ungrouped"
                    }
                }

                for page in 0..page_count {
                    FeedPageView {
                        key: "{page}",
                        query,
                        page,
                        last: page + 1 == page_count,
                        has_more,
                        empty_message: "Try another filter, or follow some channels from Explore.".to_string(),
                    }
                }

                InfiniteScroll {
                    loading: false,
                    complete: !has_more(),
                    on_load: load_next_page,
                }
            }
        }
    }
}

/// One page of the feed: its own cached read, so the pages above it stay put
/// while it loads.
///
/// Also the search page's list of followed uploads, with its own `empty_message`.
#[component]
pub(super) fn FeedPageView(
    query: ReadSignal<FeedQuery>,
    page: usize,
    last: ReadSignal<bool>,
    has_more: Signal<bool>,
    empty_message: String,
) -> Element {
    let feed = use_cached(get_feed_page, (query(), page));
    // Copied out when the page's answer changes, not on every render.
    let videos = use_memo(move || match &*feed.read() {
        Some(Ok(found)) => found.videos.clone(),
        _ => Vec::new(),
    });

    let candidates = use_memo(move || match &*feed.read() {
        Some(Ok(found)) => found.unknown_durations.clone(),
        _ => Vec::new(),
    });

    use_duration_hydration(candidates);
    // The last page decides whether there is another.
    use_effect(move || {
        let more = matches!(&*feed.read(), Some(Ok(found)) if found.has_more);
        if last() && *has_more.peek() != more {
            has_more.set(more);
        }
    });

    match &*feed.read() {
        None => rsx! { VideoGridSkeleton { count: if page == 0 { 8 } else { 4 } } },
        Some(Err(_)) if page == 0 => rsx! {
            VideoGrid {
                videos: Vec::new(),
                empty_message: "Could not load your feed. Pull down to try again.".to_string(),
            }
        },
        Some(Err(_)) => rsx! {},
        Some(Ok(found)) => {
            let without = found.without_duration;
            rsx! {
                if without > 0 {
                    Text { tone: TextTone::Secondary,
                        "{without} more "
                        if without == 1 { "video has" } else { "videos have" }
                        " no length yet, so a duration filter cannot speak for them. Visible cards are being filled in now."
                    }
                }
                if page == 0 || !found.videos.is_empty() {
                    VideoGrid { videos, empty_message: empty_message.clone() }
                }
            }
        }
    }
}
