# g3-ui reference

The components this app builds with, and their props, as of `g3-ui` 0.4.2 with
the `transitions` feature. Written so you (or your coding agent) can build a
screen without guessing. The full reference is
[docs.rs/g3-ui](https://docs.rs/g3-ui), and the
[playground](https://g3ui.g3tech.net/) shows each component live in both modes.

**g3-ui is where UI starts.** Before writing a `div` with classes, look for the
component below that already is that thing. It carries the iOS and Material
looks, dark mode, the desktop layout and ARIA.

---

## Rules that apply to every component

- **Optional props are `Option<T>`, and `rsx!` wraps them for you.** Write
  `title: "Lists"`, not `title: Some("Lists".to_string())`. String props accept
  `&str`.
- **Stateful components take a `Signal` and write it themselves.** Pass
  `value: handle`, not a value plus an `onchange` that sets it. `onchange` is
  for a side effect on top (saving). Leave the signal out and the component
  keeps its own state.
- **Every component takes `class`**, and most take **`mode`** to force iOS or
  Material on one instance. You rarely need either.
- **Spacing is built in.** Use `Stack { gap: Space::.. }` for vertical rhythm
  rather than a wrapper `div` with margins.
- Names are unprefixed (`Button`, `Card`, `Table`).
- Slots are `start` / `end` everywhere (buttons, items, cards, the header).
- `to: Route` on `Button`, `Card`, `Item` and `NavItem` makes it a link. Here,
  navigate with `onclick` and `animated_navigate` instead, so the transition
  runs (see [navigation.md](navigation.md)).

---

## Page structure

| Component | Key props | Notes |
| --- | --- | --- |
| `AppWrapper` | `theme`, `mode` | Once, in `ThemedShell` (`src/app.rs`). Theme as CSS variables, the stylesheet, the host for toasts and alerts, and the transition overlay region. |
| `TabLayout` | `route_transition_base: bool` | The frame. `AppShell` (`components/shell.rs`) is already assembled; screens never render one. |
| `AdaptiveNav` / `NavItem` | `compact: AdaptiveNavCompact`; `label`, `icon`, `selected`, `group`, `onclick` | Bottom tabs on a phone, a rail from 48rem. Lives in `AppShell`. |
| `Header` | `title`, `start`, `end`, `toolbar`, `title_content`, `title_end` | Every screen's first element. Wrapped by the app's `PageHeader`, which fills `start` (Back or the brand), `end` (Queue, History, Settings) and takes the page's `toolbar` (under the title on a phone, inline from 64rem). |
| `Content` | `width: ContentWidth::{Full, Readable, Wide}`, `padding`, `on_refresh` + `refreshing`, `fab` | The scroll container. `Readable` for forms and text, `Wide` for grids. `on_refresh` + `refreshing` give pull to refresh (feed, channel, playlist). Wraps its children in suspense and error boundaries. |

```rust
rsx! {
    PageHeader { title: "Playlists", toolbar: rsx! { FeedFilterSegments { value: app_state.feed_filter } } }
    Content { width: ContentWidth::Readable,
        Stack { /* the screen */ }
    }
}
```

## Layout and text

| Component | Key props | Notes |
| --- | --- | --- |
| `Stack` | `horizontal`, `gap: Space::{None, Xs, Sm, Md, Lg, Xl}`, `align: StackAlign`, `justify`, `wrap` | The default way to lay out children. Default gap is `Md`. |
| `Grid` | `columns`, `wide_columns`, `gap`, `wide_gap` | Columns that change at the 48rem shell width. `VideoGrid` sets 1 column on a phone and 4 wide (2 and 6 for Shorts). |
| `Text` | `variant: TextVariant::{Title, Heading, Body, Caption, Label, Overline}`, `tone: TextTone::{Primary, Secondary, Tertiary}`, `color: Color`, `truncate` | All copy. Supporting text is `Text { tone: TextTone::Secondary, .. }`; section labels `Text { variant: TextVariant::Overline, .. }`. |
| `Divider`, `ListHeader` | | Separators and headed groups inside a `List`. |

## Content

| Component | Key props | Notes |
| --- | --- | --- |
| `Card` | `title`, `subtitle`, `start`, `end`, `media`, `variant: CardVariant::{Raised, Flat, Filled}`, `onclick`, `selected` | A block of related content or a form. Playlist cards; video cards are the app's own `VideoCard`. |
| `List` | `variant: ListVariant::{EdgeToEdge, Raised, Flat, Filled}`, `lines: ListLines::{Full, Inset, None}` | A group of `Item`s. Raised + inset lines is the settings look. |
| `Item` | `label`, `description`, `overline`, `metadata`, `start`, `end`, `onclick`, `detail: ItemDetail`, `selected`, `wrap` | A row. `detail: ItemDetail::Show` draws the chevron for a row that opens something. |
| `EmptyState` | `title` (required), `icon`, `action`, `color` | Nothing to show yet, and failures (`color: Color::Danger`, with a Try again `action`). |
| `Badge`, `Chip` | `color`; `selected`, `onclick` | Counts, the LIVE marker; chips are the feed's length and group filters. |
| `Avatar` | `name` (required), `src`, `size` | Channels. Initials come from `name`. |
| `Shelf` | `title`, `end`, `snap`, `gap` | A horizontal row: the feed's filter strip, Explore's matching channels. |

## Actions and forms

| Component | Key props | Notes |
| --- | --- | --- |
| `Button` | `fill: ButtonFill::{Solid, Outline, Clear}`, `color: Color::{Accent, Neutral, Success, Warning, Danger}`, `size: ButtonSize`, `expand: ButtonExpand::{Block, Full}`, `loading`, `disabled`, `start`, `end`, `aria_label`, `onclick` | `loading: busy()` while a request runs. Icon-only buttons need `aria_label`. |
| `Input` | `label`, `value: Signal<String>`, `input_type: InputType`, `placeholder`, `helper`, `error`, `autocomplete`, `maxlength`, `debounce_ms`, `onchange` | One field with its label, helper and error. |
| `Searchbar` | `value`, `placeholder`, `debounce_ms`, `onchange`, `on_submit`, `end` | Explore's search; `on_submit` runs the remote search. |
| `SegmentGroup` / `SegmentButton` | `value: Signal<T>`, `label`, `onchange`, `scrollable`; `value: T` | Generic over the value, so options are enums (`FeedFilter`, `ExploreFilter`, `SubscriptionContent`). |
| `Toggle`, `Checkbox` | `checked: Signal<bool>`, `label`, `helper`, `onchange` | Settings switches. |
| `Select`, `RadioGroup` / `Radio` | `value`, `options` / children, `label` | Generic over the value. |

## Overlays and feedback

| Component | Key props | Notes |
| --- | --- | --- |
| `use_toast()` | `.success(msg)`, `.error(msg)`, `.show(..)` | Feedback after an action. `AppWrapper` hosts them; never mount a `Toast` yourself. Here, call `app_state.show_toast(..)`: `ToasterBridge` in `shell.rs` hands it to g3-ui. |
| `use_alert()` | `.confirm(title, message).await` | A yes/no question from a handler. |
| `BottomSheet` | `open: Signal<bool>` (required), `title`, `detents`, `on_dismiss`, `backdrop` | Short flows that are not their own route: the video action sheet, the playlist picker. A floating panel from 48rem. |
| `ConfirmModal` | `open`, `title`, `message`, `confirm_label`, `destructive`, `on_confirm` | Destructive confirmations declared in markup (deleting a playlist). |
| `Spinner` | `center`, `size`, `label` | Loading, when there is no skeleton for the shape. |

A routed sheet (a `Route` with `layer = sheet`) is a whole page,
declared outside the navbar layouts; a `BottomSheet` is an overlay inside a page. Use a route when the
sheet should survive a refresh or be linkable.

---

## The app's own components

Built on g3-ui; reuse them rather than rebuilding a variant:

| Component | For |
| --- | --- |
| `PageHeader` (`shell.rs`) | Every page's header: Back or the brand, title, toolbar, Queue/History/Settings |
| `VideoGrid`, `VideoGridSkeleton`, `VideoCard` (`video_card.rs`) | Videos in a grid, with swipe actions and the action sheet |
| `FeedFilterSegments` (`feed.rs`) | All / Videos / Shorts / Live, for the feed and a channel |
| `PersistentPlayer` (`player.rs`) | The one media element, full-width on a video and a floating mini player elsewhere |
| `AppOverlays` (`overlays.rs`) | The video action sheet, Save to playlist, New playlist and Share sheets |

## Upgrading g3-ui

The 0.3 → 0.4 renames (`G3Body` → `Content`, `ButtonStyle` → `ButtonFill`,
`--color-*` → `--g3-color-*` ..) are in the crate's CHANGELOG. When a new
version lands, read its CHANGELOG, bump the version, and let `just check`
list the call sites.
