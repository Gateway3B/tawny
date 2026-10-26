"""Resolve a YouTube video's streams on the phone, in Tawny's session shape.

The server does this with its yt-dlp sidecar; doing it here instead makes the
stream URLs belong to the phone's own address, so video comes straight from
YouTube rather than through the home server. `resolve` answers with the JSON
of a `PlaybackSource` (`src/models/playback.rs`): a DASH source whose tracks
carry the URLs, byte ranges and request headers the player and the app's local
relay need.

yt-dlp hands out usable URLs, but drops the initialisation and index byte
ranges a DASH manifest needs. They are in the player responses it parsed, so
`_extract_formats_and_subtitles` is wrapped to keep those, and each format is
joined to its layout by itag and exact size: dubbed audio tracks share an
itag, and only the size tells their files apart.
"""

from __future__ import annotations

import json
import threading
import time
import urllib.error
import urllib.request

import yt_dlp
from yt_dlp.extractor.youtube._video import YoutubeIE

import tawny_jsc  # registers the sandbox challenge solver

# On the phone, Chaquopy's `java` module reaches the V8 sandbox in
# `LocalStreamsPlugin.kt`. Elsewhere (the tests) the caller sets its own.
if tawny_jsc.evaluate is None:
    try:
        from java import jclass
    except ImportError:
        pass
    else:
        _sandbox = jclass("net.gateway3.tawny.localstreams.JsSandbox")
        tawny_jsc.evaluate = lambda script: str(_sandbox.evaluate(script))

        # yt-dlp and the standard library look for programs by running them
        # (`deno --version`, `ffmpeg -bsfs`, `ldconfig -p`, `file`). None
        # exists on a phone, and each attempt forked the whole app first. Fail
        # them before the fork; every caller already reads that as "not
        # installed".
        import subprocess

        def _no_processes(*args, **kwargs):
            raise FileNotFoundError("Tawny does not start processes on Android")

        subprocess.Popen._execute_child = _no_processes

# Clients that give working DASH URLs without a PO token, as of yt-dlp
# 2026.08.19, in order of preference. `web_embedded` covers some of what
# `android_sdkless` refuses, but its URLs are refused for other videos that
# `android_sdkless` plays. So each is asked on its own (asked together, yt-dlp
# keeps one copy of a shared format, not always the working one), and its
# URLs are tried before they are handed out.
CLIENTS = ["android_sdkless", "web_embedded"]

# One resolve at a time: the wrapper below is installed on the class, and the
# player's ahead-of-time warm-up would otherwise race the video being opened.
_lock = threading.Lock()


class ResolveError(Exception):
    pass


def resolve(video_id: str, cache_dir: str | None = None) -> str:
    with _lock:
        return json.dumps(_resolve(video_id, cache_dir))


def _resolve(video_id: str, cache_dir: str | None) -> dict:
    failures = []
    for client in CLIENTS:
        started = time.monotonic()
        try:
            source = _resolve_with(video_id, client, cache_dir)
            print(f"tawny: {client} resolved {video_id} in {time.monotonic() - started:.1f}s", flush=True)
        except Exception as error:  # noqa: BLE001  (yt-dlp raises many kinds)
            failures.append(f"{client}: {error}")
            continue
        probed = time.monotonic()
        if _plays(source):
            print(f"tawny: probe took {time.monotonic() - probed:.1f}s", flush=True)
            return source
        failures.append(f"{client}: YouTube refused its stream URLs")
    raise ResolveError("; ".join(failures) or "no client to ask")


def _plays(source: dict) -> bool:
    """Whether YouTube serves this device the first bytes of an audio track."""
    track = next(t for t in source["tracks"] if t["kind"] == "Audio")
    headers = {h["name"]: h["value"] for h in track["request_headers"]}
    headers["Range"] = "bytes=0-1"
    try:
        with urllib.request.urlopen(
            urllib.request.Request(track["url"], headers=headers), timeout=10
        ) as response:
            return response.status in (200, 206)
    except (urllib.error.URLError, OSError):
        return False


