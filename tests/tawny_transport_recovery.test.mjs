import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import vm from "node:vm";

const transportScript = readFileSync(
  new URL("../assets/tawny_transport.js", import.meta.url),
  "utf8",
);

class Events {
  listeners = new Map();

  addEventListener(name, handler, options) {
    const listeners = this.listeners.get(name) || [];
    listeners.push({ handler, once: options?.once });
    this.listeners.set(name, listeners);
  }

  removeEventListener(name, handler) {
    this.listeners.set(
      name,
      (this.listeners.get(name) || []).filter((listener) => listener.handler !== handler),
    );
  }

  dispatchEvent(event) {
    for (const listener of [...(this.listeners.get(event.type) || [])]) {
      if (listener.once) this.removeEventListener(event.type, listener.handler);
      listener.handler(event);
    }
  }
}

function session(label = "original") {
  return {
    primary: {
      protocol: "Dash",
      tracks: ["Video", "Audio"].map((kind) => ({
        kind,
        url: `https://media.example/${label}/${kind}`,
        mime_type:
          kind === "Video" ? 'video/mp4; codecs="avc1.64002a"' : 'audio/mp4; codecs="mp4a.40.2"',
        bitrate: 128_000,
        duration_ms: 6_000_000,
        init_range: { start: 0, end: 10 },
        index_range: { start: 11, end: 20 },
      })),
    },
    alternatives: [],
  };
}

function harness(fetch, userAgent = "") {
  const document = Object.assign(new Events(), { visibilityState: "visible" });
  const video = Object.assign(new Events(), {
    currentTime: 120,
    duration: 6000,
    paused: true,
    isConnected: true,
    play() {
      this.paused = false;
      return Promise.resolve();
    },
  });
  const players = [];
  const manifests = new Map();
  const delays = [];
  class Player extends Events {
    static isBrowserSupported = () => true;
    constructor() {
      super();
      players.push(this);
    }
    configuration = {};
    variants = [];
    selections = [];
    configure(config) {
      Object.assign(this.configuration, config);
    }
    getNetworkingEngine = () => ({ registerRequestFilter() {}, registerResponseFilter() {} });
    attach = async () => {};
    load = async (uri, start) => {
      this.configurationAtLoad = structuredClone(this.configuration);
      this.manifest = manifests.has(uri) ? await manifests.get(uri).text() : uri;
      video.currentTime = start;
    };
    getVariantTracks = () => this.variants;
    selectVariantTrack = (...args) => this.selections.push(args);
    retryStreaming = () => true;
    destroy = async () => {
      this.destroyed = true;
      video.paused = true;
    };
  }
  const window = {
    navigator: { userAgent },
    shaka: {
      Player,
      polyfill: { installAll() {} },
      util: { Error: { Severity: { CRITICAL: 2 }, Category: { NETWORK: 1 } } },
    },
  };
  vm.runInNewContext(transportScript, {
    window,
    document,
    fetch,
    Blob,
    AbortSignal,
    URL: {
      createObjectURL(blob) {
        const uri = `blob:${manifests.size}`;
        manifests.set(uri, blob);
        return uri;
      },
      revokeObjectURL() {},
    },
    Event: class {
      constructor(type) {
        this.type = type;
      }
      stopImmediatePropagation() {}
    },
    CustomEvent: class {
      constructor(type, { detail } = {}) {
        this.type = type;
        this.detail = detail;
      }
    },
    setTimeout(resolve, delay) {
      delays.push(delay);
      resolve();
    },
  });
  const transport = window.TawnyTransport;
  const fail = () => video.dispatchEvent({ type: "error", stopImmediatePropagation() {} });
  const visible = () => {
    document.visibilityState = "visible";
    document.dispatchEvent({ type: "visibilitychange" });
  };
  const settle = async () => {
    for (let i = 0; i < 20; i++) await new Promise(setImmediate);
  };
  return { document, video, players, delays, transport, fail, visible, settle };
}

test("live HLS prefers the HD audio ladder before loading on every platform", async () => {
  for (const userAgent of ["Chrome", "Android", "iPhone"]) {
    const h = harness(undefined, userAgent);
    await h.transport.attach(
      h.video,
      {
        primary: {
          protocol: "Hls",
          url: "https://media.example/live.m3u8",
          mime_type: "application/vnd.apple.mpegurl",
        },
      },
      { preferredAudioLanguage: "es" },
    );
    const config = h.players[0].configurationAtLoad;
    assert.deepEqual(Array.from(config.preferredAudioCodecs), ["mp4a.40.2", "mp4a", "opus"]);
    assert.equal(config.preferredAudioLanguage, "es");
    assert.equal(config.manifest.hls.sequenceMode, false);
    assert.equal(config.abr.enabled, true);
    await h.transport.detach(h.video);
  }
});

