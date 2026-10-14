"""Small internal HTTP boundary around yt-dlp for Tawny.

yt-dlp is intentionally kept out of the Tawny process and client images. This
service owns the executable, JavaScript challenge runtime, and PO-token plugin.
"""

from __future__ import annotations

import importlib.metadata
import json
import os
import re
import subprocess
import threading
import urllib.error
import urllib.request
from http import HTTPStatus
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import unquote, urlparse


VIDEO_ID = re.compile(r"^[A-Za-z0-9_-]{11}$")
CHANNEL_ID = re.compile(r"^UC[A-Za-z0-9_-]{22}$")
YTDLP_BINARY = os.environ.get("YTDLP_BINARY", "yt-dlp")
PO_TOKEN_PROVIDER_URL = os.environ.get(
    "PO_TOKEN_PROVIDER_URL", "http://pot-provider:4416"
).rstrip("/")
EXTRACTION_TIMEOUT_SECONDS = int(os.environ.get("EXTRACTION_TIMEOUT_SECONDS", "35"))
# Playing a video and listing a channel's Shorts do not share slots. The Shorts
# listings come in bursts from background work - a refresh after the app has
# been away fills every slot with them - and a viewer pressing play then got
# "extractor is busy" for something they had asked for themselves. Videos wait
# their turn instead of being turned away; a listing that finds its one slot
# taken just skips this round.
VIDEO_SLOTS = threading.BoundedSemaphore(
    int(os.environ.get("MAX_CONCURRENT_EXTRACTIONS", "2"))
)
LISTING_SLOTS = threading.BoundedSemaphore(1)
VIDEO_QUEUE_SECONDS = int(os.environ.get("VIDEO_QUEUE_SECONDS", "15"))


def build_video_command(video_id: str) -> list[str]:
    if not VIDEO_ID.fullmatch(video_id):
        raise ValueError("invalid YouTube video id")
    return [
        YTDLP_BINARY,
        "-J",
        "--no-warnings",
        "--no-playlist",
        "--socket-timeout",
        "15",
        "--js-runtimes",
        "node",
        "--extractor-args",
        f"youtubepot-bgutilhttp:base_url={PO_TOKEN_PROVIDER_URL}",
        # The player client is deliberately not pinned. `mweb` worked when this
        # service was written and now returns "Video unavailable", which left
        # playback falling back to gated URLs that serve about a minute before
        # returning 403. yt-dlp tracks which clients YouTube currently accepts;
        # pinning one opts out of that and silently rots.
        f"https://www.youtube.com/watch?v={video_id}",
    ]


def build_channel_shorts_command(channel_id: str) -> list[str]:
    if not CHANNEL_ID.fullmatch(channel_id):
        raise ValueError("invalid YouTube channel id")
    return [
        YTDLP_BINARY,
        "--flat-playlist",
        "-J",
        "--playlist-end",
        "30",
        "--no-warnings",
        "--socket-timeout",
        "15",
        f"https://www.youtube.com/channel/{channel_id}/shorts",
    ]


def build_channel_uploads_command(channel_id: str, tab: str) -> list[str]:
    if not CHANNEL_ID.fullmatch(channel_id) or tab not in ("videos", "shorts", "streams"):
        raise ValueError("invalid YouTube channel listing")
    # Flat entries avoid player extraction. Listing the complete tab lets
    # Tawny sort least popular globally, then serve small cached pages.
    return [
        YTDLP_BINARY, "--flat-playlist", "-J", "--no-warnings",
        "--socket-timeout", "15", "--extractor-args",
        "youtubetab:approximate_date",
        f"https://www.youtube.com/channel/{channel_id}/{tab}",
    ]


def provider_is_ready() -> bool:
    try:
        with urllib.request.urlopen(f"{PO_TOKEN_PROVIDER_URL}/ping", timeout=2) as response:
            return 200 <= response.status < 300
    except (OSError, urllib.error.URLError):
        return False


def plugin_version() -> str | None:
    try:
        return importlib.metadata.version("bgutil-ytdlp-pot-provider")
    except importlib.metadata.PackageNotFoundError:
        return None


