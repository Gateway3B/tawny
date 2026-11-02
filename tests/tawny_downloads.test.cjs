const assert = require("node:assert/strict");
const test = require("node:test");

global.window = {};
require("../assets/tawny_downloads.js");

const { pickTracks } = window.TawnyDownloads;

function track(kind, mime, extra) {
  return {
    kind,
    url: `http://localhost/${kind}/${mime}`,
    mime_type: mime,
    init_range: { start: 0, end: 99 },
    index_range: { start: 100, end: 199 },
    ...extra,
  };
}

const avc = (height, bitrate) =>
  track("Video", 'video/mp4; codecs="avc1.64002a"', {
    width: Math.round((height * 16) / 9),
    height,
    bitrate,
  });
const vp9 = (height, bitrate) =>
  track("Video", 'video/webm; codecs="vp09.00.40.08"', {
    width: Math.round((height * 16) / 9),
    height,
    bitrate,
  });
const aac = (extra) =>
  track("Audio", 'audio/mp4; codecs="mp4a.40.2"', { bitrate: 128_000, ...extra });
const opus = (extra) => track("Audio", 'audio/webm; codecs="opus"', { bitrate: 160_000, ...extra });

const ladder = [
  avc(1080, 4_000_000),
  avc(720, 2_000_000),
  avc(360, 600_000),
  vp9(2160, 12_000_000),
  vp9(1080, 3_000_000),
  vp9(720, 1_500_000),
  aac({ language: "en", is_default: true }),
  opus({ language: "en", is_default: true }),
  aac({ language: "it", label: "Italian" }),
];

const android = "Mozilla/5.0 (Linux; Android 14) Chrome/130";
const desktop = "Mozilla/5.0 (X11; Linux x86_64) Chrome/130";

test("takes the highest quality at or under the one asked for", () => {
  const picked = pickTracks(ladder, "720", { userAgent: android });
  assert.equal(picked.video.height, 720);
});

test("prefers the codec the device plays best", () => {
  assert.match(pickTracks(ladder, "best", { userAgent: android }).video.mime_type, /avc1/);
  assert.match(pickTracks(ladder, "best", { userAgent: desktop }).video.mime_type, /vp09/);
  assert.match(pickTracks(ladder, "best", { userAgent: android }).audio.mime_type, /mp4a/);
  assert.match(pickTracks(ladder, "best", { userAgent: desktop }).audio.mime_type, /opus/);
});

// The player streams only the preferred codec family, so a phone's Best is
// AVC's 1080p, not the 4K VP9 it would never play.
test("picks the codec before the height, as the player does", () => {
  const phone = pickTracks(ladder, "best", { userAgent: android }).video;
  assert.match(phone.mime_type, /avc1/);
  assert.equal(phone.height, 1080);
  const computer = pickTracks(ladder, "best", { userAgent: desktop }).video;
  assert.match(computer.mime_type, /vp09/);
  assert.equal(computer.height, 2160);
});

test("falls back to the smallest when nothing is that small", () => {
  const picked = pickTracks(ladder, "144", { userAgent: android });
  assert.equal(picked.video.height, 360);
});

test("keeps the original audio rather than a dub", () => {
  const dubbedFirst = [ladder[7], ...ladder.slice(0, 7)];
  const picked = pickTracks(dubbedFirst, "best", { userAgent: android });
  assert.equal(picked.audio.language, "en");
});

test("audio only keeps no video", () => {
  const picked = pickTracks(ladder, "audio", { userAgent: android });
  assert.equal(picked.video, null);
  assert.equal(picked.audio.language, "en");
});

test("a portrait Short is measured by its shorter side", () => {
  const short = [
    track("Video", 'video/mp4; codecs="avc1.64002a"', { width: 1080, height: 1920 }),
    track("Video", 'video/mp4; codecs="avc1.64002a"', { width: 720, height: 1280 }),
    aac({ is_default: true }),
  ];
  assert.equal(pickTracks(short, "720", { userAgent: android }).video.width, 720);
});

test("tracks without byte ranges cannot be kept", () => {
  const unindexed = ladder.map((entry) => ({ ...entry, index_range: null }));
  assert.equal(pickTracks(unindexed, "best", { userAgent: android }), null);
});
