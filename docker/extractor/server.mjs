// Small internal HTTP boundary around yt-dlp for Tawny.
//
// yt-dlp is intentionally kept out of the Tawny process and client images. This
// service owns the executable, JavaScript challenge runtime, and PO-token plugin.

import { spawn } from "node:child_process";
import { readdirSync, readFileSync } from "node:fs";
import { createServer } from "node:http";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

export const Event = Object.freeze({
  EXTRACTION_FAILED: "extraction_failed",
  REQUEST_FAILED: "request_failed",
  SERVICE_STARTED: "service_started",
  SERVICE_FAILED: "service_failed",
});

function writeEvent(code) {
  // No dynamic URL, address, subprocess stderr or exception is retained. The
  // line keeps the spacing the Python service printed, so log queries written
  // against it still match.
  process.stdout.write(`{"message": "service_event", "code": "${code}"}\n`);
}

// Python's int(): surrounding whitespace, a sign and digit underscores are
// accepted, anything else is a configuration error.
function integer(value) {
  const text = String(value).trim();
  if (!/^[+-]?\d+(_\d+)*$/.test(text)) {
    throw new Error("not an integer");
  }
  return Number(text.replaceAll("_", ""));
}

const VIDEO_ID = /^[A-Za-z0-9_-]{11}$/;
const CHANNEL_ID = /^UC[A-Za-z0-9_-]{22}$/;
const YTDLP_BINARY = process.env.YTDLP_BINARY ?? "yt-dlp";
const PO_TOKEN_PROVIDER_URL = (
  process.env.PO_TOKEN_PROVIDER_URL ?? "http://pot-provider:4416"
).replace(/\/+$/, "");
const EXTRACTION_TIMEOUT_SECONDS = integer(process.env.EXTRACTION_TIMEOUT_SECONDS ?? "35");
const MAX_CONCURRENT_EXTRACTIONS = integer(process.env.MAX_CONCURRENT_EXTRACTIONS ?? "2");
const VIDEO_QUEUE_SECONDS = integer(process.env.VIDEO_QUEUE_SECONDS ?? "15");

/// A counting semaphore whose acquire gives up after a timeout.
export class Slots {
  constructor(count) {
    this.free = count;
    this.waiting = [];
  }

  acquire(timeoutSeconds) {
    if (this.free > 0) {
      this.free -= 1;
      return Promise.resolve(true);
    }
    return new Promise((resolve) => {
      const waiter = { resolve, timer: null };
      waiter.timer = setTimeout(() => {
        this.waiting.splice(this.waiting.indexOf(waiter), 1);
        resolve(false);
      }, timeoutSeconds * 1000);
      this.waiting.push(waiter);
    });
  }

  release() {
    const next = this.waiting.shift();
    if (next) {
      clearTimeout(next.timer);
      next.resolve(true);
    } else {
      this.free += 1;
    }
  }
}

export function buildVideoCommand(videoId) {
  if (!VIDEO_ID.test(videoId)) {
    throw new RangeError("invalid YouTube video id");
  }
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
    `youtubepot-bgutilhttp:base_url=${PO_TOKEN_PROVIDER_URL}`,
    // The player client is deliberately not pinned. `mweb` worked when this
    // service was written and now returns "Video unavailable", which left
    // playback falling back to gated URLs that serve about a minute before
    // returning 403. yt-dlp tracks which clients YouTube currently accepts;
    // pinning one opts out of that and silently rots.
    `https://www.youtube.com/watch?v=${videoId}`,
  ];
}

export function buildChannelShortsCommand(channelId) {
  if (!CHANNEL_ID.test(channelId)) {
    throw new RangeError("invalid YouTube channel id");
  }
  return [
    YTDLP_BINARY,
    "--flat-playlist",
    "-J",
    "--playlist-end",
    "30",
    "--no-warnings",
    "--socket-timeout",
    "15",
    `https://www.youtube.com/channel/${channelId}/shorts`,
  ];
}

export function buildChannelUploadsCommand(channelId, tab) {
  if (!CHANNEL_ID.test(channelId) || !["videos", "shorts", "streams"].includes(tab)) {
    throw new RangeError("invalid YouTube channel listing");
  }
  // Flat entries avoid player extraction. Listing the complete tab lets
  // Tawny sort least popular globally, then serve small cached pages.
  return [
    YTDLP_BINARY,
    "--flat-playlist",
    "-J",
    "--no-warnings",
    "--socket-timeout",
    "15",
    "--extractor-args",
    "youtubetab:approximate_date",
    `https://www.youtube.com/channel/${channelId}/${tab}`,
  ];
}

