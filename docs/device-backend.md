# Tawny without a server

The desktop and Android apps can keep the library on the device, with no
server at all: the first launch offers "Use Tawny on this device" beside
"Connect to a Tawny server". The app runs the server inside itself, on
127.0.0.1, with the library in the app's own data folder, and can move that
library to a server later. A server itself always runs as the Compose stack
(see [deployment.md](deployment.md)).

## On this device

```text
app (desktop or mobile build)
  server_url: backend = "device"
    └─ device_backend::host: load tawny_backend.dll / libtawny_backend.so
          └─ tawny_backend_start(data_dir) → port
               (the library: this crate built with `server`)
               └─ device_backend::serve: AppServerState on SurrealKV,
                  server_router::router, axum on 127.0.0.1:48080
  server functions → http://127.0.0.1:48080
```

### Why a separate library

The backend cannot be compiled into the app. A build with the `server` feature
turns every server function call into a direct call that needs a request's
context, and Cargo's feature unification would turn on g3-cache's and
g3-auth's server halves in the app, which disable the client cache. So the
backend is a second build of this crate, with `server`, linked as a shared
library, and the app loads it with `libloading` at launch. Each side stays what it is, and the
client talks to its own device exactly as it talks to a remote server.

The library exports two C functions: `tawny_backend_abi()`, which the app
checks against its own `ABI_VERSION`, and `tawny_backend_start(data_dir)`,
which returns the port once it is listening. Requests that arrive while the
database opens wait for it rather than failing, so the app's first render is
not held up by migrations.

`src/server/mod.rs` knows it is a device backend through `run_on_device`. Then
it reads no deployment settings from the environment (`server_setting`), so a
desktop app started from a developer's shell does not pick up `.env`'s
sidecars, and builds its playback proxy URLs from its own loopback address.

Sessions on a device last ten years. A device's library sits behind a guest
session, and a lapsed session would leave it owned by an account nothing can
sign in to.

### Building it

```sh
just backend-desktop    # target/device-backend/tawny_backend.dll (or .so, .dylib)
just backend-android    # android/localstreams/src/main/jniLibs/<abi>/libtawny_backend.so
```

`just dev-desktop` and `just dev-android` build it first. A development desktop
app finds it through `TAWNY_BACKEND_LIBRARY`, which the justfile exports; a
packaged one looks next to its executable. Android finds it by name among the
APK's native libraries: Gradle packages the localstreams module's `jniLibs`.

`scripts/device-backend.mjs` builds the binary with `cargo rustc --bin tawny
--features server` and copies it into place. rustc will not emit a `cdylib`
from a binary crate, so, as dx does for the Android app itself, the binary is
linked as a shared library instead: `-shared` with a version script exporting
the two entry points on Android and Linux, `/DLL /EXPORT:tawny_backend_abi
/EXPORT:tawny_backend_start` on Windows. Its `main` is never called. dx's own
`--export-dynamic` would export every symbol, 70 MB of symbol table in a phone
build. For Android the script points Cargo at the NDK's clang
(`ANDROID_NDK_HOME`), lets rustypipe's QuickJS generate its bindings against
the NDK's headers, and strips debug info. A development build is still about
240 MB of unoptimised code; build with `--release` for a phone you hand out.

The backend makes its data folder the process's working directory before it
opens the database. SurrealKit's schema sync reads and creates
`database/setup.surql` relative to the working directory, with no setting for
it, and an app's working directory is read-only: `/` on Android, the install
folder on a desktop. Nothing in the app opens a file by a relative path.

The device backend serves the API alone: Dioxus's full router also serves
`public/` from beside the executable and panics when there is none, which is
the case beside an app. If the backend fails to start, every request answers
503 with the reason, which the setup screen shows.

### What differs from a server

- **Playback.** On Android the phone resolves streams itself
  (`local_streams`, see [on-device-playback.md](on-device-playback.md)), which
  is the first thing the player tries anyway. The desktop backend runs `yt-dlp`
  from `PATH`; without one, videos do not play. With no PO-token provider it
  asks yt-dlp for the `android_sdkless` client, then `web_embedded`, one run
  each as the phone does: yt-dlp's default clients answer without a token too, but
  their URLs are gated, and a range 80% of the way into a file gets a 403. A
  server whose provider container is down falls back to the same pair.
- **New uploads** arrive by the RSS poller while the app runs. A loopback
  address cannot receive WebSub, so there is no push.
- **The web app** cannot keep a library on the device: a page cannot run a
  server, and YouTube does not answer a page's requests. The setup screen only
  offers it in the desktop and mobile apps.
- **iOS** builds the host side but nothing yet packages the library.

## Moving a library

Settings → Library moves the library between this device and a server, either
way:

1. `export_library` reads everything the account keeps from the backend in use:
   follows, groups, playlists, queue, history, watch progress, and the videos
   the lists show.
2. The app keeps that in `pending-library.json` beside the device library and
   switches backend. It reconnects, as a guest or with a session it already had
   there.
3. Settings offers "Add your library". The viewer can first sign in to an
   account they already have on that server. `import_library` then merges the
   library in.

The import merges and never removes (`src/server/library_move.rs`):

| Part | Rule |
|---|---|
| Follows | Added when not followed; an existing follow keeps its own Videos/Shorts choice |
| Groups | Matched by id or name; channels added |
| Playlists | Matched by id or name; missing videos appended in the other side's order. An empty default playlist the viewer deleted is not recreated |
| Queue | Missing entries appended |
| History | Each video once, at its latest play; newest 250 kept |
| Progress | Watched on either side stays watched; the further position wins |

Nothing is deleted from the side the library left, and running an import twice
changes nothing, so a move that stops halfway can simply be made again.

The export carries only the videos the playlists, queue and history show, not
one per progress entry: axum caps a request body at 2 MB, and a long-used
account has thousands of the latter.
