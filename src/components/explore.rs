use crate::{
    api::{get_feed_page, get_recommendations, search_catalog, search_catalog_page},
    app::Route,
    models::{
        AppSettings, Channel, DiscoverList, ExploreFilter, FeedFilter, FeedGroup, FeedQuery, Video,
    },
    state::AppState,
};
use dioxus::prelude::*;
use g3_cache::use_cached;
use g3_route_transitions::animated_navigate;
use g3_ui::{
    Avatar, AvatarSize, Chip, Color, Content, InfiniteScroll, Searchbar, SegmentButton,
    SegmentGroup, Shelf, Space, Spinner, Stack, StackAlign, Text, TextTone,
};

use super::{PageHeader, VideoGrid, VideoGridSkeleton, feed::FeedPageView, use_duration_hydration};

/// How many matches a query lays out before the reader asks for more.
///
/// A one-letter query matches most of a real library. Laying all of it out
/// builds tens of thousands of cards in a single render, which takes the tab
/// down with it - the page has to grow with what the reader can see, not with
/// the size of the cache.
const RESULT_PAGE_SIZE: usize = 24;

#[component]
pub fn Explore() -> Element {
    let app_state = use_context::<AppState>();
    let search = app_state.explore_search;
    let explore_filter = app_state.explore_filter;
    let mut results = app_state.explore_results;
    let mut searching = use_signal(|| false);
    let mut visible_count = use_signal(|| RESULT_PAGE_SIZE);
    let needle = use_memo(move || search().trim().to_lowercase());
    // A new query or filter is a new list: start it at one page again.
    use_effect(move || {
        let _ = (needle(), explore_filter());
        visible_count.set(RESULT_PAGE_SIZE);
    });
    // With nothing typed, the page lists what YouTube recommended beside the
    // videos watched lately, or the newest unwatched uploads from followed
    // channels (`DiscoverLists`). Type-ahead filters the first page of each,
    // the same cached reads, before Enter asks the server.
    let suggestions = use_cached(
        get_feed_page,
        (following_query(&app_state.settings.read()), 0),
    );
    let recommendations = use_cached(get_recommendations, (0,));
    // What the page shows: the server's results for this query, or the
    // suggestions matched on the device. Worked out when one of those, the
    // filter or the page size changes, not on every render (a keystroke
    // re-renders the search box, not this).
    let shown = use_memo(move || {
        let filter = explore_filter();
        let page = visible_count();
        let results = results.read();
        if let Some(remote) = results
            .as_ref()
            .filter(|remote| remote.query.trim().eq_ignore_ascii_case(search().trim()))
        {
            return paged(
                remote.videos.iter().collect(),
                remote.channels.iter().collect(),
                filter,
                page,
            );
        }
        let suggestions = suggestions.read();
        let suggested: &[Video] = match &*suggestions {
            None => return Shown::loading(),
            Some(Ok(feed)) => &feed.videos,
            Some(Err(_)) => &[],
        };
        let recommendations = recommendations.read();
        let recommended: &[Video] = match &*recommendations {
            Some(Ok(found)) => &found.videos,
            _ => &[],
        };
        let needle = needle.read();
        // `DiscoverLists` has the page with nothing typed.
        if needle.is_empty() {
            return Shown::default();
        }
        app_state.with_viewer(|viewer| {
            let (videos, channels) =
                local_matches(recommended, suggested, &viewer.subscriptions, &needle);
            paged(videos, channels, filter, page)
        })
    });

    let filter = explore_filter();
    let search_value = search();
    // The next page of the server's results for what is typed, if there is one.
    let next_page = move || {
        results
            .peek()
            .as_ref()
            .filter(|remote| {
                remote
                    .query
                    .trim()
                    .eq_ignore_ascii_case(search.peek().trim())
            })
            .and_then(|remote| remote.next_page.clone())
    };
    let has_next_page = results.read().as_ref().is_some_and(|remote| {
        remote.next_page.is_some() && remote.query.trim().eq_ignore_ascii_case(search().trim())
    });

    let (loading, result_count, remaining) = {
        let shown = shown.read();
        (shown.loading, shown.result_count, shown.remaining)
    };
    let discovering = needle.read().is_empty() && filter != ExploreFilter::Channels;
    // Enter runs the remote search; typing filters the cache as it goes.
    let run_search = move |query: String| {
        let query = query.trim().to_string();
        if query.chars().count() < 2 || searching() {
            return;
        }
        searching.set(true);
        spawn(async move {
            match search_catalog(query, filter.query().to_string()).await {
                Ok(found) => {
                    if !found.remote_available {
                        app_state.show_toast(
                            "Search is unavailable right now. Showing saved results.",
                            Color::Warning,
                        );
                    }
                    results.set(Some(found));
                }
                Err(_) => {
                    app_state.show_toast("You're offline. Showing saved results.", Color::Warning)
                }
            }
            searching.set(false);
        });
    };
    // Made once and reading state when it runs, like the channel page's: the
    // infinite scroll calls it as the sentinel comes into view. It first grows
    // the laid-out page, and only asks the server when everything it has is
    // already on screen.
    let load_more = use_callback(move |_: ()| {
        if shown.peek().remaining > 0 {
            visible_count += RESULT_PAGE_SIZE;
            return;
        }
        if *searching.peek() {
            return;
        }
        let Some(token) = next_page() else {
            return;
        };
        let filter = *explore_filter.peek();
        let query = search.peek().trim().to_string();
        searching.set(true);
        spawn(async move {
            match search_catalog_page(query, filter.query().to_string(), token).await {
                Ok(page) => {
                    results.with_mut(|current| {
                        if let Some(current) = current.as_mut() {
                            for video in page.videos {
                                if !current.videos.iter().any(|item| item.id == video.id) {
                                    current.videos.push(video);
                                }
                            }
                            for channel in page.channels {
                                if !current.channels.iter().any(|item| item.id == channel.id) {
                                    current.channels.push(channel);
                                }
                            }
                            current.next_page = page.next_page;
                        }
                    });
                }
                Err(_) => {
                    app_state.show_toast("Could not load more search results", Color::Warning)
                }
            }
            searching.set(false);
        });
    });

    rsx! {
        PageHeader {
            toolbar: rsx! {
                SegmentGroup { value: app_state.explore_filter, aria_label: "Search for",
                    for option in ExploreFilter::ALL {
                        SegmentButton { key: "{option.label()}", value: option, "{option.label()}" }
                    }
                }
            },
        }

        Content {
            Stack { gap: Space::Md,
                Stack { horizontal: true, gap: Space::Sm, align: StackAlign::Center,
                    Searchbar {
                        class: "min-w-0 flex-1",
                        value: search,
                        placeholder: "Search videos and channels",
                        debounce_ms: 0,
                        on_submit: run_search,
                    }
                    if searching() {
                        Spinner {}
                    }
                }
                if !needle.read().is_empty() {
                    Text { tone: TextTone::Secondary,
                        "{result_count} results for “{search_value}”. Press Enter to search online."
                    }
                }
                if !shown.read().channels.is_empty() {
                    Shelf { title: "Channels", gap: Space::Sm,
                        for channel in shown.read().channels.iter() {
                            {
                                let id = channel.id.clone();
                                rsx! {
                                    Chip {
                                        key: "{channel.id}",
                                        start: rsx! {
                                            Avatar { name: channel.name.clone(), src: channel.avatar_url.clone(), size: AvatarSize::Sm }
                                        },
                                        onclick: move |_| { spawn(animated_navigate(Route::ChannelDetail { id: id.clone() })); },
                                        "{channel.name}"
                                    }
                                }
                            }
                        }
                    }
                }
                if discovering {
                    DiscoverLists {}
                } else {
                    if loading {
                        VideoGridSkeleton { count: 8 }
                    } else {
                        VideoGrid { videos: shown.map(|shown| &shown.videos), empty_message: "No matches yet. Check the source connection or try another search.".to_string() }
                    }
                    InfiniteScroll {
                        loading: searching(),
                        complete: remaining == 0 && !has_next_page,
                        on_load: load_more,
                    }
                }
            }
        }
    }
}

