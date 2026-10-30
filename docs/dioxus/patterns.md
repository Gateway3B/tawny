# Dioxus patterns for g3 apps

How the g3 apps use Dioxus 0.7, and the mistakes that cost us time. The rest
of `docs/dioxus/` is the framework's own architecture notes, copied from
upstream; this file is ours. When the two disagree about how *this* app should
be written, this file wins.

Pinned: `dioxus`, `dioxus-fullstack` and `manganis` `=0.7.9`, matching the `dx`
CLI. Your training data probably describes 0.5 or 0.6; server functions,
assets, signals and props all changed since.

---

## Components and props

### Props that feed hooks: `ReadSignal<T>`

A plain prop is a snapshot. A hook that reads it (`use_effect`, `use_memo`,
`use_resource`, `use_cached`) runs with the value from the first render and
never sees a new one. The old fix was `use_reactive!`; the better one is to
type the prop as a `ReadSignal`:

```rust
// Before: the effect only sees the stored preference through use_reactive!.
#[component]
fn ChannelHero(channel: Channel) -> Element {
    let stored = app_state.with_viewer(|v| v.follows(&channel.id).map(|f| f.subscription_content));
    let mut content = use_signal(|| stored);
    use_effect(use_reactive!(|stored| content.set(stored)));
    ..
}

// After: the parent still writes `ChannelHero { channel }`.
#[component]
fn ChannelHero(channel: ReadSignal<Channel>) -> Element {
    let stored = use_memo(move || {
        app_state.with_viewer(|v| v.follows(&channel.read().id).map(|f| f.subscription_content))
    });
    let mut content = use_signal(|| *stored.peek());
    use_effect(move || content.set(stored()));
    ..
}
```

- The parent passes a plain `T`; `#[component]` wraps it. The child owns one
  signal for its whole life and the new value is written into it, marked
  dirty only when it changed (`PartialEq`), so equal re-renders cost nothing.
- Hooks and effects that read it subscribe like any signal. No
  `use_reactive!`, no dependency tuple to keep in sync.
- A "mirror the prop into a local signal" effect is almost always a
  `ReadSignal` prop plus a `use_memo`, or the local signal is not needed at
  all. It is still needed for a g3-ui control that writes its value
  (`SegmentGroup` takes a `Signal`), as above: then the effect follows a memo.
- Use it for large values too: `VideoGrid` takes `videos:
  ReadSignal<Vec<Video>>`, reads it by reference, and re-renders only when the
  list changes.
- Keep plain props for values only used while rendering (a label, a flag read
  once in `rsx!`).

### Props the child writes: `Signal<T>`

A child that changes the parent's state takes the parent's `Signal` (g3-ui's
stateful components do: `SegmentGroup { value: app_state.feed_filter }`). Do
not pass a value plus an `onchange` that sets it.

### Handlers: `use_callback` for what a busy page passes down

`EventHandler<T>` / `Callback<T>` props are compared by identity, and when a
component re-renders, the macro re-points the child's old handler at the new
one (`Callback::__point_to`). A closure written in `rsx!` is a new handler
every render. Usually that is fine. On the channel page it was not: every tab
change panicked inside `__point_to` ("RefCell already borrowed", or with
generational-box's `debug_borrows`, a `ValueDroppedError` for an
`Option<Callback>`), because the pull-to-refresh and load-more closures were
rebuilt each render while the page also re-rendered for the tab.

The fix, and the rule: a handler a page passes as a prop, on a page that
re-renders for reasons of its own, is a `use_callback`, made once above any
early return, that reads what it needs when it runs:

```rust
let load_more = use_callback(move |_: ()| {
    let tab = media_tab(*tab.peek());              // read at call time
    let Some(token) = next_page_for(tab) else { return };
    page_loading.set(true);
    spawn(async move { /* fetch the page, extend the list */ });
});
rsx! { InfiniteScroll { on_load: load_more, .. } }
```

Call handlers with `.call(value)`.

### Component boundaries follow the data

