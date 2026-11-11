use crate::{
    app::Route,
    models::{Channel, GroupSuggestion},
    state::AppState,
};
use dioxus::prelude::*;
use dioxus_icons::lucide::{Check, Pencil, Plus, Sparkles, Tags, Trash2, Users};
use g3_route_transitions::animated_navigate;
use g3_ui::{
    Avatar, AvatarSize, BottomSheet, Button, ButtonExpand, ButtonFill, ButtonSize, Card, Checkbox,
    Color, Content, EmptyState, Grid, GridColumns, InfiniteScroll, Input, Item, ItemDetail, List,
    ListLines, ListVariant, Modal, Searchbar, Skeleton, SkeletonShape, Space, Stack, StackAlign,
    Text, TextTone, TextVariant,
};
use std::collections::HashSet;

use super::{PageHeader, use_after_first_paint};

/// How many subscribed channels the grid lays out before asking for more.
/// Each is a card with an avatar image, and a long subscription list built
/// them all in one render.
const CHANNEL_PAGE_SIZE: usize = 48;

/// How many non-member channels the group editor lays out at a time.
const EDITOR_PAGE_SIZE: usize = 40;

/// One channel tile, used by both the subscribed grid and the suggestions.
#[component]
pub fn ChannelCard(channel: Channel) -> Element {
    let app_state = use_context::<AppState>();
    let toggle_channel = channel.clone();
    let open_channel_id = channel.id.clone();
    let groups_channel = channel.clone();
    let is_subscribed = app_state.follows(&channel.id);
    let subscribers = channel.subscriber_count.trim().to_string();
    let groups = app_state.group_names_for(&channel.id);
    let groups_label = if groups.is_empty() {
        "No group".to_string()
    } else {
        groups.join(", ")
    };

    rsx! {
        Card {
            title: channel.name.clone(),
            subtitle: (!subscribers.is_empty()).then(|| format!("{subscribers} subscribers")),
            class: "h-full [&_.g3-card-title]:line-clamp-1",
            start: rsx! {
                Avatar { name: channel.name.clone(), src: channel.avatar_url.clone() }
            },
            onclick: move |_| { spawn(animated_navigate(Route::ChannelDetail { id: open_channel_id.clone() })); },
            end: rsx! {
                Button {
                    size: ButtonSize::Sm,
                    fill: if is_subscribed { ButtonFill::Outline } else { ButtonFill::Solid },
                    color: if is_subscribed { Color::Neutral } else { Color::Accent },
                    "aria-pressed": if is_subscribed { "true" } else { "false" },
                    start: is_subscribed.then(|| rsx! { Check { size: 15 } }),
                    onclick: move |_| {
                        app_state.toggle_subscription_for(&toggle_channel);
                    },
                    if is_subscribed { "Subscribed" } else { "Subscribe" }
                }
            },
            // Under the name rather than beside it, where a phone had no room
            // left for the name. Naming the groups lets an unsorted channel
            // stand out while scrolling the whole list.
            if is_subscribed {
                Button {
                    size: ButtonSize::Sm,
                    fill: ButtonFill::Clear,
                    color: Color::Neutral,
                    class: "max-w-full [&_.g3-btn-label]:truncate",
                    start: rsx! { Tags { size: 15 } },
                    aria_label: "Groups for {channel.name}: {groups_label}",
                    onclick: move |_| app_state.open_group_picker(&groups_channel.id, &groups_channel.name),
                    "{groups_label}"
                }
            }
        }
    }
}