test("on-demand playback keeps its platform codec preferences", async () => {
  for (const [userAgent, expected] of [
    ["Chrome", ["opus", "mp4a"]],
    ["Android", ["mp4a", "opus"]],
  ]) {
    const h = harness(undefined, userAgent);
    await h.transport.attach(h.video, session());
    assert.deepEqual(Array.from(h.players[0].configurationAtLoad.preferredAudioCodecs), expected);
    await h.transport.detach(h.video);
  }
});

test("quality switching retains audio identity and language before considering bitrate", async () => {
  const h = harness();
  await h.transport.attach(h.video, session());
  const player = h.players[0];
  const active = { id: 1, height: 240, audioId: 9, language: "en", active: true };
  const preferred = { id: 2, height: 1080, audioId: 9, language: "en", bandwidth: 3_000_000 };
  player.variants = [
    active,
    preferred,
    { id: 3, height: 1080, audioId: 10, language: "en", bandwidth: 4_000_000 },
    { id: 4, height: 1080, audioId: 11, language: "es", bandwidth: 5_000_000 },
    {
      id: 5,
      height: 1080,
      audioId: 9,
      language: "en",
      bandwidth: 6_000_000,
      allowedByKeySystem: false,
    },
  ];
  assert.equal(h.transport.setQuality(h.video, 1080), true);
  assert.equal(player.selections[0][0], preferred);
  assert.equal(player.selections[0][1], true);
  assert.equal(player.configuration.abr.enabled, false);
  assert.equal(h.transport.qualityState(h.video).selectedHeight, 1080);
  assert.equal(h.transport.setQuality(h.video, 2160), false);
  assert.equal(player.selections.length, 1);
  assert.equal(h.transport.setQuality(h.video, null), true);
  assert.equal(player.configuration.abr.enabled, true);
  assert.equal(h.transport.qualityState(h.video).auto, true);

  // The low-only HE-AAC track still has a path to HD when no variant there
  // carries its audio id, while retaining the selected language.
  active.audioId = 3;
  assert.equal(h.transport.setQuality(h.video, 1080), true);
  assert.equal(player.selections[1][0].id, 3);
  await h.transport.detach(h.video);
});

test("a failed stream renews as audio while hidden and restores the latest session at its position", async () => {
  let requests = 0;
  const h = harness(async () => ({ ok: true, json: async () => session(`renewed-${++requests}`) }));
  await h.transport.attach(h.video, session(), { videoId: "video", refreshUrl: "/renew" });
  assert.match(h.players[0].manifest, /contentType="video"/);

  h.document.visibilityState = "hidden";
  h.video.currentTime = 175;
  h.fail();
  await h.settle();
  assert.equal(h.players.length, 2);
  assert.doesNotMatch(h.players[1].manifest, /contentType="video"/);
  assert.match(h.players[1].manifest, /renewed-1\/Audio/);
  assert.equal(h.video.currentTime, 175);

  h.video.currentTime = 210;
  h.video.dispatchEvent({ type: "timeupdate" });
  h.fail();
  await h.settle();
  assert.equal(h.document.listeners.get("visibilitychange").length, 1);
  assert.doesNotMatch(h.players[2].manifest, /contentType="video"/);
  h.video.currentTime = 225;
  h.visible();
  await h.settle();
  assert.match(h.players[3].manifest, /contentType="video"/);
  assert.match(h.players[3].manifest, /renewed-2\/Video/);
  assert.equal(h.video.currentTime, 225);
  assert.equal(h.document.listeners.get("visibilitychange").length, 0);
  await h.transport.detach(h.video);
});

test("renewal retries a connection failure and a temporary server error without dropping the runtime", async () => {
  let requests = 0;
  let h;
  h = harness(async () => {
    assert.equal(h.players[0].destroyed, undefined);
    requests++;
    if (requests === 1) throw new TypeError("Failed to fetch");
    if (requests === 2) return { ok: false, status: 503 };
    h.video.currentTime = 135;
    return { ok: true, json: async () => session("reconnected") };
  });
  await h.transport.attach(h.video, session(), { refreshUrl: "/renew" });
  h.fail();
  await h.settle();
  assert.equal(requests, 3);
  assert.deepEqual(h.delays, [1000, 2000]);
  assert.match(h.players[1].manifest, /reconnected/);
  assert.equal(h.video.currentTime, 135);
  await h.transport.detach(h.video);
});

test("permanent renewal errors fail once and temporary failures have a bounded retry budget", async () => {
  for (const status of [401, 404, 503]) {
    let requests = 0;
    let exhausted = 0;
    const h = harness(async () => {
      requests++;
      return { ok: false, status };
    });
    h.video.addEventListener("tawnytransportexhausted", () => exhausted++);
    await h.transport.attach(h.video, session(), { refreshUrl: "/renew" });
    h.fail();
    await h.settle();
    assert.equal(requests, status === 503 ? 3 : 1);
    assert.equal(exhausted, 1);
    await h.transport.detach(h.video);
  }
});

