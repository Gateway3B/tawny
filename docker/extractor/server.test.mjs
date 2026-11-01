import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { request } from "node:http";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, test } from "node:test";
import { fileURLToPath } from "node:url";

import {
  buildChannelShortsCommand,
  buildChannelUploadsCommand,
  buildVideoCommand,
  createService,
  pluginVersion,
  runCommand,
  Slots,
} from "./server.mjs";

describe("commands", () => {
  test("the command has the token provider and a supported JS runtime", () => {
    const command = buildVideoCommand("aNXB-8Aqt88");
    assert.ok(command.includes("node"));
    // Pinning a player client is what broke extraction once already: `mweb`
    // started returning "Video unavailable" and playback fell back to gated
    // URLs that die after about a minute. Let yt-dlp choose.
    assert.ok(!command.some((value) => value.startsWith("youtube:player_client=")));
    assert.ok(command.some((value) => value.startsWith("youtubepot-bgutilhttp:base_url=")));
  });

  test("the command rejects non-video ids", () => {
    assert.throws(() => buildVideoCommand("../../secrets"), RangeError);
  });

  test("the Shorts command is flat and bounded", () => {
    const command = buildChannelShortsCommand("UCsXVk37bltHxD1rDPwtNM8Q");
    assert.ok(command.includes("--flat-playlist"));
    assert.ok(command.includes("30"));
    assert.ok(command.at(-1).endsWith("/shorts"));
  });

  test("the Shorts command rejects non-channel ids", () => {
    assert.throws(() => buildChannelShortsCommand("veritasium"), RangeError);
  });

  test("the upload listing keeps dates and covers the complete tab", () => {
    const command = buildChannelUploadsCommand("UCsXVk37bltHxD1rDPwtNM8Q", "videos");
    assert.ok(command.includes("--flat-playlist"));
    assert.ok(command.includes("youtubetab:approximate_date"));
    assert.ok(!command.includes("--playlist-end"));
    assert.ok(command.at(-1).endsWith("/videos"));
  });

  test("the upload listing rejects unknown tabs and channel paths", () => {
    for (const [channel, tab] of [
      ["../../secret", "videos"],
      ["UCsXVk37bltHxD1rDPwtNM8Q", "../videos"],
    ]) {
      assert.throws(() => buildChannelUploadsCommand(channel, tab), RangeError);
    }
  });
});

