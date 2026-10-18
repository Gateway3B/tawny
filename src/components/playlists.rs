use crate::{
    api::{get_playlist, get_playlist_previews},
    app::Route,
    models::{DurationFilter, PlaylistKind, PlaylistSort},
    state::AppState,
};
use dioxus::prelude::*;
use dioxus_icons::lucide::{
    ArrowDown, ArrowUp, ArrowUpDown, Check, CheckCheck, ChevronsDown, ChevronsUp, EllipsisVertical,
    ListPlus, Pencil, Play, Plus, Shuffle, Trash2,
};
use g3_cache::use_cached;
use g3_route_transitions::animated_navigate;
use g3_ui::{
    BottomSheet, Button, ButtonFill, ButtonSize, Card, Chip, Color, ConfirmModal, Content, Divider,
    DividerOrientation, EmptyState, Grid, GridColumns, Img, InfiniteScroll, Input, Item, List,
    ListLines, ListVariant, Modal, ReorderEvent, ReorderHandle, ReorderItem, ReorderLayout,
    ReorderList, SegmentButton, SegmentGroup, Shelf, Space, Spinner, Stack, StackAlign, Text,
    TextTone,
};

use super::{
    PageHeader, VideoGrid, duration_candidates, use_after_route_transition, use_duration_hydration,
};

/// How many videos a playlist lays out before asking for more. Every card is a
/// swipe row with an image, and a long playlist built them all before the push
/// animation could start.
const PLAYLIST_PAGE_SIZE: usize = 24;
/// Enough cards to fill the first screen, phone or desktop, before the rest.
const PLAYLIST_FIRST_SCREEN: usize = 8;

/// Fisher-Yates with a generator of its own.
///
/// A shuffled playlist needs no more than an order the viewer cannot predict.
/// That does not justify a randomness crate in the client bundle.
fn shuffled(mut ids: Vec<String>) -> Vec<String> {
    use std::sync::atomic::{AtomicU64, Ordering};
    use web_time::{SystemTime, UNIX_EPOCH};

    // The clock alone is not enough. On the web `SystemTime` is `Date.now()`,
    // so two shuffles inside the same millisecond would deal the same order;
    // the counter is what separates a second tap from the first.
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64
        ^ SEQUENCE.fetch_add(0x9E37_79B9_7F4A_7C15, Ordering::Relaxed);
    let mut state = seed;
    let mut next = move || {
        // splitmix64.
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    };
    for index in (1..ids.len()).rev() {
        ids.swap(index, (next() % (index as u64 + 1)) as usize);
    }
    ids
}

/// Start a run of specific videos: the first one plays, the rest wait in the
/// queue.
///
/// Used by Shuffle, whose order exists only for this sitting and so has to be
/// written down. Playing a playlist in its own order goes through
/// [`AppState::start_playlist_run`] instead, which queues the playlist itself.
///
/// The playing video is deliberately left out of the queue. It is what the
/// queue is *for* - the list of what comes next - and the player drops the
/// finished video from it as it advances anyway.
fn play_run(app_state: AppState, ids: Vec<String>, verb: &str) {
    let Some((first, rest)) = ids.split_first() else {
        return;
    };
    // A shuffled order is not the playlist's order, so any playlist run in
    // progress would fight it for what comes next.
    app_state.clear_playlist_run();
    app_state.queue_run(rest);
    let count = ids.len();
    app_state.show_toast(
        format!("{verb} {count} video{}", if count == 1 { "" } else { "s" }),
        Color::Success,
    );
    let first = first.clone();
    spawn(async move {
        animated_navigate(Route::VideoDetail { id: first }).await;
    });
}

/// Hand the playlist to the queue as a run, then open `video_id`.
///
/// The whole point of the marker: what comes next is decided when the current
/// video ends, against the playlist as it is arranged then, so this does not have
/// to know or freeze the order.
fn play_from_playlist(app_state: AppState, playlist_id: &str, video_id: String, announce: bool) {
    app_state.start_playlist_run(playlist_id);
    if announce {
        // Says that the *playlist* is now what plays next, which opening one
        // video does not. Tapping a card is its own answer, so it stays quiet.
        app_state.show_toast("Playing this playlist", Color::Success);
    }
    spawn(async move {
        animated_navigate(Route::VideoDetail { id: video_id }).await;
    });
}