export async function providerIsReady() {
  try {
    const response = await fetch(`${PO_TOKEN_PROVIDER_URL}/ping`, {
      signal: AbortSignal.timeout(2000),
    });
    await response.body?.cancel();
    return response.status >= 200 && response.status < 300;
  } catch {
    return false;
  }
}

// Where pip puts packages in the python base image, and in Debian's Python.
function sitePackagesDirectories() {
  const directories = ["/usr/lib/python3/dist-packages"];
  for (const lib of ["/usr/local/lib", "/usr/lib"]) {
    let entries = [];
    try {
      entries = readdirSync(lib);
    } catch {
      continue;
    }
    for (const entry of entries) {
      if (/^python3(\.\d+)?$/.test(entry)) {
        directories.push(join(lib, entry, "site-packages"), join(lib, entry, "dist-packages"));
      }
    }
  }
  return directories;
}

/// The installed PO-token plugin's version, read from the package metadata
/// pip wrote, or null when it is not installed. The plugin is a Python
/// package loaded by yt-dlp, so its metadata is the only place that says
/// which version yt-dlp will load.
export function pluginVersion(directories = sitePackagesDirectories()) {
  for (const directory of directories) {
    let entries = [];
    try {
      entries = readdirSync(directory);
    } catch {
      continue;
    }
    for (const entry of entries) {
      const name = /^bgutil[-_.]ytdlp[-_.]pot[-_.]provider-(.+)\.dist-info$/i.exec(entry);
      if (!name) {
        continue;
      }
      try {
        const metadata = readFileSync(join(directory, entry, "METADATA"), "utf8");
        const version = /^Version:\s*(.+?)\s*$/m.exec(metadata);
        if (version) {
          return version[1];
        }
      } catch {
        // A dist-info directory without metadata still names the version.
      }
      return name[1];
    }
  }
  return null;
}

/// Runs a command to completion. Resolves `{ timedOut: true }` once a command
/// that outlived the timeout has been killed.
export function runCommand(command, timeoutSeconds = EXTRACTION_TIMEOUT_SECONDS) {
  return new Promise((resolve, reject) => {
    const child = spawn(command[0], command.slice(1), { stdio: ["ignore", "pipe", "pipe"] });
    const stdout = [];
    const stderr = [];
    let exited = false;
    let timedOut = false;
    child.stdout.on("data", (chunk) => stdout.push(chunk));
    child.stderr.on("data", (chunk) => stderr.push(chunk));
    const timer = setTimeout(() => {
      timedOut = true;
      child.kill("SIGKILL");
      if (exited) {
        resolve({ timedOut: true });
      }
    }, timeoutSeconds * 1000);
    child.on("error", (error) => {
      clearTimeout(timer);
      reject(error);
    });
    // yt-dlp may leave a JavaScript runtime holding its pipes, so a timed-out
    // run waits only for yt-dlp itself to exit.
    child.on("exit", () => {
      exited = true;
      if (timedOut) {
        resolve({ timedOut: true });
      }
    });
    child.on("close", (code) => {
      clearTimeout(timer);
      if (!timedOut) {
        resolve({
          timedOut: false,
          returncode: code ?? -1,
          stdout: Buffer.concat(stdout),
          stderr: Buffer.concat(stderr),
        });
      }
    });
  });
}

// Python's urllib.parse.unquote: percent escapes decode as UTF-8 and invalid
// sequences become U+FFFD instead of failing the request.
function unquote(text) {
  return text.replace(/(?:%[0-9A-Fa-f]{2})+/g, (run) =>
    new TextDecoder().decode(Buffer.from(run.replaceAll("%", ""), "hex")),
  );
}

