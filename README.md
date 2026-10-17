# Tawny

Tawny is a calm, self-hosted YouTube client built with Dioxus and `g3-ui`. The same client targets Android, iOS, web, macOS, Windows, and Linux, with a SurrealDB-backed server that watches subscribed channels for new uploads.

Tawny is designed as a self-hosted replacement for a LibreTube plus Piped deployment. The Dioxus backend performs search, feeds, channels, video details, comments, captions, and stream orchestration directly against YouTube; its deployment-local yt-dlp and PO-token sidecars are included in this repository, and no public Piped instance is contacted.

This repository currently contains a working vertical slice:

- cached subscription feed with All, Videos, Shorts, and Live filters;
- real YouTube search for videos and channels with continuation paging, request coalescing, and a five-minute result cache;
- full channel pages with Videos, Shorts, Live, refresh, and continuation paging;
- subscription management with direct extraction, official RSS fallback, and YouTube WebSub callbacks;
- playlists, creation, detail views, and offline viewing of what you have opened;
- playlists, subscriptions, queue, history, and watch progress kept per account on the server, shown at once from a device cache (`g3-cache`) and changed with set-style mutations that are safe across devices;
- configurable swipe-left and swipe-right playlist destinations;
- channel pages, a persistent play queue, watch history, and a full video action sheet;
- a persistent player that expands on video pages and becomes a mini-player above navigation elsewhere, with one-tap speed controls;
- subscription groups with one-tap feed filters;
- direct YouTube video details, comments, captions, chapters, recommendations, and expiring playback sources;
- cached descriptions, chapter jumps, captions, comments, and related videos;
- same-origin ranged media/caption proxying, HLS/DASH manifest rewriting, adaptive quality, buffered seeking, and session retry;
- responsive mobile, web, and desktop navigation using `g3-ui`;
- server-side playback resolution for SABR, HLS, DASH, progressive streams, and per-video PO tokens.

A new account starts with the default playlists and follows no one, so its feed is empty until you follow a channel. Library actions show on the device at once and are then saved to SurrealDB; the screens refetch what the change affected. See [docs/architecture.md](docs/architecture.md#data-on-the-client).

## Quick start

You need [Rust](https://rustup.rs), [Docker](https://docker.com),
[Node 20+](https://nodejs.org), [lefthook](https://lefthook.dev), and:

```bash
cargo install cargo-binstall
cargo binstall dioxus-cli@0.7.9 just cargo-nextest
```

Then:

```bash
cp .env.template .env      # its host-facing URLs already match the Compose layout
just setup                 # npm install + git hooks
just db-up                 # SurrealDB, the yt-dlp sidecar and its PO-token provider
just dev                   # http://localhost:8080
```

The default Compose profile starts only the server's dependencies, so `dx
serve` owns the app build and port 8080. Nothing needs installing on the host
for yt-dlp. See [docs/docker.md](docs/docker.md).

## Commands

| Command | Does |
| --- | --- |
| `just dev` | Dev server (web). `just dev-android`, `dev-ios`, `dev-desktop` for the apps |
| `just check` | Type-check web, server, mobile and desktop |
| `just test` | Node and Rust tests, web and server |
| `just lint-strict` | Clippy with warnings as errors on every build, plus Biome |
| `just test-ui` | Playwright |
| `just format` | rustfmt + Biome |
| `just db-up` / `db-down` / `db-reset` | Local services |
| `just pre-push` | What the git hook runs before a push |

## Documentation

| | |
| --- | --- |
| [AGENTS.md](AGENTS.md) | The rules for working in this code, for people and coding agents |
| [docs/architecture.md](docs/architecture.md) | The system, data on the client, the server and SurrealDB |
| [docs/adding-a-feature.md](docs/adding-a-feature.md) | Server function → cache → screen |
| [docs/dioxus/](docs/dioxus/README.md) | Dioxus as used here, and the framework's own notes |
| [docs/g3-ui.md](docs/g3-ui.md), [docs/styling.md](docs/styling.md) | Components and theming |
| [docs/navigation.md](docs/navigation.md) | Routes and transitions |
| [docs/authentication.md](docs/authentication.md) | Guests, accounts and sessions |
| [docs/youtube.md](docs/youtube.md) | Search, channels, the feed, RSS and WebSub |
| [docs/playback.md](docs/playback.md) | Resolving, proxying and playing streams |
| [docs/mobile.md](docs/mobile.md) | Android, iOS and desktop |
| [docs/docker.md](docs/docker.md), [docs/deployment.md](docs/deployment.md) | Compose, images, Portainer, backups |
| [docs/troubleshooting.md](docs/troubleshooting.md) | Known failure modes |
| [database/README.md](database/README.md), [tests/README.md](tests/README.md) | Schema workflow, test suite |

Tawny is not affiliated with or endorsed by YouTube. YouTube trademarks belong
to their respective owners.