/// One channel in a group's membership editor: a checkbox for this group,
/// with the channel's other groups named under it.
#[component]
fn GroupMemberRow(
    channel: Channel,
    group_id: String,
    group_name: String,
    member: ReadSignal<bool>,
    other_groups: Vec<String>,
) -> Element {
    let app_state = use_context::<AppState>();
    let mut checked = use_signal(|| *member.peek());
    // The membership also changes from the row's text and from the picker.
    use_effect(move || {
        let member = member();
        if *checked.peek() != member {
            checked.set(member);
        }
    });

    let toggle_group = group_id.clone();
    let toggle_channel = channel.id.clone();
    let check_channel = channel.id.clone();

    rsx! {
        Item {
            label: channel.name.clone(),
            description: (!other_groups.is_empty()).then(|| format!("Also in {}", other_groups.join(", "))),
            start: rsx! {
                Avatar { name: channel.name.clone(), src: channel.avatar_url.clone(), size: AvatarSize::Sm }
            },
            onclick: move |_| app_state.set_channel_in_group(&toggle_group, &toggle_channel, !member()),
            // The row toggles membership; a chevron would promise a page.
            detail: ItemDetail::Hide,
            // One control per row. A second button for the channel's other
            // groups sat beside the checkbox and read as a second checkbox.
            end: rsx! {
                Checkbox {
                    checked,
                    aria_label: "{channel.name} in {group_name}",
                    onchange: move |value| app_state.set_channel_in_group(&group_id, &check_channel, value),
                }
            },
        }
    }
}

/// One suggested channel: a checkbox for whether "Add" takes it.
#[component]
fn SuggestionRow(
    channel: Channel,
    picked: Signal<HashSet<String>>,
    other_groups: Vec<String>,
) -> Element {
    let mut checked = use_signal(|| picked.peek().contains(&channel.id));
    let row_channel = channel.id.clone();
    let check_channel = channel.id.clone();
    let mut pick = move |channel_id: &str, value: bool| {
        if value {
            picked.write().insert(channel_id.to_string());
        } else {
            picked.write().remove(channel_id);
        }
    };

    rsx! {
        Item {
            label: channel.name.clone(),
            description: (!other_groups.is_empty()).then(|| format!("In {}", other_groups.join(", "))),
            start: rsx! {
                Avatar { name: channel.name.clone(), src: channel.avatar_url.clone(), size: AvatarSize::Sm }
            },
            onclick: move |_| {
                let value = !checked();
                checked.set(value);
                pick(&row_channel, value);
            },
            detail: ItemDetail::Hide,
            end: rsx! {
                Checkbox {
                    checked,
                    aria_label: "Add {channel.name}",
                    onchange: move |value| pick(&check_channel, value),
                }
            },
        }
    }
}

/// "1 channel", "3 channels".
pub(super) fn channel_count(count: usize) -> String {
    if count == 1 {
        "1 channel".to_string()
    } else {
        format!("{count} channels")
    }
}