class Handler(BaseHTTPRequestHandler):
    server_version = "TawnyYtdlp/1"

    def send_bytes(
        self, status: HTTPStatus, body: bytes, content_type: str = "application/json"
    ) -> None:
        self.send_response(status)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        self.send_header("X-Content-Type-Options", "nosniff")
        self.end_headers()
        self.wfile.write(body)

    def send_json(self, status: HTTPStatus, payload: dict[str, object]) -> None:
        self.send_bytes(status, json.dumps(payload).encode("utf-8"))

    def do_GET(self) -> None:  # noqa: N802 - BaseHTTPRequestHandler API
        path = unquote(urlparse(self.path).path)
        if path == "/health":
            plugin = plugin_version()
            provider = provider_is_ready()
            status = HTTPStatus.OK if plugin and provider else HTTPStatus.SERVICE_UNAVAILABLE
            self.send_json(
                status,
                {
                    "ready": status == HTTPStatus.OK,
                    "provider": provider,
                    "plugin_version": plugin,
                },
            )
            return

        video_prefix = "/v1/videos/"
        shorts_prefix = "/v1/channels/"
        if path.startswith(video_prefix):
            resource_id = path.removeprefix(video_prefix)
            if not VIDEO_ID.fullmatch(resource_id):
                self.send_json(HTTPStatus.BAD_REQUEST, {"error": "invalid video id"})
                return
            if not provider_is_ready():
                self.send_json(
                    HTTPStatus.SERVICE_UNAVAILABLE,
                    {"error": "PO-token provider is unavailable"},
                )
                return
            command = build_video_command(resource_id)
        elif path.startswith(shorts_prefix) and path.endswith("/shorts"):
            resource_id = path.removeprefix(shorts_prefix).removesuffix("/shorts")
            if not CHANNEL_ID.fullmatch(resource_id):
                self.send_json(HTTPStatus.BAD_REQUEST, {"error": "invalid channel id"})
                return
            command = build_channel_shorts_command(resource_id)
        elif path.startswith(shorts_prefix) and "/uploads/" in path:
            resource_id, tab = path.removeprefix(shorts_prefix).split("/uploads/", 1)
            try:
                command = build_channel_uploads_command(resource_id, tab)
            except ValueError:
                self.send_json(HTTPStatus.BAD_REQUEST, {"error": "invalid channel listing"})
                return
        else:
            self.send_json(HTTPStatus.NOT_FOUND, {"error": "not found"})
            return

        slots = VIDEO_SLOTS if path.startswith(video_prefix) else LISTING_SLOTS
        wait = VIDEO_QUEUE_SECONDS if slots is VIDEO_SLOTS else 1
        if not slots.acquire(timeout=wait):
            self.send_json(HTTPStatus.TOO_MANY_REQUESTS, {"error": "extractor is busy"})
            return
        try:
            result = subprocess.run(
                command,
                capture_output=True,
                check=False,
                timeout=EXTRACTION_TIMEOUT_SECONDS,
            )
        except subprocess.TimeoutExpired:
            self.send_json(HTTPStatus.GATEWAY_TIMEOUT, {"error": "extraction timed out"})
            return
        finally:
            slots.release()

        if result.returncode != 0:
            stderr = result.stderr.decode("utf-8", errors="replace").strip()
            print(f"yt-dlp failed for {resource_id}: {stderr}", flush=True)
            # yt-dlp's own words go back to Tawny, which sorts them into
            # YouTube refusing the video versus something to fix here.
            self.send_json(
                HTTPStatus.BAD_GATEWAY,
                {"error": "yt-dlp extraction failed", "ytdlp_error": stderr[-2000:]},
            )
            return
        self.send_bytes(HTTPStatus.OK, result.stdout)

    def log_message(self, message: str, *args: object) -> None:
        print(f"{self.address_string()} - {message % args}", flush=True)


def main() -> None:
    port = int(os.environ.get("PORT", "8080"))
    server = ThreadingHTTPServer(("0.0.0.0", port), Handler)
    print(f"Tawny yt-dlp service listening on 0.0.0.0:{port}", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
