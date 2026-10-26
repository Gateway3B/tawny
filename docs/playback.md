# Playback

How a video goes from a YouTube id to a playing media element: resolved on the server, proxied, and played by Shaka in the page. The server side is `src/server/{resolve,ytdlp,proxy,segments}.rs`; the client side is `src/components/player.rs` and `assets/tawny_transport.js`.

## The model

`playback_session` orchestrates direct extraction and returns a `PlaybackSession` (see `models.rs`) over the server function boundary. Stable application models remain inside Tawny; the volatile yt-dlp executable, JavaScript runtime, and provider plugin live behind a small private HTTP sidecar.

A `PlaybackSource` carries a `protocol` of `Sabr`, `Hls`, `Dash`, `Progressive`, or `EmbedFallback`. Adaptive sources carry a `tracks` list holding at least one indexed `Video` and one indexed `Audio` representation, and normally every available quality so the client can select automatically. Each track keeps its codec, bitrate, duration, resolution, and DASH initialization/index byte ranges; range ends are inclusive. The yt-dlp sidecar obtains a video-bound token from the bgutil provider; legacy direct extraction can additionally use `rustypipe-botguard`. Playback sources and PO tokens are deliberately kept out of the persistent cache because they are short-lived and request-bound.

Resolving a session costs several seconds, nearly all of it yt-dlp, so the server holds each resolved session in memory for an hour. Concurrent asks for one video share a single resolve task. Proxy registrations are minted per hand-out, so a held session gets their full lifetime. Only sessions with a yt-dlp source are reused; the embed or gated extractor URLs are re-resolved on the next ask. Clients ask ahead: the player warms whatever is up next in the queue once the current video's session is in, and a watch page requests playback alongside its details instead of after them. A player whose session stopped working asks with `fresh=true`, which discards the held session.

Stream URLs come from the Compose-managed yt-dlp service, joined to the extractor's stream layout by itag. The sidecar checks its adjacent bgutil provider before invoking the token-backed mweb client and also owns the fallback used to enumerate channel Shorts. This split exists because stream layout and usable URLs fail independently. Current YouTube GVS URLs can require a video-bound PO token and otherwise return 403 after an initially successful extraction. Byte ranges, codecs, languages, and sizes still come from the extractor — an itag identifies one specific transcode, so different clients hand out different URLs for the same file. Sizes are compared per itag and a mismatch keeps the extractor's URL rather than pointing ranges at the wrong bytes. Direct host execution remains a legacy fallback only when `TAWNY_YTDLP_SERVICE_URL` is absent.

Source order is direct YouTube player extraction ranked by `playback_source_priority` — adaptive track sets first, then HLS, progressive, and plain manifests. The privacy-enhanced YouTube embed is **not** an automatic fallback: when every direct source fails, the player surfaces the transport's own error with a retry action, and the embed is offered only as an explicit user choice.

The player is mounted in the shared application shell, expands above video details, and switches to a fixed mini-player above bottom navigation on other routes without replacing the media element. Non-embed sources and every adaptive track are registered as short-lived same-origin proxy targets. The proxy forwards byte ranges and configured request headers, streams response bodies, bounds connect and per-chunk read time so a wedged upstream surfaces as an error instead of an indefinite stall, and rewrites HLS playlists plus DASH `BaseURL` elements so child requests stay same-origin. Expired proxy registrations are pruned as new sessions are created.

The client generates an in-memory DASH manifest for extracted representation sets and sends it to the packaged Shaka transport. Shaka handles Media Source capability negotiation, automatic bitrate selection, buffering, seeking, DASH/HLS manifests, and retry recovery. Progressive playback and native HLS are retained for platforms without Media Source support. On a fatal request, the transport records the current position, resolves fresh signed sources, and loads the renewed primary at that position before trying another protocol. The position is also retained across a final Dioxus media-element remount.

`Sabr` sources are accepted when the URL exposes a playable DASH/HLS relay or when the application installs a `window.TawnySabrAdapter`; raw YouTube UMP remains isolated behind that explicit adapter because it is not a media URL. The native adapter must follow the current stateful SABR contract rather than treating the endpoint like an ordinary segment URL: increment `rn` on every request, use the requested segment start as `playerTimeMs`, advertise selected formats and buffered ranges, carry playback cookies and SABR contexts forward, obey server backoff and redirects, and renew the per-session PO token when protection status requires it. The parsed UMP media headers and payloads then feed the platform media-buffer implementation without exposing Googlevideo URLs to the UI.