#[component]
pub fn Subscriptions() -> Element {
    let app_state = use_context::<AppState>();
    // The name dialog both creates and renames: `renaming` holds the group
    // being renamed, and is empty for a new one.
    let mut name_dialog_open = use_signal(|| false);
    let mut renaming = use_signal(|| None::<String>);
    let mut group_name = use_signal(String::new);
    // Which group's membership editor is open.
    let mut editing_group = use_signal(|| None::<String>);
    let mut editor_open = use_signal(|| false);
    let mut group_search = use_signal(String::new);
    // The members when the editor opened. They lead the list and keep their
    // place while boxes are ticked, so a row does not jump from under a finger.
    let mut opened_members = use_signal(HashSet::<String>::new);
    // How many of the other subscriptions are laid out: there can be hundreds.
    let mut others_shown = use_signal(|| EDITOR_PAGE_SIZE);
    // Suggested channels for the group being edited; `None` while they load.
    let mut suggest_open = use_signal(|| false);
    let mut suggestions = use_signal(|| None::<Result<Vec<GroupSuggestion>, String>>);
    let mut picked = use_signal(HashSet::<String>::new);
    // Which request the sheet is waiting for, so a slow answer for a group
    // the viewer has since left is dropped.
    let mut suggest_request = use_signal(|| 0_u64);
    // The editor steps aside for a dialog opened from it, which a sheet would
    // cover, and comes back once that dialog closes however it was closed.
    let mut return_to_editor = use_signal(|| false);
    use_effect(move || {
        if !name_dialog_open() && !suggest_open() && *return_to_editor.peek() {
            return_to_editor.set(false);
            editor_open.set(true);
        }
    });

    let mut visible_count = use_signal(|| CHANNEL_PAGE_SIZE);
    // Sorting out the channel catalogue is the slow part of this page, so the
    // header and placeholders go up first.
    let painted = use_after_first_paint();

    let groups = app_state.with_viewer(|viewer| viewer.subscription_groups.clone());
    let subscribed = if painted() {
        app_state.with_viewer(|viewer| viewer.subscriptions.clone())
    } else {
        Vec::new()
    };
    let remaining = subscribed.len().saturating_sub(visible_count());

    let editing = editing_group();
    let editing_group_name = editing
        .as_ref()
        .and_then(|id| groups.iter().find(|group| &group.id == id))
        .map(|group| group.name.clone())
        .unwrap_or_default();
    let editing_members = editing
        .as_ref()
        .and_then(|id| groups.iter().find(|group| &group.id == id))
        .map(|group| group.channel_ids.clone())
        .unwrap_or_default();
    let group_query = group_search().trim().to_lowercase();
    // Only while the editor is up. Members first, then the rest a page at
    // a time.
    let (editor_members, editor_others, more_others) = if editor_open() {
        let opened = opened_members.read();
        let (members, others): (Vec<_>, Vec<_>) = subscribed
            .iter()
            .filter(|channel| {
                group_query.is_empty()
                    || channel.name.to_lowercase().contains(&group_query)
                    || channel.handle.to_lowercase().contains(&group_query)
            })
            .cloned()
            .partition(|channel| opened.contains(&channel.id));
        let more = others.len().saturating_sub(others_shown());
        let others = others.into_iter().take(others_shown()).collect::<Vec<_>>();
        (members, others, more)
    } else {
        (Vec::new(), Vec::new(), 0)
    };
    let member_count = editing_members.len();
    // Every other group each channel is in, named on its editor row.
    let other_groups_of = |channel_id: &str| {
        groups
            .iter()
            .filter(|group| Some(&group.id) != editing.as_ref())
            .filter(|group| group.channel_ids.iter().any(|id| id == channel_id))
            .map(|group| group.name.clone())
            .collect::<Vec<_>>()
    };
    let is_renaming = renaming().is_some();
    let rename_current = editing_group_name.clone();
    // The suggested channels, split into the ticked and the unticked tier.
    let (likely_channels, maybe_channels) = if suggest_open() {
        match &*suggestions.read() {
            Some(Ok(list)) => {
                let tier = |likely: bool| {
                    list.iter()
                        .filter(|suggestion| suggestion.likely == likely)
                        .filter_map(|suggestion| {
                            subscribed
                                .iter()
                                .find(|channel| channel.id == suggestion.channel_id)
                        })
                        .cloned()
                        .collect::<Vec<_>>()
                };
                (tier(true), tier(false))
            }
            _ => (Vec::new(), Vec::new()),
        }
    } else {
        (Vec::new(), Vec::new())
    };
    let suggestions_loading = suggestions.read().is_none();
    let suggestions_error = match &*suggestions.read() {
        Some(Err(message)) => Some(message.clone()),
        _ => None,
    };
    let picked_count = picked.read().len();
    let add_group_name = editing_group_name.clone();
    let add_order = likely_channels
        .iter()
        .chain(&maybe_channels)
        .map(|channel| channel.id.clone())
        .collect::<Vec<_>>();

    rsx! {
        PageHeader {}
        Content {
            Stack { gap: Space::Lg,
                Card {
                    title: "Groups",
                    subtitle: "Group channels to filter your feed with one tap.",
                    end: rsx! {
                        Button {
                            size: ButtonSize::Sm,
                            fill: ButtonFill::Clear,
                            start: rsx! { Plus { size: 16 } },
                            onclick: move |_| {
                                renaming.set(None);
                                group_name.set(String::new());
                                name_dialog_open.set(true);
                            },
                            "New group"
                        }
                    },
                    if !groups.is_empty() {
                        List { variant: ListVariant::Filled, lines: ListLines::Inset,
                            for group in groups.clone() {
                                {
                                    let group_id = group.id.clone();
                                    let members_at_open = group.channel_ids.iter().cloned().collect::<HashSet<_>>();
                                    let rename_id = group.id.clone();
                                    let rename_name = group.name.clone();
                                    let delete_id = group.id.clone();
                                    rsx! {
                                        Item {
                                            key: "{group.id}",
                                            // A long name wraps rather than ending in "…".
                                            wrap: true,
                                            label: group.name.clone(),
                                            description: channel_count(group.channel_ids.len()),
                                            onclick: move |_| {
                                                editing_group.set(Some(group_id.clone()));
                                                opened_members.set(members_at_open.clone());
                                                others_shown.set(EDITOR_PAGE_SIZE);
                                                group_search.set(String::new());
                                                editor_open.set(true);
                                            },
                                            // No channel faces here: on a phone they
                                            // covered the group's name.
                                            end: rsx! {
                                                Button {
                                                    fill: ButtonFill::Clear,
                                                    color: Color::Neutral,
                                                    size: ButtonSize::Sm,
                                                    aria_label: "Rename {group.name}",
                                                    onclick: move |_| {
                                                        renaming.set(Some(rename_id.clone()));
                                                        group_name.set(rename_name.clone());
                                                        name_dialog_open.set(true);
                                                    },
                                                    Pencil { size: 16 }
                                                }
                                                Button {
                                                    fill: ButtonFill::Clear,
                                                    color: Color::Danger,
                                                    size: ButtonSize::Sm,
                                                    aria_label: "Delete {group.name}",
                                                    onclick: move |_| {
                                                        if let Some(name) = app_state.delete_subscription_group(&delete_id) {
                                                            app_state.show_toast(format!("Deleted {name}"), Color::Neutral);
                                                        }
                                                    },
                                                    Trash2 { size: 16 }
                                                }
                                            },
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                Stack { gap: Space::Sm,
                    // Styled as the Suggested shelf title below, so the two
                    // sections read as peers.
                    if !painted() {
                        Text { variant: g3_ui::TextVariant::Overline, "Subscriptions" }
                        Grid { columns: GridColumns::Fit(20.0),
                            for index in 0..6 {
                                Skeleton { key: "{index}", shape: SkeletonShape::Block, class: "h-16 rounded-lg" }
                            }
                        }
                    } else if subscribed.is_empty() {
                        Text { variant: g3_ui::TextVariant::Overline, "Subscriptions · 0" }
                        EmptyState {
                            title: "No subscriptions yet",
                            icon: rsx! { Users { size: 40 } },
                            "Subscribe to a channel from search or a video to see it here."
                        }
                    } else {
                        Text { variant: g3_ui::TextVariant::Overline, "Subscriptions · {subscribed.len()}" }
                        Grid { columns: GridColumns::Fit(20.0),
                            for channel in subscribed.iter().take(visible_count()).cloned() {
                                ChannelCard { key: "{channel.id}", channel }
                            }
                        }
                        InfiniteScroll {
                            loading: false,
                            complete: remaining == 0,
                            on_load: move |_| {
                                if remaining > 0 {
                                    visible_count += CHANNEL_PAGE_SIZE;
                                }
                            },
                        }
                    }
                }

            }

            // Membership editing lives in one place, so adding a channel to a
            // specific group does not mean hunting for that channel's tile. A
            // tall sheet: it clears the status bar and the tab bar, keeps its
            // buttons at the top of a long list, and swipes away like the
            // app's other pickers. A modal grew past the screen on a phone.
            BottomSheet {
                open: editor_open,
                title: editing_group_name.clone(),
                max_height: "calc(100dvh - 7rem)",
                Stack {
                    Stack { horizontal: true, gap: Space::Sm, align: StackAlign::Center,
                        Button {
                            size: ButtonSize::Sm,
                            fill: ButtonFill::Outline,
                            start: rsx! { Sparkles { size: 16 } },
                            disabled: subscribed.is_empty(),
                            onclick: move |_| {
                                let Some(group_id) = editing_group.peek().clone() else { return };
                                let request = *suggest_request.peek() + 1;
                                suggest_request.set(request);
                                suggestions.set(None);
                                picked.set(HashSet::new());
                                return_to_editor.set(true);
                                editor_open.set(false);
                                suggest_open.set(true);
                                spawn(async move {
                                    let result = crate::api::suggest_group_channels(group_id)
                                        .await
                                        .map_err(|error| crate::auth::readable(&error.to_string()));
                                    if *suggest_request.peek() != request {
                                        return;
                                    }
                                    if let Ok(list) = &result {
                                        picked.set(
                                            list.iter()
                                                .filter(|suggestion| suggestion.likely)
                                                .map(|suggestion| suggestion.channel_id.clone())
                                                .collect(),
                                        );
                                    }
                                    suggestions.set(Some(result));
                                });
                            },
                            "Suggest"
                        }
                        div { class: "flex-1" }
                        Button {
                            size: ButtonSize::Sm,
                            fill: ButtonFill::Clear,
                            color: Color::Neutral,
                            // An icon: with words, three buttons did not fit a
                            // 360px phone and every label broke mid-word.
                            aria_label: "Rename {editing_group_name}",
                            onclick: move |_| {
                                renaming.set(editing_group.peek().clone());
                                group_name.set(rename_current.clone());
                                return_to_editor.set(true);
                                editor_open.set(false);
                                name_dialog_open.set(true);
                            },
                            Pencil { size: 18 }
                        }
                        Button { size: ButtonSize::Sm, onclick: move |_| editor_open.set(false), "Done" }
                    }
                    Searchbar { value: group_search, placeholder: "Search subscribed channels", debounce_ms: 0 }
                    if subscribed.is_empty() {
                        Text { tone: TextTone::Secondary, "Subscribe to a channel first." }
                    } else if editor_members.is_empty() && editor_others.is_empty() {
                        Text { tone: TextTone::Secondary, "No subscribed channels match your search." }
                    } else {
                        if !editor_members.is_empty() {
                            Text { variant: TextVariant::Overline, "In this group · {member_count}" }
                            List { lines: ListLines::Inset,
                                for channel in editor_members {
                                    GroupMemberRow {
                                        key: "{channel.id}",
                                        member: editing_members.contains(&channel.id),
                                        other_groups: other_groups_of(&channel.id),
                                        group_id: editing.clone().unwrap_or_default(),
                                        group_name: editing_group_name.clone(),
                                        channel,
                                    }
                                }
                            }
                        }
                        if !editor_others.is_empty() {
                            Text { variant: TextVariant::Overline, "Add from your subscriptions" }
                            List { lines: ListLines::Inset,
                                for channel in editor_others {
                                    GroupMemberRow {
                                        key: "{channel.id}",
                                        member: editing_members.contains(&channel.id),
                                        other_groups: other_groups_of(&channel.id),
                                        group_id: editing.clone().unwrap_or_default(),
                                        group_name: editing_group_name.clone(),
                                        channel,
                                    }
                                }
                            }
                        }
                        if more_others > 0 {
                            Button {
                                fill: ButtonFill::Clear,
                                expand: ButtonExpand::Block,
                                onclick: move |_| others_shown += EDITOR_PAGE_SIZE,
                                "Show more ({more_others})"
                            }
                        }
                    }
                }
            }

            // In place of the editor, which steps aside for it and returns
            // after. Clear matches start ticked; the rest are offered.
            BottomSheet {
                open: suggest_open,
                title: format!("Suggestions for {editing_group_name}"),
                max_height: "calc(100dvh - 7rem)",
                on_dismiss: move |_| suggest_request += 1,
                Stack {
                    Stack { horizontal: true, gap: Space::Sm, align: StackAlign::Center,
                        Button {
                            size: ButtonSize::Sm,
                            fill: ButtonFill::Clear,
                            color: Color::Neutral,
                            onclick: move |_| {
                                suggest_request += 1;
                                suggest_open.set(false);
                            },
                            "Cancel"
                        }
                        div { class: "flex-1" }
                        Button {
                            size: ButtonSize::Sm,
                            disabled: picked_count == 0,
                            onclick: move |_| {
                                let Some(group_id) = editing_group.peek().clone() else { return };
                                let chosen = picked.peek().clone();
                                let channel_ids = add_order
                                    .iter()
                                    .filter(|id| chosen.contains(*id))
                                    .cloned()
                                    .collect::<Vec<_>>();
                                let count = channel_ids.len();
                                // The added channels join the members at the top.
                                opened_members.write().extend(channel_ids.iter().cloned());
                                app_state.add_channels_to_group(&group_id, channel_ids);
                                app_state.show_toast(
                                    format!("Added {} to {add_group_name}", channel_count(count)),
                                    Color::Success,
                                );
                                suggest_open.set(false);
                            },
                            if picked_count == 0 { "Add" } else { "Add {picked_count}" }
                        }
                    }
                    if suggestions_loading {
                        Text { tone: TextTone::Secondary,
                            "Reading your channels. The first time on a server this takes a minute or so."
                        }
                        for index in 0..5 {
                            Skeleton { key: "{index}", shape: SkeletonShape::Block, class: "h-12 rounded-lg" }
                        }
                    } else if let Some(message) = suggestions_error {
                        EmptyState {
                            title: "Could not suggest channels",
                            icon: rsx! { Sparkles { size: 40 } },
                            "{message}"
                        }
                    } else if likely_channels.is_empty() && maybe_channels.is_empty() {
                        EmptyState {
                            title: "No clear matches",
                            icon: rsx! { Sparkles { size: 40 } },
                            "None of your other subscriptions stand out for this group. A more descriptive name, or a few channels added by hand, gives it more to go on."
                        }
                    } else {
                        Text { tone: TextTone::Secondary,
                            "Matched on each channel's description and recent videos. Untick any that don't belong."
                        }
                        if !likely_channels.is_empty() {
                            Text { variant: g3_ui::TextVariant::Overline, "Likely" }
                            List { lines: ListLines::Inset,
                                for channel in likely_channels.iter().cloned() {
                                    SuggestionRow {
                                        key: "{channel.id}",
                                        other_groups: other_groups_of(&channel.id),
                                        picked,
                                        channel,
                                    }
                                }
                            }
                        }
                        if !maybe_channels.is_empty() {
                            Text { variant: g3_ui::TextVariant::Overline, "Maybe" }
                            List { lines: ListLines::Inset,
                                for channel in maybe_channels.iter().cloned() {
                                    SuggestionRow {
                                        key: "{channel.id}",
                                        other_groups: other_groups_of(&channel.id),
                                        picked,
                                        channel,
                                    }
                                }
                            }
                        }
                    }
                }
            }

            Modal {
                open: name_dialog_open,
                title: if is_renaming { "Rename group" } else { "New group" },
                actions: rsx! {
                    Button { fill: ButtonFill::Clear, onclick: move |_| name_dialog_open.set(false), "Cancel" }
                    Button {
                        disabled: group_name().trim().is_empty(),
                        onclick: move |_| {
                            let name = group_name().trim().to_string();
                            if name.is_empty() { return; }
                            match renaming.peek().clone() {
                                Some(group_id) => {
                                    if app_state.rename_subscription_group(&group_id, &name) {
                                        app_state.show_toast(format!("Renamed to {name}"), Color::Success);
                                    }
                                }
                                None => {
                                    app_state.create_subscription_group(name.clone(), Vec::new());
                                    app_state.show_toast(format!("Created {name}"), Color::Success);
                                }
                            }
                            group_name.set(String::new());
                            name_dialog_open.set(false);
                        },
                        if is_renaming { "Save" } else { "Create" }
                    }
                },
                Input {
                    label: "Group name",
                    value: group_name,
                    placeholder: "Documentaries",
                    autofocus: true,
                }
            }
        }
    }
}
