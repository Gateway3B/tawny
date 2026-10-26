# On-device playback (Android)

The server resolves a video with its yt-dlp sidecar, and the stream URLs
YouTube gives it only work from the server's own address. So every byte of
video goes YouTube → home server → phone, and far from home that round trip is
the slow part. On Android the app can resolve the video itself instead. The
URLs then belong to the phone, and video comes straight from YouTube. The
server still holds the library; only the video bytes skip it.

It is on by default ("Stream from this phone" in Settings, Android only). If
anything about it fails, the player asks the server as before, so the cost of a
failure is the seconds the attempt took.

## How it fits together

```
player.rs ─ resolve_session (playback_resolver.rs)
              ├─ Android + setting on: local_streams::resolve
              │    └─ JNI → LocalStreamsPlugin.kt (android/localstreams)
              │          └─ Chaquopy Python: tawny_local.resolve (yt-dlp)
              │                └─ JS challenges: tawny_jsc → JsSandbox.kt (Android V8)
              │    tracks re-pointed at the local relay (local_relay.rs, 127.0.0.1)
              └─ otherwise, or on any failure: resolve_playback (the server)
```

- **yt-dlp in the app.** `android/localstreams` is a Gradle module that
  Chaquopy builds with an embedded CPython 3.13, plus yt-dlp and yt-dlp-ejs
  pinned to the versions the server's sidecar runs. `tawny_local.py` asks one
  YouTube client at a time (`android_sdkless`, then `web_embedded`). Those are
  the clients that give working DASH URLs without a PO token. It tries one byte
  of a client's URLs before handing them out, and answers a `PlaybackSource` in
  the same shape the server's would be.
- **Byte ranges.** yt-dlp drops the init and index ranges a DASH manifest
  needs. `tawny_local.py` keeps the player responses yt-dlp parsed and joins
  each format to its layout by itag and exact size: dubbed audio tracks share
  an itag, and only the size tells their files apart.
- **JavaScript challenges.** yt-dlp normally runs its solver script in Deno,
  Node, Bun or QuickJS as a subprocess. An app cannot ship such an executable:
  Android refuses to run binaries from app storage, and native libraries are
  only extracted to disk if the app's own Gradle file asks, and `dx` generates
  that file. `tawny_jsc.py` registers a yt-dlp challenge provider that sends
  the same script to Android's V8 sandbox (`androidx.javascriptengine`)
  instead. It needs a recent Android System WebView.
- **The local relay.** YouTube's media servers send no CORS headers, so Shaka
  in the WebView cannot fetch them directly, and they want the headers yt-dlp
  resolved them with. `local_relay.rs` listens on 127.0.0.1, adds both, and
  streams byte ranges through unbuffered. Each stream gets an unguessable path.
- **Fallback.** Live streams, videos YouTube refuses these clients, a resolve
  slower than 45 seconds, or a missing V8 sandbox all fall back to the server.
  The transport's session renewal (`refreshUrl`) always goes to the server, so
  a device session that stops working mid-video recovers through it.

## What it costs

- **App size.** About 30–40 MB for Python, yt-dlp and the solver, per ABI. The
  module builds for arm64-v8a and x86_64 (the emulator).
- **First play after launch.** A few extra seconds to start Python and the
  sandbox. After that a resolve takes about as long as the server's.
- **Upkeep.** yt-dlp breaks when YouTube changes. When the server's sidecar is
  bumped, bump `android/localstreams/build.gradle.kts` to the same version. The
  wrapper uses yt-dlp internals (`YoutubeIE._extract_formats_and_subtitles`,
  the EJS provider base class), so check it against the new version.
- **Licence.** Tawny is MIT/Apache. yt-dlp is Unlicense, Chaquopy is MIT, and
  `androidx.javascriptengine` is Apache, so this path adds nothing GPL to the
  app (unlike NewPipeExtractor, which LibreTube uses).

## Testing the Python without a phone

The extractor container has the same yt-dlp and a Node runtime. Node in a bare
`vm` context stands in for the V8 sandbox, which has no `console` either:

```sh
docker exec tawny-extractor-1 mkdir -p /tmp/tl
docker cp android/localstreams/src/main/python/. tawny-extractor-1:/tmp/tl/
docker exec -i tawny-extractor-1 python3 - <<'EOF'
import json, subprocess, sys
sys.path.insert(0, "/tmp/tl")
import tawny_jsc, tawny_local
NODE = "const fs=require('fs');const vm=require('vm');process.stdout.write(String(vm.runInNewContext(fs.readFileSync(0,'utf8'))))"
tawny_jsc.evaluate = lambda s: subprocess.run(["node", "-e", NODE], input=s, capture_output=True, text=True, check=True).stdout
source = json.loads(tawny_local.resolve("aqz-KE-bpKQ", "/tmp/tl-cache"))
print(len(source["tracks"]), "tracks")
EOF
```

## How LibreTube does it, and why this differs

LibreTube resolves on the phone with NewPipeExtractor (Java), which deciphers
signatures with Rhino, gets PO tokens by running BotGuard in a hidden WebView,
and plays through ExoPlayer. Its fork mostly plays through SABR, YouTube's
stateful streaming protocol, with its own Kotlin client. That suits a native
ExoPlayer app. Tawny plays in a WebView with Shaka and already speaks yt-dlp's
output on the server, so running yt-dlp on the phone reuses the path the
server has proven and keeps the app free of GPL code.