function requestPath(url) {
  // An absolute-form or scheme-relative target keeps only its path, as
  // urllib.parse.urlparse reads it.
  const path = url.split(/[?#]/, 1)[0].replace(/^(?:[A-Za-z][A-Za-z0-9+.-]*:)?\/\/[^/]*/, "");
  return unquote(path);
}

function sendBytes(response, status, body, contentType = "application/json") {
  response.writeHead(status, {
    "Content-Type": contentType,
    "Content-Length": String(body.length),
    "Cache-Control": "no-store",
    "X-Content-Type-Options": "nosniff",
    // One request per connection, as the Python service answered over
    // HTTP/1.0, so Tawny's pooled client never reuses a socket this end is
    // about to close.
    Connection: "close",
  });
  response.end(body);
}

function sendJson(response, status, payload) {
  sendBytes(response, status, Buffer.from(JSON.stringify(payload), "utf8"));
}

/// The service, with its outside effects replaceable for tests.
export function createService({
  log = writeEvent,
  isProviderReady = providerIsReady,
  installedPluginVersion = pluginVersion,
  run = runCommand,
  videoSlots = MAX_CONCURRENT_EXTRACTIONS,
} = {}) {
  // Playing a video and listing a channel's Shorts do not share slots. The
  // Shorts listings come in bursts from background work - a refresh after the
  // app has been away fills every slot with them - and a viewer pressing play
  // then got "extractor is busy" for something they had asked for themselves.
  // Videos wait their turn instead of being turned away; a listing that finds
  // its one slot taken just skips this round.
  const VIDEO_SLOTS = new Slots(videoSlots);
  const LISTING_SLOTS = new Slots(1);

  async function handle(request, response) {
    response.setHeader("Server", "TawnyYtdlp/1");
    if (request.method !== "GET") {
      // Only GET is served, as BaseHTTPRequestHandler answered for the others.
      sendBytes(
        response,
        501,
        Buffer.from("<p>Error code: 501</p><p>Message: Unsupported method.</p>\n"),
        "text/html;charset=utf-8",
      );
      return;
    }
    const path = requestPath(request.url ?? "/");
    if (path === "/health") {
      const plugin = installedPluginVersion();
      const provider = await isProviderReady();
      const status = plugin && provider ? 200 : 503;
      sendJson(response, status, {
        ready: status === 200,
        provider,
        plugin_version: plugin,
      });
      return;
    }

    const videoPrefix = "/v1/videos/";
    // Whole-path shapes, so `/uploads/shorts` is never read as the Shorts route.
    const shortsRoute = /^\/v1\/channels\/([^/]*)\/shorts$/.exec(path);
    const uploadsRoute = /^\/v1\/channels\/([^/]*)\/uploads\/([^/]*)$/.exec(path);
    let command;
    if (path.startsWith(videoPrefix)) {
      const resourceId = path.slice(videoPrefix.length);
      if (!VIDEO_ID.test(resourceId)) {
        sendJson(response, 400, { error: "invalid video id" });
        return;
      }
      if (!(await isProviderReady())) {
        sendJson(response, 503, { error: "PO-token provider is unavailable" });
        return;
      }
      command = buildVideoCommand(resourceId);
    } else if (shortsRoute) {
      const resourceId = shortsRoute[1];
      if (!CHANNEL_ID.test(resourceId)) {
        sendJson(response, 400, { error: "invalid channel id" });
        return;
      }
      command = buildChannelShortsCommand(resourceId);
    } else if (uploadsRoute) {
      try {
        command = buildChannelUploadsCommand(uploadsRoute[1], uploadsRoute[2]);
      } catch {
        sendJson(response, 400, { error: "invalid channel listing" });
        return;
      }
    } else {
      sendJson(response, 404, { error: "not found" });
      return;
    }

    const slots = path.startsWith(videoPrefix) ? VIDEO_SLOTS : LISTING_SLOTS;
    const wait = slots === VIDEO_SLOTS ? VIDEO_QUEUE_SECONDS : 1;
    if (!(await slots.acquire(wait))) {
      sendJson(response, 429, { error: "extractor is busy" });
      return;
    }
    let result;
    try {
      result = await run(command);
    } finally {
      slots.release();
    }
    if (result.timedOut) {
      sendJson(response, 504, { error: "extraction timed out" });
      return;
    }

    if (result.returncode !== 0) {
      const stderr = new TextDecoder().decode(result.stderr).trim();
      log(Event.EXTRACTION_FAILED);
      // yt-dlp's own words go back to Tawny, which sorts them into
      // YouTube refusing the video versus something to fix here.
      sendJson(response, 502, {
        error: "yt-dlp extraction failed",
        ytdlp_error: [...stderr].slice(-2000).join(""),
      });
      return;
    }
    sendBytes(response, 200, result.stdout);
  }

  // Request URLs include viewing choices, so there are no access logs, and a
  // failed request reports only that it failed: never its address or error.
  const server = createServer((request, response) => {
    handle(request, response).catch(() => {
      log(Event.REQUEST_FAILED);
      response.socket?.destroy();
    });
  });
  return server;
}

export function main() {
  const failed = () => {
    writeEvent(Event.SERVICE_FAILED);
    process.exit(1);
  };
  try {
    const port = integer(process.env.PORT ?? "8080");
    const server = createService();
    server.once("error", failed);
    server.listen(port, "0.0.0.0", () => writeEvent(Event.SERVICE_STARTED));
  } catch {
    failed();
  }
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  main();
}
