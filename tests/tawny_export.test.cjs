const assert = require("node:assert/strict");
const { readFileSync } = require("node:fs");
const path = require("node:path");
const test = require("node:test");

global.window = {};
require("../assets/tawny_export.js");

const { boxes, manifestKey, retagFragment, tracksOf, writeMp4 } = window.TawnyExport;

// Fragmented MP4s made with ffmpeg (testsrc and a sine tone), shaped like
// YouTube's DASH files: ftyp + moov, then moof + mdat pairs.
const FIXTURES = path.join(__dirname, "fixtures");

/// A file split the way Shaka keeps a download: the init segment, and one
/// stored segment per moof + mdat, under keys from `firstKey`.
function asStored(name, type, firstKey, store, secondsPerSegment) {
  const bytes = new Uint8Array(readFileSync(path.join(FIXTURES, name)));
  const top = boxes(bytes);
  const moov = top.find((box) => box.type === "moov");
  store.set(firstKey, bytes.slice(0, moov.start + moov.size).buffer);
  const segments = [];
  top.forEach((box, index) => {
    if (box.type !== "moof") return;
    const mdat = top[index + 1];
    const key = firstKey + 1 + segments.length;
    store.set(key, bytes.slice(box.start, mdat.start + mdat.size).buffer);
    segments.push({ dataKey: key, startTime: segments.length * secondsPerSegment });
  });
  return { type, initKey: firstKey, segments };
}

async function exportOf(tracks, store) {
  const chunks = [];
  await writeMp4(tracks, async (key) => store.get(key), {
    write: async (chunk) => chunks.push(chunk),
  });
  const out = new Uint8Array(chunks.reduce((total, chunk) => total + chunk.length, 0));
  let at = 0;
  for (const chunk of chunks) {
    out.set(chunk, at);
    at += chunk.length;
  }
  return out;
}

function inside(bytes, parent, type) {
  return boxes(bytes, parent.start + parent.header, parent.start + parent.size).filter(
    (box) => box.type === type,
  );
}

const u32 = (bytes, at) => new DataView(bytes.buffer, bytes.byteOffset).getUint32(at);

function fragments(bytes) {
  return boxes(bytes)
    .filter((box) => box.type === "moof")
    .map((moof) => {
      const mfhd = inside(bytes, moof, "mfhd")[0];
      const traf = inside(bytes, moof, "traf")[0];
      const tfhd = inside(bytes, traf, "tfhd")[0];
      return { sequence: u32(bytes, mfhd.start + 12), track: u32(bytes, tfhd.start + 12) };
    });
}

test("video and audio become one file with two tracks and every fragment", async () => {
  const store = new Map();
  const video = asStored("export_video.mp4", "video", 100, store, 1);
  const audio = asStored("export_audio.m4a", "audio", 200, store, 1);
  const out = await exportOf([video, audio], store);

  const top = boxes(out);
  assert.deepEqual(
    top.slice(0, 2).map((box) => box.type),
    ["ftyp", "moov"],
  );
  assert.equal(String.fromCharCode(...out.subarray(8, 12)), "isom");
  const moov = top[1];
  const traks = inside(out, moov, "trak");
  const ids = traks.map((trak) => {
    const tkhd = inside(out, trak, "tkhd")[0];
    return u32(out, tkhd.start + 20);
  });
  assert.deepEqual(ids, [1, 2]);
  const mvex = inside(out, moov, "mvex")[0];
  assert.deepEqual(
    inside(out, mvex, "trex").map((trex) => u32(out, trex.start + 12)),
    [1, 2],
  );
  const mvhd = inside(out, moov, "mvhd")[0];
  assert.equal(u32(out, mvhd.start + mvhd.size - 4), 3, "the next free track id");

  const found = fragments(out);
  assert.equal(found.length, video.segments.length + audio.segments.length);
  assert.deepEqual(
    found.map((fragment) => fragment.sequence),
    found.map((_, index) => index + 1),
  );
  // Video leads at each moment, then the audio from the same time.
  assert.deepEqual(
    found.slice(0, 2).map((fragment) => fragment.track),
    [1, 2],
  );
  assert.equal(found.filter((fragment) => fragment.track === 1).length, video.segments.length);

  // The media itself is carried over untouched.
  const mdatBytes = (bytes) =>
    boxes(bytes)
      .filter((box) => box.type === "mdat")
      .reduce((total, box) => total + box.size, 0);
  const sourceMdat =
    mdatBytes(new Uint8Array(readFileSync(path.join(FIXTURES, "export_video.mp4")))) +
    mdatBytes(new Uint8Array(readFileSync(path.join(FIXTURES, "export_audio.m4a"))));
  assert.equal(mdatBytes(out), sourceMdat);
});

test("an audio-only download exports as a single-track file", async () => {
  const store = new Map();
  const audio = asStored("export_audio.m4a", "audio", 200, store, 1);
  const out = await exportOf([audio], store);
  const moov = boxes(out).find((box) => box.type === "moov");
  assert.equal(inside(out, moov, "trak").length, 1);
  assert.ok(fragments(out).every((fragment) => fragment.track === 1));
});

test("a fragment with an absolute data offset is refused, not corrupted", () => {
  const store = new Map();
  const video = asStored("export_video.mp4", "video", 100, store, 1);
  const segment = new Uint8Array(store.get(video.segments[0].dataKey));
  const moof = boxes(segment)[0];
  const traf = inside(segment, moof, "traf")[0];
  const tfhd = inside(segment, traf, "tfhd")[0];
  segment[tfhd.start + 11] |= 0x01;
  assert.throws(() => retagFragment(segment, 1, 1), /cannot be moved/);
});

test("the manifest key comes from Shaka's offline URI as Shaka writes it", () => {
  // What Shaka 4.16 hands back for a download: the mechanism is `idb`.
  assert.equal(manifestKey("offline:manifest/idb/v5/1"), 1);
  assert.equal(manifestKey("offline:manifest/indexeddb/v5/27"), 27);
  assert.equal(manifestKey("offline:manifest/idb/v3/1"), null, "an older storage version");
  assert.equal(manifestKey("offline:segment/idb/v5/1"), null);
  assert.equal(manifestKey(null), null);
});

test("stored streams are read video first, and only MP4 is accepted", () => {
  const segment = { initSegmentKey: 1, dataKey: 2, startTime: 0 };
  const tracks = tracksOf({
    streams: [
      { type: "audio", mimeType: "audio/mp4", segments: [segment] },
      { type: "text", mimeType: "text/vtt", segments: [segment] },
      { type: "video", mimeType: "video/mp4", segments: [segment] },
    ],
  });
  assert.deepEqual(
    tracks.map((track) => track.type),
    ["video", "audio"],
  );
  assert.throws(
    () => tracksOf({ streams: [{ type: "video", mimeType: "video/webm", segments: [segment] }] }),
    /Only MP4 downloads can be exported yet, and this one is video\/webm/,
  );
});
