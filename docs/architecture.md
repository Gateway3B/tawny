# Tawny architecture

## Shape of the system

```text
YouTube Innertube/RSS ──► source normalization
          │                       │
YouTube WebSub ──────────► SurrealDB/SurrealKV
          │                       │
          └──► yt-dlp sidecar ◄── PO-token provider
                                  │
                   Dioxus server functions (g3-auth guard)
                                  │
                Tawny clients + g3-cache (IndexedDB / redb)
                                  │
                       playback proxy/embed fallback
```

Tawny is self-contained as a Compose project. The Dioxus server handles the Piped-like application work while two deployment-local sidecars isolate yt-dlp and PO-token churn from the Rust process. SurrealDB, the provider, the yt-dlp API, and the production Tawny image are defined together; no public Piped or third-party instance is contacted.

The server owns every account's data: follows, playlists, groups, queue, history and watch progress. The client holds what its screens last saw of that data, and owns only per-device settings and the playing video's position between saves. Playback resolution is isolated behind its own module because YouTube delivery behavior changes independently from feed and library behavior.

## Data on the client

The client never holds the catalog, the shared set of every channel and video the instance has met. Each screen asks the server for what it shows, through `g3-cache`:

| Read | Server function | Holds |
|---|---|---|
| The viewer | `get_viewer` | Followed channels (without descriptions), playlists as ids, groups, queue, history ids. Read by nearly every screen, from `AppState`. |
| Feed | `get_feed_page(query, page)` | One page of 24. Groups, kind, length and watched filters are applied in SurrealDB. Each page is its own read, so "load more" appends. |
| Playlist | `get_playlist(id)` | The playlist and its videos. Shared with a running playlist, so what autoplay walks is what the page shows. |
| Playlists page | `get_playlist_previews` | Counts and cover thumbnails. |
| Queue, history | `get_videos(ids)` | The videos behind the viewer's ids. |
| Channel, video | `get_channel_details`, `get_video_details` | As before, cached per id. |

Every video the server hands out carries the asking viewer's progress and its channel's avatar.

`g3-cache` persists each read to IndexedDB on web and a redb file on desktop and mobile, so a relaunch shows the last answer at once and refetches behind it; a mounted screen also refetches when the app regains focus. Offline, the screens you have opened show what they last showed. The cache is keyed to the signed-in account and emptied when it changes.

### Changes made on another device

Focus alone does not catch these. A phone left open on a playlist is never refocused, and on Android a loaded video keeps the WebView reporting itself visible in the background, so coming back to the app is not a focus event. So the server counts, in memory and per account, every write by kind (`LibraryChanges`: subscriptions, groups, playlists, queue, history, watched, progress), and each mutating server function in `src/api/library.rs` notes what it wrote. `library_sync::LibrarySync` asks for the counts (`get_library_changes`) whenever the app comes back into view (visibility, focus, `online`, Android's `tawnynativeplaybackresume`) and every 15 seconds while it is in view, and refetches only the kinds that moved, through `data_change::invalidate`. A server restart starts a new epoch, which counts as everything having changed.

A position saved elsewhere brings a **paused** player up to date (`catch_up_playback`): picking the phone back up after watching on the computer finds the video where the computer left it. It moves only when the server's copy differs from what this device last sent or read for that video, so a failed save of its own never pulls it back, and never while playing.

Returning to a list keeps the reader's place even when the refetch puts new videos above it: the shell's scroll memory holds the view to the card that was at the top (`data-scroll-anchor` on each `VideoCard`) as cards are added or removed. At the very top nothing is held, so new uploads show there.

### Changing data

A change is shown at once and then made on the server:

1. `AppState` edits the cached reads it touches with `update_cached` (the viewer, a playlist, every cached list holding a video);
2. it sends a "set to" server function: `set_subscriptions`, `set_in_playlist`, `save_playlist`, `set_queue`, `record_history`, `save_progress` and so on;
3. `data_change::invalidate` refetches the reads that change could affect, which replaces the guess with the server's answer and undoes it if the server refused.

Mutations say what the viewer saw rather than toggling (`set_in_playlist(id, video, true)`), so a device acting on stale state cannot undo another device's change. The queue is sent whole, since it is one list arranged as a whole. Playback position is recorded locally on every tick and saved at the points where losing it would matter: pause, leaving the video, and a slower interval.

`AppSettings` are per device and stay in the WebView's local storage.

## Server and SurrealDB

Compose points the server at a dedicated SurrealDB container backed by a named SurrealKV volume. Tawny itself uses the configured `SURREALDB_HOST` endpoint rather than embedding a storage engine; server tests use `mem://`. SurrealKit embeds and hash-tracks `database/schema/**/*.surql`, applying changed schema files at startup. The schema models:

- channels and videos;
- users and subscription relations;
- playlists and ordered playlist-item relations;
- subscription groups and their channel membership;
- normalized video-detail cache entries.

Server caches are `g3-cache` `ServerCache` statics, bounded and expiring: search results for five minutes (identical searches share one upstream request, and searches take turns), SponsorBlock segments for an hour, and DASH segment ranges by exact file. Resolved playback sessions keep their own cache in `AppServerState`, because they share in-flight resolves and only reuse yt-dlp sources. Video details are cached in SurrealDB for two hours, and stale details are kept for offline fallback.

Tawny's server connects to the SurrealDB endpoint in `SURREALDB_HOST`. The checked-in Compose stack supplies a persistent SurrealDB container for development and production, while server tests use an in-memory database. `TAWNY_DATA_DIR` controls Tawny's extractor cache and other app-owned server data. See [database/README.md](../database/README.md) for scoped local SurrealKit operator commands.

## Authentication

See [authentication.md](authentication.md).

## Discovery source contract

See [youtube.md](youtube.md).

## Playback model

See [playback.md](playback.md).

## Feature roadmap

The current slice covers the main navigation and interaction model. LibreTube parity should be added in these bounded layers:

1. comment replies and richer search suggestions/filters;
2. native SABR/UMP adapter and background playback controls;
3. SponsorBlock, Return YouTube Dislike, DeArrow, and live chat;
4. downloads, and suggested channels from what the viewer watches;
5. import/export, notification controls, and privacy settings.

Keeping those integrations behind source/provider traits prevents volatile YouTube details from spreading through the Dioxus UI.
