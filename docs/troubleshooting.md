# Troubleshooting

Problems people hit in this app, roughly in the order they hit them. The
Dioxus-specific ones are explained further in
[dioxus/patterns.md](dioxus/patterns.md).

---

## Setup

**`dx and dioxus versions are incompatible`**, or the page never hydrates
The CLI must be built from the same Dioxus fork as the libraries
(`=0.8.0-alpha.3` from Gateway3B/dioxus): `cargo install --git
https://github.com/Gateway3B/dioxus --branch main dioxus-cli --locked --force`.

**`Failed to connect to the database`**
The services are not running, or `.env` points elsewhere. `just db-up`, then
check `SURREALDB_HOST`/`SURREALDB_PORT` in `.env` (Compose publishes
SurrealDB on `127.0.0.1:8001`).

**Search, channels or playback fail with extractor errors**
The yt-dlp sidecar or its PO-token provider is down. `docker compose ps`
should show `extractor` and `pot-provider` healthy; the extractor's health
check covers both. See [docker.md](docker.md).

---

## Building

**Works in `cargo check`, fails in CI**
`cargo check` builds only the web feature set. `just check` builds all four.

**Clippy: `this ... is held across an await point` (on a signal)**
A signal borrow is held across `.await`. Copy the value into a local first.

**`dx` builds break after setting `rust-lld` as the linker**
Do not set it on Windows.

---

## Running

**The page stays on "Loading" after an edit, and the console says
`Failed to resolve module specifier "env"`**
`dx serve` tried to hot-patch the running app and left a wasm build that
imports `env`. Stop the server, then:

```bash
cargo clean -p tawny --target wasm32-unknown-unknown --profile wasm-dev
```

delete `target/dx`, and start it again. Stop the server before editing Rust
to avoid it.

**`App panicked!` with `RefCell already borrowed` or a `ValueDroppedError`
from `Callback::__point_to`**
A handler passed as a prop was a closure rebuilt on every render of a page
that re-renders often. Make it a `use_callback` above any early return, reading
what it needs when it runs (see patterns.md, "Handlers"). To find which one,
add `generational-box = { version = "=0.8.0-alpha.3", features = ["debug_borrows"] }`
to `Cargo.toml` for a build, patched from the fork like the other Dioxus
crates: the panic then names where the value was made.

**A new guest's first requests show `401`**
They run before the session cookie lands, and are retried once it exists.
Anything that keeps failing with `401` is a server function missing from the
session's reach, not this.

**A screen shows stale data after a change made elsewhere**
The read is missing from the mutation's `DataChange` in `src/data_change.rs`.

**Every launch starts with an empty cache**
`g3_cache::set_cache_owner(None)` ran before the session was known.

**Playback stalls or fails**
See [playback.md](playback.md): the player reports the transport's own error.
Check the sidecar first, then the server log for the resolve.

---

## Mobile and desktop

**The app cannot reach the server on a device**
`localhost` on a phone is the phone. The Android emulator uses
`http://127.0.0.1:8080` through `adb reverse`; a physical device needs
`SERVER_URL` set to a reachable address. See [mobile.md](mobile.md).

**Connecting to a server fails**
Saving a server address reconnects in place, cancelling the previous attempt
and clearing the previous account's cached library. An attempt times out after
20 seconds so the address can be corrected and retried without restarting.