/// One row of the index, resolved once per render.
///
/// Built from a lookup rather than a scan per playlist: finding three
/// thumbnails used to walk the whole video cache for every playlist on the
/// page, which is tens of thousands of rows each time.
struct PlaylistRow {
    id: String,
    name: String,
    count: usize,
    unwatched: usize,
    thumbnails: Vec<String>,
}

/// `ids` with `id` taken out and put back at position `to`.
fn with_moved(ids: &[String], id: &str, to: usize) -> Vec<String> {
    let mut ids = ids.to_vec();
    if let Some(from) = ids.iter().position(|ordered| ordered == id) {
        let moved = ids.remove(from);
        ids.insert(to.min(ids.len()), moved);
    }
    ids
}

/// "1 video", "3 videos".
fn video_count(count: usize) -> String {
    if count == 1 {
        "1 video".to_string()
    } else {
        format!("{count} videos")
    }
}

/// Up to three thumbnails from a playlist: the first large, the next two
/// stacked beside it.
#[component]
fn PlaylistCollage(thumbnails: Vec<String>) -> Element {
    rsx! {
        if thumbnails.is_empty() {
            div {
                class: "grid aspect-video place-items-center",
                style: "background: var(--g3-color-control); color: var(--g3-color-text-tertiary);",
                ListPlus { size: 30 }
            }
        } else {
            div { class: "grid aspect-video grid-cols-3 grid-rows-2 gap-0.5",
                for (index, thumbnail) in thumbnails.into_iter().enumerate() {
                    Img {
                        key: "{index}",
                        src: thumbnail,
                        alt: "",
                        class: if index == 0 { "col-span-2 row-span-2 h-full" } else { "h-full" },
                    }
                }
            }
        }
    }
}