describe("plugin version", () => {
  test("is read from the metadata pip installed, or null without it", () => {
    const root = mkdtempSync(join(tmpdir(), "tawny-extractor-"));
    try {
      assert.equal(pluginVersion([root, join(root, "missing")]), null);
      const distInfo = join(root, "bgutil_ytdlp_pot_provider-2.0.0.dist-info");
      mkdirSync(distInfo);
      writeFileSync(
        join(distInfo, "METADATA"),
        "Metadata-Version: 2.4\nName: bgutil-ytdlp-pot-provider\nVersion: 2.0.0\n",
      );
      assert.equal(pluginVersion([join(root, "missing"), root]), "2.0.0");
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });
});

describe("slots", () => {
  test("a waiter takes a released slot and a late one times out", async () => {
    const slots = new Slots(1);
    assert.equal(await slots.acquire(0.05), true);
    const queued = slots.acquire(5);
    slots.release();
    assert.equal(await queued, true);
    assert.equal(await slots.acquire(0.05), false);
  });
});

describe("subprocess", () => {
  test("collects output and exit code, and kills a run past its timeout", async () => {
    const node = process.execPath;
    const done = await runCommand([node, "-e", "process.stdout.write('{}');process.exit(0)"], 10);
    assert.deepEqual([done.timedOut, done.returncode, done.stdout.toString()], [false, 0, "{}"]);
    const failed = await runCommand(
      [node, "-e", "console.error('ERROR: nope');process.exit(1)"],
      10,
    );
    assert.equal(failed.returncode, 1);
    assert.match(failed.stderr.toString(), /ERROR: nope/);
    const slow = await runCommand([node, "-e", "setTimeout(() => {}, 60000)"], 0.2);
    assert.deepEqual(slow, { timedOut: true });
    await assert.rejects(runCommand([join(tmpdir(), "no-such-yt-dlp")], 10), { code: "ENOENT" });
  });
});

function get(port, path, headers = {}) {
  return new Promise((resolve, reject) => {
    request({ host: "127.0.0.1", port, path, headers }, (response) => {
      const chunks = [];
      response.on("data", (chunk) => chunks.push(chunk));
      response.on("end", () =>
        resolve({ status: response.statusCode, body: Buffer.concat(chunks).toString("utf8") }),
      );
    })
      .on("error", reject)
      .end();
  });
}

// stderr is captured too: the service must never write there, and a stray
// Node warning or stack trace would carry a request's details.
async function withService(options, body) {
  const events = [];
  const errors = [];
  const writeError = process.stderr.write;
  process.stderr.write = (chunk, ...rest) => {
    errors.push(String(chunk));
    return writeError.call(process.stderr, chunk, ...rest);
  };
  const server = createService({ log: (code) => events.push(code), ...options });
  try {
    await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
    await body(server.address().port);
  } finally {
    await new Promise((resolve) => server.close(resolve));
    process.stderr.write = writeError;
  }
  return { events, errors };
}

describe("privacy of logs", () => {
  test("a failed extraction keeps private diagnostics out of logs", async () => {
    const { events, errors } = await withService(
      {
        isProviderReady: async () => true,
        run: async () => ({
          timedOut: false,
          returncode: 1,
          stdout: Buffer.alloc(0),
          stderr: Buffer.from("provider-error-canary token-canary"),
        }),
      },
      async (port) => {
        const response = await get(port, "/v1/videos/aNXB-8Aqt88?query-canary", {
          Cookie: "cookie-canary",
        });
        assert.equal(response.status, 502);
        // The private internal response still lets Tawny classify failure.
        assert.match(JSON.parse(response.body).ytdlp_error, /provider-error-canary/);
      },
    );
    assert.deepEqual(errors, []);
    assert.deepEqual(events, ["extraction_failed"]);
  });

  test("access logs do not retain unknown paths or request details", async () => {
    const { events, errors } = await withService({}, async (port) => {
      const response = await get(port, "/unknown-person-canary?token-canary", {
        Authorization: "Bearer secret-canary",
      });
      assert.equal(response.status, 404);
    });
    assert.deepEqual(events, []);
    assert.deepEqual(errors, []);
  });

  test("unexpected handler errors never print a trace or address", async () => {
    const { events, errors } = await withService(
      {
        isProviderReady: async () => true,
        run: async () => {
          throw new Error("exception-secret-canary");
        },
      },
      async (port) => {
        await assert.rejects(get(port, "/v1/videos/aNXB-8Aqt88"), { code: "ECONNRESET" });
      },
    );
    assert.deepEqual(errors, []);
    assert.deepEqual(events, ["request_failed"]);
  });

  test("bad startup configuration is reported without its value", () => {
    const result = spawnSync(
      process.execPath,
      [fileURLToPath(new URL("server.mjs", import.meta.url))],
      {
        env: { ...process.env, PORT: "private-config-canary" },
        encoding: "utf8",
      },
    );
    assert.equal(result.status, 1);
    assert.equal(result.stderr, "");
    assert.deepEqual(JSON.parse(result.stdout), {
      message: "service_event",
      code: "service_failed",
    });
    assert.equal(result.stdout, '{"message": "service_event", "code": "service_failed"}\n');
  });
});

describe("routes", () => {
  test("a busy listing slot is turned away and a timed-out run is reported", async () => {
    let finish;
    const blocked = new Promise((resolve) => {
      finish = resolve;
    });
    await withService(
      {
        run: () => blocked,
      },
      async (port) => {
        const first = get(port, "/v1/channels/UCsXVk37bltHxD1rDPwtNM8Q/shorts");
        const second = await get(port, "/v1/channels/UCsXVk37bltHxD1rDPwtNM8Q/uploads/videos");
        assert.equal(second.status, 429);
        assert.deepEqual(JSON.parse(second.body), { error: "extractor is busy" });
        finish({ timedOut: true });
        const answered = await first;
        assert.equal(answered.status, 504);
        assert.deepEqual(JSON.parse(answered.body), { error: "extraction timed out" });
      },
    );
  });

  test("ids are validated and health says what is missing", async () => {
    await withService(
      {
        isProviderReady: async () => false,
        installedPluginVersion: () => "2.0.0",
        run: async () => ({ timedOut: false, returncode: 0, stdout: Buffer.from("{}") }),
      },
      async (port) => {
        assert.equal((await get(port, "/v1/videos/short")).status, 400);
        assert.equal((await get(port, "/v1/videos/aNXB-8Aqt88")).status, 503);
        assert.equal((await get(port, "/v1/channels/veritasium/shorts")).status, 400);
        const listing = await get(port, "/v1/channels/UCsXVk37bltHxD1rDPwtNM8Q/uploads/streams");
        assert.equal(listing.status, 200);
        assert.equal(listing.body, "{}");
        const bad = await get(port, "/v1/channels/UCsXVk37bltHxD1rDPwtNM8Q/uploads/members");
        assert.deepEqual(JSON.parse(bad.body), { error: "invalid channel listing" });
        const health = await get(port, "/health");
        assert.equal(health.status, 503);
        assert.deepEqual(JSON.parse(health.body), {
          ready: false,
          provider: false,
          plugin_version: "2.0.0",
        });
      },
    );
  });
});