def _resolve_with(video_id: str, client: str, cache_dir: str | None) -> dict:
    responses: list[dict] = []
    original = YoutubeIE._extract_formats_and_subtitles

    def keep_responses(self, video_id_, player_responses, *args, **kwargs):
        responses.extend(player_responses)
        return original(self, video_id_, player_responses, *args, **kwargs)

    YoutubeIE._extract_formats_and_subtitles = keep_responses
    try:
        options = {
            "quiet": True,
            "no_warnings": True,
            "skip_download": True,
            "noplaylist": True,
            "cachedir": cache_dir or False,
            "extractor_args": {"youtube": {"player_client": [client]}},
        }
        with yt_dlp.YoutubeDL(options) as ydl:
            info = ydl.extract_info(
                f"https://www.youtube.com/watch?v={video_id}", download=False
            )
    finally:
        YoutubeIE._extract_formats_and_subtitles = original
    return source_from(info, responses)


def _layouts(responses: list[dict]) -> dict:
    layouts = {}
    for response in responses:
        streaming = response.get("streamingData") or {}
        for fmt in streaming.get("adaptiveFormats") or []:
            if "initRange" not in fmt or "indexRange" not in fmt or "itag" not in fmt:
                continue
            key = (int(fmt["itag"]), int(fmt.get("contentLength") or 0))
            layouts.setdefault(key, fmt)
    return layouts


def _byte_range(value: dict) -> dict:
    return {"start": int(value["start"]), "end": int(value["end"])}


def _headers(fmt: dict) -> list[dict]:
    return [
        {"name": name, "value": value}
        for name, value in (fmt.get("http_headers") or {}).items()
        if value
    ]


def source_from(info: dict, responses: list[dict]) -> dict:
    if info.get("is_live") or info.get("live_status") in ("is_live", "is_upcoming"):
        raise ResolveError("live streams are played through the server")
    tracks = _tracks(info.get("formats") or [], _layouts(responses))
    if not any(t["kind"] == "Video" for t in tracks) or not any(
        t["kind"] == "Audio" for t in tracks
    ):
        raise ResolveError("YouTube gave this device no adaptive audio and video")
    return _source(tracks)


def _tracks(formats: list[dict], layouts: dict) -> list[dict]:
    tracks = []
    seen = set()
    for fmt in formats:
        if fmt.get("protocol") != "https" or not fmt.get("url"):
            continue
        format_id = str(fmt.get("format_id") or "")
        # Dynamic-range-compressed audio duplicates a normal track.
        if "drc" in format_id:
            continue
        itag_text = format_id.split("-")[0]
        if not itag_text.isdigit():
            continue
        has_video = fmt.get("vcodec") not in (None, "none")
        has_audio = fmt.get("acodec") not in (None, "none")
        if has_video == has_audio:
            continue
        layout = layouts.get((int(itag_text), int(fmt.get("filesize") or 0)))
        if layout is None:
            continue
        key = (int(itag_text), int(layout.get("contentLength") or 0))
        if key in seen:
            continue
        seen.add(key)
        audio_track = layout.get("audioTrack") or {}
        track = {
            "kind": "Video" if has_video else "Audio",
            "url": fmt["url"],
            "mime_type": layout.get("mimeType") or "",
            "bitrate": int(layout.get("bitrate") or 0) or None,
            "content_length": int(layout.get("contentLength") or 0) or None,
            "duration_ms": int(layout.get("approxDurationMs") or 0) or None,
            "width": layout.get("width"),
            "height": layout.get("height"),
            "fps": layout.get("fps"),
            "quality_label": layout.get("qualityLabel")
            if has_video
            else f"{int(layout.get('bitrate') or 0) // 1000} kbps",
            "language": (audio_track.get("id") or "").split(".")[0] or None
            if not has_video
            else None,
            "label": audio_track.get("displayName") if not has_video else None,
            "is_default": bool(audio_track.get("audioIsDefault", True))
            if not has_video
            else False,
            "init_range": _byte_range(layout["initRange"]),
            "index_range": _byte_range(layout["indexRange"]),
            "request_headers": _headers(fmt),
        }
        tracks.append(track)
    return tracks


def _source(tracks: list[dict]) -> dict:
    first_video = next(t for t in tracks if t["kind"] == "Video")
    return {
        "protocol": "Dash",
        "url": first_video["url"],
        "mime_type": None,
        "po_token": None,
        "expires_at": None,
        "quality_label": "This device",
        "request_headers": [],
        "tracks": tracks,
    }