#[component]
pub fn Playlists() -> Element {
    let app_state = use_context::<AppState>();
    let mut create_open = use_signal(|| false);
    let mut playlist_name = use_signal(String::new);
    let mut delete_target = use_signal(|| None::<(String, String)>);
    let mut delete_open = use_signal(|| false);
    let mut menu_target = use_signal(|| None::<(String, String)>);
    let mut menu_open = use_signal(|| false);
    let mut rename_open = use_signal(|| false);
    let mut rename_name = use_signal(String::new);
    // Arranging shows a grip on each cover to drag it into place. Off by default,
    // so a swipe across the grid scrolls the page rather than starting a drag.
    let mut editing = use_signal(|| false);
    // No filters here. The index is a set of covers to pick from, not a list to
    // work through - filtering it hides the playlist you came to open.
    let previews = use_cached(get_playlist_previews, ());
    let rows = match &*previews.read() {
        Some(Ok(previews)) => previews
            .iter()
            .map(|preview| PlaylistRow {
                id: preview.id.clone(),
                name: preview.name.clone(),
                count: preview.count,
                unwatched: preview.unwatched,
                thumbnails: preview.thumbnails.clone(),
            })
            .collect::<Vec<_>>(),
        // Before the covers arrive, the names are already known.
        _ => app_state.with_viewer(|viewer| {
            viewer
                .playlists
                .iter()
                .map(|playlist| PlaylistRow {
                    id: playlist.id.clone(),
                    name: playlist.name.clone(),
                    count: playlist.video_ids.len(),
                    unwatched: playlist.video_ids.len(),
                    thumbnails: Vec::new(),
                })
                .collect::<Vec<_>>()
        }),
    };
    let delete_name = delete_target()
        .map(|(_, name)| name)
        .unwrap_or_else(|| "This playlist".into());
    // The card's menu: which playlist it is for, and where that one sits now,
    // which decides the moves it offers.
    let ordered_ids = rows.iter().map(|row| row.id.clone()).collect::<Vec<_>>();
    let playlist_count = ordered_ids.len();
    let arranging = editing() && playlist_count > 1;
    let dragged_ids = ordered_ids.clone();
    let reorder = move |moved: ReorderEvent| {
        let mut ids = dragged_ids.clone();
        moved.apply(&mut ids);
        app_state.reorder_playlists(ids);
    };
    let menu_name = menu_target().map(|(_, name)| name).unwrap_or_default();
    let menu_name_for_rename = menu_name.clone();
    let menu_index = menu_target()
        .and_then(|(id, _)| ordered_ids.iter().position(|ordered| *ordered == id))
        .unwrap_or(0);
    let move_playlist = move |to: usize| {
        let Some((id, name)) = menu_target() else {
            return;
        };
        app_state.reorder_playlists(with_moved(&ordered_ids, &id, to));
        menu_open.set(false);
        app_state.show_toast(format!("Moved {name}"), Color::Neutral);
    };

    rsx! {
        PageHeader {
            end_slot: rsx! {
                if playlist_count > 1 {
                    // An icon like the header's other buttons: a word here
                    // crowded the title on a phone.
                    Button {
                        fill: ButtonFill::Clear,
                        aria_label: if arranging { "Done arranging" } else { "Arrange playlists" },
                        onclick: move |_| editing.toggle(),
                        if arranging {
                            Check { size: 22 }
                        } else {
                            ArrowUpDown { size: 22 }
                        }
                    }
                }
                Button {
                    fill: ButtonFill::Clear,
                    aria_label: "New playlist",
                    onclick: move |_| create_open.set(true),
                    Plus { size: 22 }
                }
            },
        }
        Content {
            if rows.is_empty() {
                EmptyState {
                    title: "No playlists yet",
                    icon: rsx! { ListPlus { size: 40 } },
                    action: rsx! {
                        Button { start: rsx! { Plus { size: 17 } }, onclick: move |_| create_open.set(true), "New playlist" }
                    },
                    "Swipe a video, or use its menu, to save it to one."
                }
            }
            ReorderList {
                layout: ReorderLayout::Grid,
                disabled: !arranging,
                on_reorder: reorder,
                Grid { columns: GridColumns::Fit(16.0),
                    for (index, row) in rows.into_iter().enumerate() {
                        {
                            let playlist_id = row.id.clone();
                            let remove_id = row.id.clone();
                            let remove_name = row.name.clone();
                            let menu_id = row.id.clone();
                            let menu_label = row.name.clone();
                            // The count that answers "is there anything left in
                            // here", which is the reason to open one.
                            let subtitle = if row.unwatched > 0 && row.unwatched < row.count {
                                format!("{} · {} unwatched", video_count(row.count), row.unwatched)
                            } else {
                                video_count(row.count)
                            };
                            rsx! {
                                ReorderItem { key: "{row.id}", index,
                                    Card {
                                        title: row.name.clone(),
                                        subtitle,
                                        media: rsx! {
                                            // Delete sits where a video card's remove
                                            // does, so the two read as the same control.
                                            div { class: "relative",
                                                PlaylistCollage { thumbnails: row.thumbnails }
                                                if arranging {
                                                    div { class: "absolute top-2 left-2",
                                                        ReorderHandle { label: "Move {row.name}", class: "playlist-grip" }
                                                    }
                                                }
                                                div { class: "absolute top-2 right-2",
                                                    Button {
                                                        size: ButtonSize::Sm,
                                                        color: Color::Neutral,
                                                        aria_label: "Delete {row.name}",
                                                        onclick: move |event: MouseEvent| {
                                                            event.stop_propagation();
                                                            delete_target.set(Some((remove_id.clone(), remove_name.clone())));
                                                            delete_open.set(true);
                                                        },
                                                        Trash2 { size: 15 }
                                                    }
                                                }
                                            }
                                        },
                                        onclick: move |_| { spawn(animated_navigate(Route::PlaylistDetail { id: playlist_id.clone() })); },
                                        end: rsx! {
                                            Button {
                                                fill: ButtonFill::Clear,
                                                size: ButtonSize::Sm,
                                                aria_label: "Actions for {row.name}",
                                                onclick: move |event: MouseEvent| {
                                                    event.stop_propagation();
                                                    menu_target.set(Some((menu_id.clone(), menu_label.clone())));
                                                    menu_open.set(true);
                                                },
                                                EllipsisVertical { size: 20 }
                                            }
                                        },
                                    }
                                }
                            }
                        }
                    }
                }
            }
            BottomSheet { open: menu_open, title: "{menu_name}",
                List { variant: ListVariant::Raised, lines: ListLines::Inset,
                    Item {
                        start: rsx! { Pencil { size: 18 } },
                        label: "Rename",
                        onclick: move |_| {
                            rename_name.set(menu_name_for_rename.clone());
                            menu_open.set(false);
                            rename_open.set(true);
                        },
                    }
                    if menu_index > 0 {
                        Item {
                            start: rsx! { ChevronsUp { size: 18 } },
                            label: "Move to the top",
                            onclick: { let mut go = move_playlist.clone(); move |_| go(0) },
                        }
                        Item {
                            start: rsx! { ArrowUp { size: 18 } },
                            label: "Move up",
                            onclick: { let mut go = move_playlist.clone(); move |_| go(menu_index - 1) },
                        }
                    }
                    if menu_index + 1 < playlist_count {
                        Item {
                            start: rsx! { ArrowDown { size: 18 } },
                            label: "Move down",
                            onclick: { let mut go = move_playlist.clone(); move |_| go(menu_index + 1) },
                        }
                        Item {
                            start: rsx! { ChevronsDown { size: 18 } },
                            label: "Move to the bottom",
                            onclick: { let mut go = move_playlist.clone(); move |_| go(playlist_count - 1) },
                        }
                    }
                }
            }
            Modal {
                open: rename_open,
                title: "Rename playlist",
                actions: rsx! {
                    Button { fill: ButtonFill::Clear, onclick: move |_| rename_open.set(false), "Cancel" }
                    Button {
                        disabled: rename_name().trim().is_empty(),
                        onclick: move |_| {
                            let name = rename_name().trim().to_string();
                            let Some((id, _)) = menu_target() else { return };
                            if name.is_empty() { return; }
                            app_state.rename_playlist(&id, name.clone());
                            rename_open.set(false);
                            app_state.show_toast(format!("Renamed to {name}"), Color::Success);
                        },
                        "Rename"
                    }
                },
                Input { label: "Playlist name", value: rename_name, autofocus: true }
            }
            ConfirmModal {
                open: delete_open,
                title: "Delete playlist?",
                message: "{delete_name} will be removed from this device and your sync server.",
                confirm_label: "Delete",
                destructive: true,
                on_confirm: move |_| {
                    if let Some((id, _)) = delete_target()
                        && let Some(name) = app_state.delete_playlist(&id)
                    {
                        app_state.show_toast(format!("Deleted {name}"), Color::Neutral);
                    }
                    delete_target.set(None);
                },
            }
            Modal {
                open: create_open,
                title: "New playlist",
                actions: rsx! {
                    Button { fill: ButtonFill::Clear, onclick: move |_| create_open.set(false), "Cancel" }
                    Button {
                        disabled: playlist_name().trim().is_empty(),
                        onclick: move |_| {
                            let name = playlist_name().trim().to_string();
                            if name.is_empty() { return; }
                            app_state.create_playlist(name.clone());
                            playlist_name.set(String::new());
                            create_open.set(false);
                            app_state.show_toast(format!("Created {name}"), Color::Success);
                        },
                        "Create"
                    }
                },
                Input {
                    label: "Playlist name",
                    value: playlist_name,
                    placeholder: "Sunday watchlist",
                    helper: "It will be cached on this device immediately.",
                    autofocus: true,
                }
            }
        }
    }
}