/// The followed channels' newest unwatched uploads, as the search page asks
/// for them.
fn following_query(settings: &AppSettings) -> FeedQuery {
    FeedQuery {
        group: FeedGroup::All,
        kind: FeedFilter::All,
        duration: None,
        hide_watched: true,
        thresholds: settings.into(),
    }
}

/// The search page with nothing typed: what YouTube recommended beside recent
/// watches, or the newest uploads from followed channels, each a list that
/// loads another page as the reader nears its end.
#[component]
fn DiscoverLists() -> Element {
    let app_state = use_context::<AppState>();
    let mut list = app_state.explore_discover;
    // Kept in the app state like the feed's, so a returning scroll position
    // has as many pages to land in as were loaded.
    let mut pages = app_state.explore_discover_pages;
    // Whether the last page on screen said there is more.
    let has_more = use_signal(|| false);
    let query = use_memo(move || following_query(&app_state.settings.read()));
    // A different question starts again at one page. The first run is not
    // one: the list and its pages are those this screen was left with.
    let mut asked = use_signal(|| query.peek().clone());
    use_effect(move || {
        let current = query.read().clone();
        if *asked.peek() != current {
            asked.set(current);
            pages.set(1);
        }
    });

    let load_next_page = use_callback(move |_: ()| {
        if *has_more.peek() {
            pages += 1;
        }
    });

    let shown = list();
    let page_count = pages();

    rsx! {
        Shelf { aria_label: "Suggestions", gap: Space::Sm, snap: false,
            for option in DiscoverList::ALL {
                Chip {
                    key: "{option.label()}",
                    selected: shown == option,
                    onclick: move |_| {
                        if *list.peek() != option {
                            list.set(option);
                            pages.set(1);
                        }
                    },
                    "{option.label()}"
                }
            }
        }
        Text { tone: TextTone::Secondary,
            match shown {
                DiscoverList::Recommended => "Suggested by YouTube beside videos you watched",
                DiscoverList::Following => "The newest unwatched uploads from channels you follow",
            }
        }
        // Keyed by the list as well as the page, so switching lists builds
        // fresh pages rather than handing one list's page to the other.
        for page in 0..page_count {
            match shown {
                DiscoverList::Recommended => rsx! {
                    RecommendationPageView {
                        key: "recommended-{page}",
                        page,
                        last: page + 1 == page_count,
                        has_more,
                    }
                },
                DiscoverList::Following => rsx! {
                    FeedPageView {
                        key: "following-{page}",
                        query,
                        page,
                        last: page + 1 == page_count,
                        has_more,
                        empty_message: "Follow some channels and their new uploads will show up here.".to_string(),
                    }
                },
            }
        }
        InfiniteScroll {
            loading: false,
            complete: !has_more(),
            on_load: load_next_page,
        }
    }
}

