const assert = require("node:assert/strict");

global.window = {};
require("../assets/tawny_transport.js");

const source = {
  protocol: "Dash",
  tracks: [
    {
      kind: "Video",
      url: "http://localhost/video?x=1&y=2",
      mime_type: 'video/mp4; codecs="avc1.64002a"',
      bitrate: 2_500_000,
      duration_ms: 125_000,
      width: 1920,
      height: 1080,
      fps: 60,
      init_range: { start: 0, end: 739 },
      index_range: { start: 740, end: 1127 },
    },
    {
      kind: "Audio",
      url: "http://localhost/audio",
      mime_type: 'audio/mp4; codecs="mp4a.40.2"',
      bitrate: 128_000,
      duration_ms: 125_000,
      language: "en-US",
      label: "English",
      is_default: true,
      init_range: { start: 0, end: 719 },
      index_range: { start: 720, end: 1043 },
    },
  ],
};

const manifest = window.TawnyTransport.buildDashManifest(source);
assert.match(manifest, /mediaPresentationDuration="PT125S"/);
assert.match(manifest, /indexRange="740-1127"/);
assert.match(manifest, /indexRange="720-1043"/);
assert.match(manifest, /Initialization range="0-719"/);
assert.match(manifest, /codecs="avc1\.64002a"/);
assert.match(manifest, /lang="en-US"/);
assert.match(manifest, /value="main"/);
assert.match(manifest, /video\?x=1&amp;y=2/);
assert.equal(window.TawnyTransport.transportKind(source), "generated-dash");

const mobileSession = window.TawnyTransport.normalizePlaybackSession(
  {
    primary: source,
    alternatives: [],
  },
  { serverUrl: "http://127.0.0.1:8080" },
);
assert.equal(
  mobileSession.primary.tracks[0].url,
  "http://localhost/video?x=1&y=2",
  "third-party media URLs are not rewritten",
);
assert.equal(
  window.TawnyTransport.normalizePlaybackUrl(
    "http://localhost:8080/api/v1/playback/proxy/token?part=1",
    { serverUrl: "http://127.0.0.1:8080" },
  ),
  "http://127.0.0.1:8080/api/v1/playback/proxy/token?part=1",
);

const mixedManifest = window.TawnyTransport.buildDashManifest({
  ...source,
  tracks: [
    ...source.tracks,
    {
      ...source.tracks[0],
      url: "http://localhost/video-vp9",
      mime_type: 'video/webm; codecs="vp09.00.40.08"',
      bitrate: 1_800_000,
    },
    {
      ...source.tracks[1],
      url: "http://localhost/audio-opus",
      mime_type: 'audio/webm; codecs="opus"',
      bitrate: 96_000,
    },
    {
      ...source.tracks[1],
      url: "http://localhost/audio-he-aac",
      mime_type: 'audio/mp4; codecs="mp4a.40.5"',
      bitrate: 48_000,
    },
  ],
});
assert.equal((mixedManifest.match(/contentType="video"/g) || []).length, 2);
assert.equal((mixedManifest.match(/contentType="audio"/g) || []).length, 2);
assert.doesNotMatch(mixedManifest, /mp4a\.40\.5/);
const adaptationSets = mixedManifest.match(/<AdaptationSet[^>]*>.*?<\/AdaptationSet>/g);
for (const adaptationSet of adaptationSets) {
  assert.ok(!(adaptationSet.includes("avc1") && adaptationSet.includes("vp09")));
  assert.ok(!(adaptationSet.includes("mp4a") && adaptationSet.includes("opus")));
  assert.ok(!(adaptationSet.includes("mp4a.40.2") && adaptationSet.includes("mp4a.40.5")));
}

assert.throws(
  () => window.TawnyTransport.buildDashManifest({ tracks: [source.tracks[0]] }),
  /both video and audio/,
);

// A renewed session that plays on earns another renewal; one that fails
// straight away does not, so a broken video cannot renew in a loop.
const { restoresRefresh } = window.TawnyTransport;
assert.equal(restoresRefresh({ refreshAttempts: 1 }, 600, 629), false);
assert.equal(restoresRefresh({ refreshAttempts: 1 }, 600, 630), true);
assert.equal(restoresRefresh({ refreshAttempts: 0 }, 600, 900), false);
assert.equal(restoresRefresh({}, 0, 900), false);