A component re-renders when anything it reads in its body changes, and a
render re-runs everything in it: every clone, every `format!`, every derived
list. So split a screen along what each part reads, not along how it looks:

- **State that changes often gets its own component.** The search box's text
  re-renders the box; the results are a memo, so a keystroke does not rebuild
  the grid unless the matches changed.
- **A read used by one part lives in that part.** Each page of the feed
  (`FeedPageView`) makes its own `use_cached` read, so "load more" appends a
  page without redrawing the ones above.
- **A read used by several parts is a memo in the parent**, passed down as a
  `ReadSignal`.
- **Never call `use_cached` for the same key in two components that mount
  together.** g3-cache does not share an in-flight fetch between hooks.

### Clones: pay on a change or a tap, never on a render

`Signal`, `Memo`, `ReadSignal`, `Resource`, `Cached`, `Callback`, `Store` and
`AppState` are `Copy` handles. Code that clones data on every render almost
always has a handle it could hold instead:

1. **Borrow in render; don't clone out of a read.** A read guard can live for
   the whole body, as long as nothing writes that signal meanwhile.
2. **Borrow the field, not the struct.** `app_state.settings.read().theater_mode`
   instead of `app_state.settings()` (which clones every setting to read one).
3. **Shared data is a memo, passed as a `ReadSignal`.** Explore works its
   page out once per change and hands the grid a view into it without a copy:

   ```rust
   let shown = use_memo(move || paged(..));          // Shown { videos, channels, .. }
   rsx! { VideoGrid { videos: shown.map(|s| &s.videos) } }
   ```

   `Memo<T>`, `Signal<T>` and a mapped signal all convert to a `ReadSignal`
   prop without copying the value. A `ReadSignal` prop does **not** save a
   clone when the parent has only a plain value.
4. **Collections change in memos.** The channel page's list for a tab (the
   answer's videos plus the extra pages loaded since) is a memo over the
   cached read, not rebuilt from a clone of the whole `ChannelDetails` each
   render.
5. **Closures that capture only `Copy` handles are `Copy`.** Read the id
   inside the closure instead of capturing clones.
6. **Never call a signal to inspect a collection.** `videos().is_empty()`
   clones the whole `Vec`; write `videos.read().is_empty()`.

---

## State

### Hooks

- **Every `use_*` runs every render, in the same order.** All hooks at the top;
  branch on their results after. A hook after an early `return` is the same
  bug even if it happens to work: the persistent player synced its metadata
  from a `use_effect` below `let Some(video) = active_video else { return }`,
  and because `use_effect` keeps its first closure, it sent the first video's
  id and autoplay setting for as long as the player lived. It now sits above
  the return and reads the active video through a memo of `(id, audio_only,
  is_short)`, so it follows the current video without rerunning on progress
  ticks.
- **Never hold a signal borrow across `.await`.** Copy the value out first.
  `clippy.toml` lints `await_holding_invalid_type` for it. That includes a
  temporary: `save(id.peek().clone()).await` holds the guard to the end of the
  statement.
- Read with `peek()` when you want the value without subscribing: in event
  handlers, and in an effect that writes the thing it would otherwise read.
- **An effect depends on exactly what it reads.** The feed resets to one page
  with `use_effect(move || { let _ = query.read(); pages.set(1); })`.
- **Side effects go in effects, not in the component body. Writing a signal is
  a side effect too**: derive with a memo, or write from an effect.

### Derived values: `use_memo`

A value computed from signals (a filtered list, a query, a label) goes in a
`use_memo`. It recomputes when an input changes and notifies readers only when
its result changes. The feed's `FeedQuery` is a memo over the filters and
settings, shared by every page of the feed.

### Stores: nested state edited in parts

`Signal<Struct>` notifies every reader when any field changes. A **store**
tracks each field (and each `Vec`/`HashMap` item) separately (`#[derive(Store)]`,
`use_store`, one lens method per field). Use one when a struct is edited field
by field and read in parts. `AppSettings` would fit, but it persists to local
storage as a whole through `use_persistent_signal`; until that moves to a
store, read single fields by borrowing (above).