/// One page of recommendations: its own cached read, so the pages above it
/// stay put while it loads, with a skeleton until it arrives.
#[component]
fn RecommendationPageView(page: usize, last: ReadSignal<bool>, has_more: Signal<bool>) -> Element {
    let found = use_cached(get_recommendations, (page,));
    let videos = use_memo(move || match &*found.read() {
        Some(Ok(found)) => found.videos.clone(),
        _ => Vec::new(),
    });

    let candidates = use_memo(move || match &*found.read() {
        Some(Ok(found)) => found.unknown_durations.clone(),
        _ => Vec::new(),
    });

    use_duration_hydration(candidates);
    // The last page decides whether there is another.
    use_effect(move || {
        let more = matches!(&*found.read(), Some(Ok(found)) if found.has_more);
        if last() && *has_more.peek() != more {
            has_more.set(more);
        }
    });

    match &*found.read() {
        None => rsx! { VideoGridSkeleton { count: if page == 0 { 8 } else { 4 } } },
        Some(Err(_)) if page == 0 => rsx! {
            VideoGrid {
                videos: Vec::new(),
                empty_message: "Could not load recommendations. Check the server connection.".to_string(),
            }
        },
        Some(Err(_)) => rsx! {},
        Some(Ok(found)) if page == 0 || !found.videos.is_empty() => rsx! {
            VideoGrid {
                videos,
                empty_message: "Watch a few videos and YouTube's suggestions for them will show up here.".to_string(),
            }
        },
        Some(Ok(_)) => rsx! {},
    }
}

/// What the device has for a typed `needle`: followed channels, and
/// recommended and suggested videos, that match. Enter asks the server, which
/// searches the whole catalog.
fn local_matches<'a>(
    recommended: &'a [Video],
    suggestions: &'a [Video],
    subscriptions: &'a [Channel],
    needle: &str,
) -> (Vec<&'a Video>, Vec<&'a Channel>) {
    let mut seen = std::collections::HashSet::new();
    let videos = recommended
        .iter()
        .chain(suggestions)
        .filter(|video| seen.insert(video.id.as_str()))
        .filter(|video| {
            video.title.to_lowercase().contains(needle)
                || video.channel_name.to_lowercase().contains(needle)
        })
        .collect();
    let channels = subscriptions
        .iter()
        .filter(|channel| {
            channel.name.to_lowercase().contains(needle)
                || channel.handle.to_lowercase().contains(needle)
        })
        .collect();
    (videos, channels)
}

/// What the page shows, held in a memo.
#[derive(Clone, Debug, Default, PartialEq)]
struct Shown {
    /// Nothing to show yet: no server results and no suggestions.
    loading: bool,
    videos: Vec<Video>,
    channels: Vec<Channel>,
    /// What the query found, before paging.
    result_count: usize,
    /// How many of those are still off the page.
    remaining: usize,
}

impl Shown {
    fn loading() -> Self {
        Self {
            loading: true,
            ..Self::default()
        }
    }
}

/// The first `page` of each list under `filter`, copied out, with the total
/// the filter left and how many of those are still off the page.
fn paged(
    mut videos: Vec<&Video>,
    mut channels: Vec<&Channel>,
    filter: ExploreFilter,
    page: usize,
) -> Shown {
    match filter {
        ExploreFilter::Videos => channels.clear(),
        ExploreFilter::Channels => videos.clear(),
        ExploreFilter::All => {}
    }
    // Counted before paging, so the line above the results reports what the
    // query found rather than how much of it is on screen.
    let result_count = videos.len() + channels.len();
    let remaining = videos.len().saturating_sub(page) + channels.len().saturating_sub(page);
    Shown {
        loading: false,
        videos: videos.into_iter().take(page).cloned().collect(),
        channels: channels.into_iter().take(page).cloned().collect(),
        result_count,
        remaining,
    }
}
