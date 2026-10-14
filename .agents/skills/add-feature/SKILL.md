---
name: add-feature
description: Add a feature to Tawny end to end — SurrealDB schema, an AppServerState method, a server function, cache invalidation, and a g3-ui screen. Use when asked to add a new entity, action, screen, or page backed by data.
---

# Add a feature

Build it in dependency order. Each step compiles before the next starts. The
long-form guide, with a worked example, is `docs/adding-a-feature.md`; a
compact real one is `set_in_playlist` (`src/api/library.rs`) with
`AppState::add_to_playlist` (`src/state.rs`).

## 0. Before writing anything

- Read `AGENTS.md` if you have not this session, and
  `docs/dioxus/patterns.md`.
- Read `docs/g3-ui.md` for the components you will need.
- If it touches YouTube data or playback, read `docs/youtube.md` or
  `docs/playback.md` first.

## 1. Schema — `database/schema/<table>.surql`

- `IF NOT EXISTS` on every definition; the server applies the files on start.
- An account's data carries `owner`, a record of `app_user`, and is keyed by
  it where ids repeat across accounts.
- Data moves go in `database/presync.surql` or `backfill.surql`.

## 2. Server method — `src/server/<area>.rs`

- A method on `AppServerState`, taking the owner from its caller.
- Values bound, never formatted into SurrealQL.
- YouTube calls go through `youtube.rs`/`ytdlp.rs` and fall back to cached
  data on failure.
- A server test against `mem://`.

## 3. Server function — `src/api/<area>.rs`

- `#[get]` or `#[post]` under `/api/v1/`, with `owner: crate::auth::Owner`.
- "Set to", not "toggle".
- Public only if a signed-out client must reach it, and then the pinned list
  in `auth/accounts.rs`'s tests changes too.

## 4. Cache invalidation — `src/data_change.rs`

- A new cached read goes under every `DataChange` whose mutations can change
  its answer.

## 5. Screen — `src/components/`

- `PageHeader` + `Content`; a new page is a `Route` variant with its
  `#[transition]` (`docs/navigation.md`).
- All hooks before any early return. Reads through `use_cached`; derived lists
  in `use_memo`; lists passed down as signals (`VideoGrid { videos }` takes a
  memo). Handlers a busy page passes down are `use_callback`s.
- Writes through `AppState`: `update_cached` at once, then
  `self.send(DataChange::.., api::..)`.
- `app_state.show_toast(..)` for feedback.

## 6. Finish

```bash
just check && just test && just lint-strict
```

Then use the `verify-in-browser` skill. Report what you checked, and anything
you could not.
