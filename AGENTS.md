# AGENTS.md

Instructions for coding agents working in this repository (Claude Code, Codex,
Cursor, Copilot, and anything else that reads `AGENTS.md`). Useful for people
too. Follow these over your defaults.

## What this is

Tawny is an independent personal project for individuals running their own
home-lab server. Keep product UI and deployment independent of company branding,
shared company services, and regulatory compliance workflows.

**Tawny** is a calm, self-hosted YouTube client: a subscription feed, search,
channels, playlists, a queue and history, and a persistent player, with no
Piped instance in between. The server talks to YouTube directly (rustypipe,
official RSS and WebSub) and resolves playable streams through a yt-dlp
sidecar. It is one Rust codebase for web (WASM), Android, iOS and desktop,
built on the **g3 stack** (the same shape as
[g3-stack](https://github.com/g3techhq/g3-stack)).

| Piece | Version | Reference |
| --- | --- | --- |
| Dioxus (fullstack, router) | 0.8.0-alpha.3 from the Gateway3B fork, pinned `=` | [docs/dioxus/](docs/dioxus/README.md), [dioxuslabs.com/learn/0.7](https://dioxuslabs.com/learn/0.7/) |
| g3-ui (components, theme) | 0.6 | [docs/g3-ui.md](docs/g3-ui.md) |
| g3-route-transitions (navigation) | 0.5 | [docs/navigation.md](docs/navigation.md) |
| g3-native-plugins (media, storage, back button) | 0.4 | [docs/mobile.md](docs/mobile.md) |
| g3-auth (session guard, `#[public]`, record ids) | 0.2 | [docs/authentication.md](docs/authentication.md) |
| g3-cache (client and server caches) | 0.2 | [docs/architecture.md](docs/architecture.md#data-on-the-client) |
| SurrealDB + SurrealKit | 3.2.4 | [database/README.md](database/README.md) |
| rustypipe (YouTube extraction) | 0.11 | [docs/youtube.md](docs/youtube.md) |
| yt-dlp sidecar, bgutil PO tokens | `docker/extractor` | [docs/playback.md](docs/playback.md) |
| Shaka Player (DASH/HLS in the page) | 4.16 | [docs/playback.md](docs/playback.md) |
| Tailwind (layout utilities only) | v4, run by `dx` | [docs/styling.md](docs/styling.md) |

**Your training data is probably wrong about these.** Dioxus 0.7 changed server
functions, assets and signals from earlier versions, the fork tracks upstream
`main` past 0.7 (`dioxus-core` is now `dioxus-vdom`; use `dioxus::core::..`),
and the g3 crates are new.
Read the linked doc before writing code against a library, not after it fails
to compile. Prefer the patterns already in `src/` over what you remember.

### Domain terms

- **Channel, video**: the shared catalog, normalized from YouTube into the
  `channel` and `video` tables. A video can be a **Short** or **live**; the
  feed filters on that (`FeedFilter`).
- **Follow** (a subscription): a `user_subscription` edge from an account to a
  channel, with a **`SubscriptionContent`** (videos, Shorts, or both).
  **Groups** sort follows into feed filters.
- **Feed**: the followed channels' uploads, 24 at a time
  (`get_feed_page(query, page)`), filtered in SurrealDB.
- **Playlist**: an account's ordered list (`playlist` + `playlist_item`). The
  **queue** is a list of video ids and playlist runs (`playlist:<id>`
  entries); **history** is the last 250 videos opened.
- **Viewer**: one account's own state, read by almost every screen
  (`get_viewer`): follows, playlists as ids, groups, queue, history.
- **Settings** (`AppSettings`): per device, in local storage, not the server.
- **Playback session**: what the server resolves for one video
  (`PlaybackSession` of `PlaybackSource`s), served to the player through a
  same-origin proxy with short-lived tokens.
- **Extractor sidecar**: the yt-dlp service in `docker/extractor`, with its
  bgutil PO-token provider. **WebSub**: YouTube's push notifications for new
  uploads.

## Map

```
src/app.rs              Route enum + transitions + theme. Read first.
src/state.rs            AppState: the viewer, settings, the player, overlays
src/data_change.rs      What each kind of mutation makes stale in the client cache
src/main.rs             Server: DB, layers, the session-less routes, router
src/server_url.rs       The backend address, chosen before launch (native builds)
src/health.rs           Container probe, outside the session layers
src/auth/               Accounts and sessions (g3-auth), and the client's session
src/db/                 Connection and schema sync
src/api/                Server functions by area: catalog, playback, feed, account, library
src/server/             AppServerState, the server's engine: ytdlp, youtube (normalizing),
                        resolve, proxy, segments, store, browse, feed, websub, viewer
src/models/             Types shared by client and server
src/components/         Screens and the shell; player.rs is the persistent player
src/cache.rs            use_persistent_signal (per-device settings)
src/library_sync.rs     Refetching what another device changed; catching up a paused player
src/playback_resolver.rs Where the player gets a session: the phone (Android) or the server
src/local_streams.rs    Android: resolving playback on the phone (yt-dlp in the app), see docs/on-device-playback.md
src/local_relay.rs      Android: 127.0.0.1 relay that hands phone-resolved streams to the player
src/subscriptions_io.rs Importing and exporting subscription lists
assets/                 tawny_transport.js and tawny_player_controls.js (the player's JS side)
database/schema/        One .surql file per table (desired state, not migrations)
docker/extractor/       The yt-dlp sidecar image
android/localstreams/   The Android module behind local_streams: Chaquopy Python, yt-dlp, V8 sandbox
docs/                   Human and agent documentation; docs/dioxus/ for the framework
tests/                  Node tests for the player JS; tests/ui for Playwright
```

`src/api/library.rs` (`set_in_playlist`) with the playlist screens in
`src/components/playlists.rs` is a compact example of a change end to end: a
"set to" server function, an optimistic `update_cached`, and a `DataChange`.

## Commands

```bash
just check        # web + server + mobile + desktop type-check. `cargo check` alone is NOT enough
just test         # Node + Rust tests, web + server
just lint         # clippy (web, server) + biome
just lint-strict  # clippy on all four builds with warnings as errors, as CI runs it
just format       # rustfmt + biome
just db-up        # SurrealDB and the extractor sidecars in Docker
just dev          # dev server, http://localhost:8080
just test-ui      # Playwright
```

## Definition of done

Before you say a change is finished:

1. `just check` passes, on all four feature sets.
2. `just test` passes.
3. `just lint-strict` is clean.
4. For UI changes, you looked at it in a browser at **390×844**, and at
   **1440×900** if layout changed, and read the `dx serve` output for server
   errors. See "Verifying in a browser" below.

If you could not do one of these, say which and why.

---

## Rules

### UI: g3-ui first

- **Build screens from g3-ui components.** A page is `PageHeader` (the app's
  wrapper over g3-ui's `Header`) + `Content`, a block is a `Card`, a row is an
  `Item`, a vertical flow is a `Stack`, copy is `Text { variant, tone }`, an
  empty or failed state is an `EmptyState`. Grids of videos are `VideoGrid`.
  The catalog is [docs/g3-ui.md](docs/g3-ui.md).
- **Do not add CSS classes or rules** for things a component or prop already
  does. Tailwind is for layout only.
- **No color literals** outside `tawny_theme` in `src/app.rs`.
- **No viewport media queries.** Use `@container g3-app-shell (width >= 48rem)`.
- Stateful g3-ui components take an owned `Signal` and write it.
- Feedback goes through `app_state.show_toast(..)`.
- Icon-only buttons need `aria_label`.

### Components, props and hooks

The full guide, with examples from this code, is
[docs/dioxus/patterns.md](docs/dioxus/patterns.md).

- **Hooks run unconditionally, in the same order, every render.** All `use_*`
  calls before any early `return`, the player's included.
- **Never hold a signal borrow across `.await`.** Copy the value first.
  `clippy.toml` lints for it.
- **A prop read inside a hook is a `ReadSignal<T>`.** No `use_reactive!`.
- **Derived values are `use_memo`**, and lists are copied when their data
  changes, not on every render (`VideoGrid` takes a `ReadSignal<Vec<Video>>`).
- **Never write a signal while rendering.** Derive with `use_memo`; write from
  `use_effect` or a handler.
- **Handlers a page passes as props are `use_callback`s** when the page
  re-renders often, reading what they need when they run. A closure rebuilt
  on every render and passed as an optional handler has panicked in
  `Callback::__point_to` (see patterns.md).
- **Borrow the one setting you need**: `app_state.settings.read().theater_mode`,
  not `app_state.settings()`, which clones the whole struct.
- Format with `cargo fmt`. Do not run `dx fmt`: 0.7.9 corrupted closures
  inside `rsx!`, and the fork's has not been checked.

### Reading and changing data

- **Screens read through `use_cached(server_fn, (args,))`**, which shows the
  last answer at once (IndexedDB on web, redb on native) and refetches behind
  it. See [docs/architecture.md](docs/architecture.md#data-on-the-client).
- **A change is shown at once, then made on the server, then reconciled**:
  `update_cached` the reads it touches, call the "set to" server function,
  then `data_change::invalidate(..)` for what it could have affected.
- **Mutations say "set to", not "toggle"** (`set_in_playlist(id, video,
  true)`), so a stale device cannot undo another's change. The queue is sent
  whole.
- The client never holds the catalog. A screen asks for what it shows.

### Server

- Server functions live in `src/api/`, one file per area, and call methods on
  `AppServerState` (`src/server/`). Paths are under `/api/v1/`.
- **The account comes from the session** (`Owner`), never from an argument.
- **Bind values** in SurrealQL. Never `format!` a value into a query.
- **YouTube is volatile.** Extraction goes through `src/server/youtube.rs`
  (normalizing rustypipe) and `ytdlp.rs` (the sidecar); a failure falls back
  to the stale server cache, then to what the device last saw, never an error
  page. Stream URLs and PO tokens are never persisted.
- Guarded by default: `#[g3_auth::public]` only for guest creation, sign-in
  and sign-out, and a test pins that list. Health, the playback proxy and the
  WebSub callback are mounted outside the session layers on purpose.
- Server caches are `g3_cache::ServerCache` statics, bounded and expiring.

### SurrealDB

- Schema changes go in `database/schema/<table>.surql` with `IF NOT EXISTS`;
  the server applies them on start. `database/presync.surql` and
  `backfill.surql` run around the sync for data moves.
- Tests use an in-memory database (`kv-mem`).
- **Record ids are `g3_auth::RecordId`/`RecordIdKey`, never
  `surrealdb_types`'s.** On the server they are the same type; on the client
  g3-auth's copy keeps SurrealDB's `Value` (regex, geometry) out of the wasm,
  about a megabyte. A test in `src/models/mod.rs` fails on the old import.

### Cross-platform

- `src/` compiles four ways: web, server, mobile and desktop. Server-only code
  sits behind `#[cfg(feature = "server")]`.
- **Gate calls, never markup.** The server renders HTML the client hydrates.
- The player is JS as much as Rust: `assets/tawny_transport.js` (Shaka, the
  proxy, retries) and `tawny_player_controls.js`, with Node tests in `tests/`.
  Keep the Rust ↔ JS message shapes in step (`sync_player_metadata`,
  `attach_player_session`).

### Code style

- Comment **why**, not what. The existing comments set the tone and density.
- Prefer a Rust unit test over a browser test; server tests run against
  `mem://`. One test talks to YouTube and is `#[ignore]`d.
- Commits follow Conventional Commits; lefthook checks them.

---

## Verifying in a browser

Do not start `dx serve` commands on your own. Use the provided server on port
8080 for verification. Stop the watcher before editing Rust to avoid broken hot
updates, and have the server restarted after the edits are ready.

```bash
just db-up
just dev                  # or reuse a server already on :8080
```

- Check `http://127.0.0.1:8080` first; reuse a running server rather than
  starting a second one. If you start one, stop only that one afterward.
- Non-interactive start: `dx serve --web --addr 127.0.0.1 --port 8080 --open false --interactive false`.
- Set the viewport to **390×844** before interacting.
- A first visit signs in as a new guest automatically, with the demo channels
  and playlists. A new guest's first requests can log `401` before the cookie
  lands; they retry.
- **Stop the dev server before editing Rust.** dx tries to hot-patch the
  running app and can leave a wasm that imports `env`; the page then stays on
  "Loading". If that happens, run
  `cargo clean -p tawny --target wasm32-unknown-unknown --profile wasm-dev`,
  delete `target/dx`, and start again.
- Read the `dx serve` output for `[500]`s and panics, and the page for the
  `App panicked!` overlay.

## Where to look when stuck

- [docs/troubleshooting.md](docs/troubleshooting.md): known failure modes and fixes
- [docs/dioxus/patterns.md](docs/dioxus/patterns.md): Dioxus as used here, and its sharp edges
- [docs/architecture.md](docs/architecture.md): the system, the caches, the server
- [docs/playback.md](docs/playback.md), [docs/youtube.md](docs/youtube.md): the domain engines
- [docs/on-device-playback.md](docs/on-device-playback.md): Android resolving and streaming without the server
- [docs/deployment.md](docs/deployment.md), [docs/docker.md](docs/docker.md): images and hosting
- The g3 crates' READMEs and source:
  [g3-ui](https://github.com/g3techhq/g3-ui),
  [g3-route-transitions](https://github.com/g3techhq/g3-route-transitions),
  [g3-native-plugins](https://github.com/g3techhq/g3-native-plugins),
  [g3-auth](https://github.com/g3techhq/g3-auth),
  [g3-cache](https://github.com/g3techhq/g3-cache)