## Sources and the sidecar

The pinned Shaka Player package supplies the cross-platform DASH/HLS Media Source transport. Dioxus packages its compiled browser runtime as a local app asset; playback never depends on a third-party CDN.

Playback is resolved server-side from direct YouTube extraction, which preserves every indexed audio/video representation including codec, bitrate, duration, quality, and DASH initialization/index byte ranges.

Stream URLs come from the Compose-managed yt-dlp service and are matched to the extractor's streams by itag. The sidecar owns yt-dlp, its bgutil plugin, and the Node runtime used for YouTube JavaScript challenges; it requests video-bound GVS PO tokens from the adjacent provider. Raw media URLs are replaced with short-lived same-origin proxy URLs before reaching the client. The player turns the representation set into a local DASH manifest and uses its packaged adaptive engine for automatic quality selection, buffering, seeking, retry recovery, DASH, and HLS. A fatal media request snapshots the current timestamp and renews the active playback session first, then tries protocol fallbacks, without replaying from zero.

### PO-token provider setup

There is no separate host installation step. `docker compose up -d --build` starts the provider and the extractor service together, and the extractor health check verifies both the installed plugin and the provider's `/ping` endpoint. Development Tawny uses `TAWNY_YTDLP_SERVICE_URL=http://127.0.0.1:8090`; production Compose replaces that with the internal `http://extractor:8080` address. `TAWNY_YTDLP_BIN` and `TAWNY_PO_TOKEN_PROVIDER_URL` remain supported only as a legacy non-Compose fallback.

The privacy-enhanced YouTube embed is never substituted automatically. If every direct source fails, the player reports the transport's actual error and offers a retry; switching to the embed is an explicit user action.

The media element lives in the shared app shell: it is full-width on a video route and changes into a compact bar above bottom navigation while the user explores the rest of the app, without being unmounted. Playback speed, captions, chapter seeking, and picture-in-picture operate on that same element. Set `TAWNY_BOTGUARD_BIN` to a compatible `rustypipe-botguard` executable for PO-token-backed browser-client streams; the extractor caches session tokens while content-bound data stays ephemeral. A `Sabr` source is played through a playable DASH/HLS bridge or a registered `TawnySabrAdapter`; raw UMP parsing and BotGuard execution remain behind that adapter boundary rather than being mislabeled as ordinary media playback.

Tawny is not affiliated with or endorsed by YouTube. YouTube trademarks belong to their respective owners.

## Watch progress

While a video plays, its position is saved to the server about every five
seconds, and again when playback stops or the page is hidden or left, so
another device can pick up close to where this one stopped. When a video
starts, the player also reads its saved position from the server
(`get_video_progress`) and moves there if it differs by more than five seconds
from where it opened and the viewer has not moved it yet: the list it was
opened from may have been fetched on this device before the viewer stopped on
another.

## Taps on the video

With a mouse, a click on the video plays or pauses. On a touch screen, or any
pointer that cannot hover, a tap shows the controls and the next tap hides
them; only the play button plays or pauses. Double taps on the left or right
third seek, on every device.

## Pauses that are not the viewer's

The controls keep a playback intent. When the element pauses while the
intent is still "playing" and the page is hidden or in picture-in-picture,
that is taken as the browser or Shaka suspending playback, and it is resumed.
So every pause the viewer asks for has to record the intent first: the play
button does, the Android and iOS apps' own media sessions set it on the
element, and in a browser the player takes over the Media Session's play,
pause, stop and seek actions. Without that, a headset or lock-screen pause on
a phone browser was resumed at once.

On Android, a headset's single Play/Pause button asks the media element
whether it is paused rather than trusting the session's last reported state,
and every command's result is written back to the session. A pause that the
page never reported (the browser had already paused the element) otherwise
left the session saying "playing", and every press asked for Pause again. A
paused player also keeps its foreground service for twenty minutes (Media3's
default is ten) before letting it go: dropped at once, the app was cached and frozen in
the background, a headset press could not reach it, and a resume was refused a
new foreground start, so audio stopped with the screen off once the buffer
ran out. The session and its notification stay until the player closes.

The transport renews an expired playback session once, and a renewal that
then plays for 30 seconds earns another. A single renewal per video ran out on
long background listening, where proxy registrations expire after 90 minutes
and a network drop also spends one.