### App-wide state

`AppState` is a `Copy` struct of signals provided once above the router. It
holds what spans screens: the viewer, settings, the player's state, overlays.
Per-screen state (a filter, a tab) goes in the route or the screen.

---

## Data

### Reads: `use_cached`, not `use_resource`

```rust
let details = use_cached(get_channel_details, (id.clone(),));   // last answer at once, refetched behind
match &*details.read() {
    None => rsx! { Spinner { center: true } },
    Some(Err(_)) => rsx! { EmptyState { title: "Channel unavailable", .. } },
    Some(Ok(details)) => rsx! { .. },
}
```

- Keyed by the function and its arguments; a changed argument is a new read.
- A `Cached` is `Copy` but has no `PartialEq`, so it cannot be a prop. Pass a
  memo over it instead.
- `use_resource` is for reads that must not be cached, such as resolving a
  playback session.

### Writes: show, send, reconcile

`AppState::add_to_playlist` is the shape of every write:

```rust
self.edit_viewer(|viewer| /* insert the id */);                           // the viewer, at once
update_cached(get_playlist, (playlist_id.to_string(),), |c| /* insert */); // the playlist, at once
self.send(DataChange::Playlists, api::set_in_playlist(id, video, true));  // "set to", then invalidate
```

Mutations say what the viewer saw, never "toggle", so a device acting on
stale state cannot undo another device's change.

### Server functions

```rust
#[post("/api/v1/playlists/membership", state: State<AppServerState>, owner: crate::auth::Owner)]
pub async fn set_in_playlist(playlist_id: String, video_id: String, member: bool) -> Result<bool> { .. }
```

- The account is the `Owner` extractor, from the session, never an argument.
- **A server function called during SSR runs without middleware**: the auth
  guard never sees it.
- Bind values with `.bind(..)`; never `format!` a value into SurrealQL.

---

## Rendering

### Hydration

The server renders HTML and the client hydrates it; the two trees must match.
**Gate calls, never markup.** A console `Error deserializing data` or a
handler that never fires is a hydration mismatch. Head elements are part of
the tree too: render them on every build.

### Routing

Every page is a `Route` variant. Navigate with `animated_navigate`, not the
navigator directly (see `docs/navigation.md`).

---

## Tooling

- **`dx` pins:** `dioxus*` and `manganis` must match the installed `dx`.
- **Do not set `rust-lld` as the linker on Windows**; `dx` builds break.
- **Stop `dx serve` before editing Rust.** It tries to hot-patch fullstack
  apps and can leave a wasm whose JS imports `env`; the page then never
  leaves "Loading". Clean with
  `cargo clean -p tawny --target wasm32-unknown-unknown --profile wasm-dev` and
  delete `target/dx`.
- **`debug = 0` in `[profile.wasm-dev]`** keeps rebuilds fast, and dx strips
  names, so a wasm panic's stack is `wasm-function[N]`. To find a signal
  borrow conflict, add
  `generational-box = { version = "=0.7.10", features = ["debug_borrows"] }`
  temporarily: the panic then names where the value was created.
- **A hidden tab never runs `requestAnimationFrame`.** Code that waits a frame
  stalls in a background tab or a hidden preview pane.
- **Do not run `dx fmt` (0.7.9) on this code.** It has dropped `);` and
  duplicated statements inside closures in `rsx!`. Format with `cargo fmt`;
  `.vscode/settings.json` turns the Dioxus extension's formatter off.
- **IndexedDB requests must finish** even if the future awaiting them is
  dropped. g3-cache runs them on their own task.

---

## Where the framework is documented

- `docs/dioxus/framework-agents.md`: Dioxus's own agent guide.
- `docs/dioxus/architecture/`: `04-SIGNALS.md` (signals, memos, stores),
  `05-FULLSTACK.md` (server functions, SSR, hydration), `09-ROUTER.md`,
  `07-HOTRELOAD.md`, `08-ASSETS.md`.
- [dioxuslabs.com/learn/0.7](https://dioxuslabs.com/learn/0.7/).
