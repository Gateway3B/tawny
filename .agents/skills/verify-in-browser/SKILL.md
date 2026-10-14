---
name: verify-in-browser
description: Check a UI change in the running Tawny app — start or reuse the dev server and its sidecars, load it as a fresh guest at a phone viewport, exercise the flow, and read the server log and page for errors. Use after changing a screen, route, the player, or a server function the UI calls.
---

# Verify in the browser

Type-checking proves it compiles. This proves it works.

## 1. Services and server

1. `just db-up` starts SurrealDB, the yt-dlp sidecar and its PO-token
   provider. `docker compose ps` should show them healthy.
2. Reuse a dev server that is already answering (see `.Codex/launch.json`
   for its port). Otherwise start one. Stop only a server you started.
3. **Do not edit Rust while `dx serve` is running.** It tries to hot-patch and
   can leave a wasm that imports `env`; the page then never leaves "Loading".
   Stop the server, run
   `cargo clean -p tawny --target wasm32-unknown-unknown --profile wasm-dev`,
   delete `target/dx`, and start again.
4. Wait for "Build completed" in the output before loading the page.

## 2. Walk the flow

1. Set the viewport to **390×844** first.
2. Load `/`. A first visit becomes a new guest with the demo channels and
   playlists. Its first requests may log `401` before the cookie lands.
3. Exercise the change the way a user would. Useful paths: the feed's filters
   and groups; Search (type to filter, Enter to search online); a channel page
   and each of its tabs; a playlist, its filters and sorts; opening a video.
4. After a mutation, check the other screens that show the same data update
   without a reload. If one does not, a read is missing from
   `src/data_change.rs`.
5. Reload once: screens should paint from the client cache at once.
6. If layout changed, repeat at **1440×900**.
7. Scripted clicks: g3-ui's segmented controls are `role="radio"`; a video
   card's open button is its `.g3-card-action`. Coordinates can miss when the
   viewport is emulated; clicking the element through JS is reliable.

## 3. Read the evidence

- **The page**: an `App panicked!` overlay. Read its message; see
  `docs/troubleshooting.md`.
- **Server output**: `[500]`s, panics, extractor errors.
- **Network**: YouTube-backed reads are slow on a cold cache; give a channel
  page some seconds before calling it stuck.

## 4. Report

State what you exercised, at which viewports, and what the logs showed. Say
plainly what you could not verify (a native build, real playback on a device).