test("an interrupted renewal response body is retried", async () => {
  let requests = 0;
  const h = harness(async () => ({
    ok: true,
    status: 200,
    json: async () => {
      if (++requests === 1) throw new TypeError("Connection closed while reading response");
      return session("complete");
    },
  }));
  await h.transport.attach(h.video, session(), { refreshUrl: "/renew" });
  h.fail();
  await h.settle();
  assert.equal(requests, 2);
  assert.match(h.players[1].manifest, /complete/);
  await h.transport.detach(h.video);
});

test("native renewal uses the authenticated resolver channel and disposes it with the player", async () => {
  const h = harness(() => assert.fail("Native renewal must not fetch from the WebView"));
  const messages = [];
  let answers = 0;
  const refreshSession = h.transport.nativeSessionRefresher(
    h.video,
    (message) => messages.push(message),
    async () => {
      if (++answers === 1) return { error: "Connection interrupted", retryable: true };
      return { session: session("authenticated") };
    },
  );
  await h.transport.attach(h.video, session(), { refreshSession });
  const generation = h.video.__tawnyGeneration;
  h.fail();
  await h.settle();
  assert.deepEqual(messages, ["refresh", "refresh"]);
  assert.equal(h.video.__tawnyGeneration, generation);
  assert.match(h.players[1].manifest, /authenticated/);
  await h.transport.detach(h.video);
  await refreshSession.closed;
  assert.deepEqual(messages, ["refresh", "refresh", "detached"]);
  assert.equal(h.video.__tawnyDisposeRefreshBridge, null);
  await assert.rejects(refreshSession(), /Playback changed/);
});

test("replacing a native refresh channel disposes the old one and permanent resolver errors are not retried", async () => {
  const h = harness();
  const messages = [];
  h.transport.nativeSessionRefresher(
    h.video,
    (message) => messages.push(`old:${message}`),
    async () => ({}),
  );
  const refreshSession = h.transport.nativeSessionRefresher(
    h.video,
    (message) => messages.push(`new:${message}`),
    async () => ({ error: "Video unavailable", retryable: false }),
  );
  let exhausted = 0;
  h.video.addEventListener("tawnytransportexhausted", () => exhausted++);
  await h.transport.attach(h.video, session(), { refreshSession });
  h.fail();
  await h.settle();
  assert.deepEqual(messages, ["old:detached", "new:refresh"]);
  assert.equal(exhausted, 1);
  await h.transport.detach(h.video);
  assert.deepEqual(messages, ["old:detached", "new:refresh", "new:detached"]);
});

test("detaching during native renewal keeps the reply channel alive until its stale answer is consumed", async () => {
  const h = harness();
  let answer;
  const messages = [];
  const refresh = h.transport.nativeSessionRefresher(
    h.video,
    (message) => messages.push(message),
    () =>
      new Promise((resolve) => {
        answer = resolve;
      }),
  );
  let closed = false;
  refresh.closed.then(() => {
    closed = true;
  });
  const pending = refresh();
  await h.transport.detach(h.video);
  await h.settle();
  assert.equal(closed, false);
  assert.deepEqual(messages, ["refresh", "detached"]);
  answer({ session: session("stale") });
  await assert.rejects(pending, /Playback changed/);
  await refresh.closed;
  assert.equal(closed, true);
});

test("a renewal finishing after detach cannot load or report errors for the old video", async () => {
  for (const succeeds of [true, false]) {
    let finish;
    let exhausted = 0;
    let requests = 0;
    const h = harness(() => {
      requests++;
      return new Promise((resolve, reject) => {
        finish = succeeds ? resolve : reject;
      });
    });
    h.video.addEventListener("tawnytransportexhausted", () => exhausted++);
    await h.transport.attach(h.video, session(), { refreshUrl: "/renew" });
    h.document.visibilityState = "hidden";
    h.fail();
    await h.transport.detach(h.video);
    finish(succeeds ? { ok: true, json: async () => session("stale") } : new TypeError("Offline"));
    await h.settle();
    assert.equal(h.players.length, 1);
    assert.equal(requests, 1);
    assert.equal(exhausted, 0);
  }
});

test("hidden attachment cleans up restoration on detach and respects explicit audio-only playback", async () => {
  for (const audioOnly of [false, true]) {
    const h = harness();
    h.document.visibilityState = "hidden";
    await h.transport.attach(h.video, session(), { audioOnly });
    assert.doesNotMatch(h.players[0].manifest, /contentType="video"/);
    await h.transport.detach(h.video);
    h.visible();
    await h.settle();
    assert.equal(h.players.length, 1);
    assert.equal(h.document.listeners.get("visibilitychange")?.length || 0, 0);
  }
});