#[component]
pub fn PlaylistDetail(id: String) -> Element {
    let app_state = use_context::<AppState>();
    // The arrangement lives in settings rather than in local signals, because a
    // run resolved out of the queue has to be able to read it long after this
    // page is gone. See `PlaylistView`.
    let view = app_state.playlist_view(&id);
    // The segmented control follows the persisted kind. Settings hydrate from
    // local storage after the first render, hence the effect rather than an
    // initial value alone.
    let mut kind = use_signal(|| view.kind);
    let persisted_kind_id = id.clone();
    use_effect(move || {
        let persisted = app_state.playlist_view(&persisted_kind_id).kind;
        if *kind.peek() != persisted {
            kind.set(persisted);
        }
    });

    // Shares its cache entry with a run of this playlist, so an edit here is
    // what the run walks next.
    let contents = use_cached(get_playlist, (id.clone(),));
    let (playlist, mut videos) = match &*contents.read() {
        Some(Ok(Some(found))) => (Some(found.playlist.clone()), found.videos.clone()),
        _ => (None, Vec::new()),
    };
    let loading = contents.read().is_none();

    // Ahead of the early return below, because a hook has to run on every
    // render of this component, and ahead of the filters, because
    // `duration_candidates` has to see the rows a duration chip discards.
    //
    // The window is the whole list, not the page on screen: one visit works
    // through it a batch at a time until every entry has a length. That is the
    // point - a saved video is usually saved from the feed, where RSS gave it no
    // runtime, and the refresh backfill only ever reaches followed channels.
    //
    // Held until the push has landed. Each batch writes the library, and that
    // re-render arriving mid-slide is what made opening a playlist stutter.
    let settled = use_after_route_transition();
    let candidates = use_memo(move || {
        if !settled() {
            return Vec::new();
        }
        match &*contents.read() {
            Some(Ok(Some(found))) => duration_candidates(&found.videos, found.videos.len()),
            _ => Vec::new(),
        }
    });
    use_duration_hydration(candidates);
    let mut visible_count = use_signal(|| PLAYLIST_PAGE_SIZE);
    // The cards on screen, as a memo so the grid only renders again when they
    // change. Built in the render, every background refetch rebuilt all of
    // them mid-slide, identical or not. And only the first screenful comes
    // with the slide: a push cannot start moving until its page has rendered,
    // and a full page of cards held a cached playlist still for ~0.4 s after
    // the tap (4x CPU). The rest fill in once it has landed.
    let shown_id = id.clone();
    let shown = use_memo(move || {
        let Some(Ok(Some(found))) = &*contents.read() else {
            return Vec::new();
        };
        let view = app_state.playlist_view(&shown_id);
        let mut videos = found.videos.clone();
        view.arrange(&mut videos, &app_state.settings.read());
        videos.retain(|video| view.shows(video));
        let limit = if settled() {
            visible_count()
        } else {
            visible_count().min(PLAYLIST_FIRST_SCREEN)
        };
        videos.truncate(limit);
        videos
    });

    let Some(playlist) = playlist else {
        // Still renders the bar: reaching a stale deep link with no way back was
        // a dead end.
        return rsx! {
            PageHeader { title: "Playlist".to_string(), back_to: Route::Playlists {} }
            Content {
                if loading {
                    Spinner { center: true }
                } else {
                EmptyState {
                    title: "Playlist not found",
                    icon: rsx! { ListPlus { size: 40 } },
                    "It may have been deleted, or be on another account."
                }
                }
            }
        };
    };

    let saved_count = playlist.video_ids.len();
    let watched_count = videos.iter().filter(|video| video.watched).count();
    // Same trap as the feed: the subscription RSS carries no duration, so a
    // saved row often stores 0, and a duration chip is right to exclude it but
    // cannot say so on its own. Counted before the filter runs, because after it
    // they are gone - and past the kind filter, so a Shorts view does not claim
    // to be missing the lengths of long-form uploads it was not showing anyway.
    let without_duration = if view.duration.is_some() {
        videos
            .iter()
            .filter(|video| {
                view.kind.matches(video) && !video.is_live && video.duration_seconds == 0
            })
            .count()
    } else {
        0
    };
    // Exactly what a run walks, arranged the same way, so what is on screen and
    // what plays next cannot disagree.
    view.arrange(&mut videos, &app_state.settings.read());
    videos.retain(|video| view.shows(video));

    let run: Vec<String> = videos.iter().map(|video| video.id.clone()).collect();
    // The run above is the whole arranged list; only its first pages are laid
    // out, by `shown`.
    let remaining = videos.len().saturating_sub(visible_count());
    let shuffle_run = run.clone();
    let filtered = view.is_filtered();
    let clear_id = playlist.id.clone();
    let play_all_id = playlist.id.clone();
    let play_all_first = run.first().cloned();
    let grid_playlist_id = playlist.id.clone();
    let chip_view = view.clone();
    let sort_view = view.clone();
    let unwatched_view = view.clone();
    let kind_view = view.clone();
    let kind_id = playlist.id.clone();
    let unwatched_id = playlist.id.clone();
    let chip_id = playlist.id.clone();
    let sort_id = playlist.id.clone();

    rsx! {
        PageHeader {
            title: playlist.name.clone(),
            back_to: Route::Playlists {},
            // The same control the feed carries, in the same place, because it
            // answers the same question: which kind of upload am I looking at.
            toolbar: rsx! {
                SegmentGroup {
                    value: kind,
                    aria_label: "Show",
                    onchange: move |picked: PlaylistKind| {
                        let mut updated = kind_view.clone();
                        updated.kind = picked;
                        app_state.set_playlist_view(&kind_id, updated);
                    },
                    for option in PlaylistKind::ALL {
                        SegmentButton { key: "{option.label()}", value: option, "{option.label()}" }
                    }
                }
            },
        }
        Content {
            Stack { gap: Space::Md,
                if saved_count > 1 {
                    Shelf { aria_label: "Playlist filters and order", gap: Space::Sm,
                        Chip {
                            selected: unwatched_view.only_unwatched,
                            onclick: move |_| {
                                let mut updated = unwatched_view.clone();
                                updated.only_unwatched = !updated.only_unwatched;
                                app_state.set_playlist_view(&unwatched_id, updated);
                            },
                            "Unwatched"
                        }
                        for duration in DurationFilter::ALL {
                            {
                                let active = chip_view.duration == Some(duration);
                                let chip_view = chip_view.clone();
                                let chip_id = chip_id.clone();
                                rsx! {
                                    Chip {
                                        key: "{duration.label()}",
                                        selected: active,
                                        onclick: move |_| {
                                            let mut updated = chip_view.clone();
                                            updated.duration = if active { None } else { Some(duration) };
                                            app_state.set_playlist_view(&chip_id, updated);
                                        },
                                        "{duration.label()}"
                                    }
                                }
                            }
                        }
                        Divider { orientation: DividerOrientation::Vertical }
                        for option in PlaylistSort::ALL {
                            {
                                let active = sort_view.sort == option;
                                let direction = if active { sort_view.descending } else { option.default_descending() };
                                let sort_view = sort_view.clone();
                                let sort_id = sort_id.clone();
                                rsx! {
                                    Chip {
                                        key: "{option:?}",
                                        selected: active,
                                        // Tapping the current order again reverses it.
                                        onclick: move |_| {
                                            let mut updated = sort_view.clone();
                                            if active {
                                                updated.descending = !updated.descending;
                                            } else {
                                                updated.sort = option;
                                                updated.descending = option.default_descending();
                                            }
                                            app_state.set_playlist_view(&sort_id, updated);
                                        },
                                        "{option.label(direction)}"
                                    }
                                }
                            }
                        }
                    }
                }
                // Keep playlist-wide actions below the filters they operate on.
                // Cleanup remains neutral and outlined, with the same full tap
                // height as the playback controls.
                Stack { class: "playlist-actions", horizontal: true, gap: Space::Sm, align: StackAlign::Center,
                    Button {
                        class: "playlist-play",
                        aria_label: "Play all",
                        disabled: run.is_empty(),
                        start: rsx! { Play { size: 17, fill: "currentColor" } },
                        onclick: move |_| {
                            if let Some(first) = play_all_first.clone() {
                                play_from_playlist(app_state, &play_all_id, first, true);
                            }
                        },
                        span { class: "playlist-collapse-label", "Play all" }
                    }
                    Button {
                        class: "playlist-icon-only",
                        aria_label: "Shuffle",
                        fill: ButtonFill::Outline,
                        color: Color::Neutral,
                        disabled: shuffle_run.len() < 2,
                        start: rsx! { Shuffle { size: 17 } },
                        onclick: move |_| play_run(app_state, shuffled(shuffle_run.clone()), "Shuffling"),
                    }
                    if watched_count > 0 {
                        Button {
                            class: "playlist-remove-watched",
                            fill: ButtonFill::Outline,
                            color: Color::Neutral,
                            start: rsx! { CheckCheck { size: 15 } },
                            onclick: move |_| {
                                // Toasts the count once the server has answered.
                                app_state.remove_watched_from_playlist(&clear_id);
                            },
                            "Remove watched"
                        }
                    }
                }
                if without_duration > 0 {
                    Text { tone: TextTone::Secondary,
                        "{without_duration} more "
                        if without_duration == 1 { "video has" } else { "videos have" }
                        " no length yet, so a duration filter cannot speak for them. They are being filled in now."
                    }
                }
                VideoGrid {
                    videos: shown,
                    playlist_id: grid_playlist_id,
                    empty_message: if filtered {
                        "Nothing here matches those filters.".to_string()
                    } else {
                        "Add videos from the buttons on a card.".to_string()
                    },
                }
                InfiniteScroll {
                    loading: false,
                    complete: remaining == 0,
                    on_load: move |_| {
                        // Only a first screenful is out during the slide, which
                        // leaves this in view without anyone having scrolled.
                        if remaining > 0 && settled() {
                            visible_count += PLAYLIST_PAGE_SIZE;
                        }
                    },
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::with_moved;

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|id| id.to_string()).collect()
    }

    #[test]
    fn a_playlist_moves_to_the_place_asked_for() {
        let order = ids(&["a", "b", "c", "d"]);
        assert_eq!(with_moved(&order, "c", 0), ids(&["c", "a", "b", "d"]));
        assert_eq!(with_moved(&order, "a", 3), ids(&["b", "c", "d", "a"]));
        assert_eq!(with_moved(&order, "b", 2), ids(&["a", "c", "b", "d"]));
    }

    #[test]
    fn moving_somewhere_unreasonable_or_something_missing_changes_nothing_odd() {
        let order = ids(&["a", "b"]);
        assert_eq!(with_moved(&order, "a", 9), ids(&["b", "a"]));
        assert_eq!(with_moved(&order, "gone", 0), order);
    }
}
