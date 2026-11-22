// Exporting a download as one ordinary video file, for other apps to open
// (Keyscribe reads piano videos from one).
//
// A download is Shaka's offline copy of a DASH stream: one video track and
// one audio track, each kept as an init segment (ftyp + moov) and its
// fragments (moof + mdat), in this origin's IndexedDB. YouTube's MP4
// fragments address their samples from their own moof (`default-base-is-moof`),
// so they stay valid wherever they sit in a file. The export writes one header
// that declares both tracks, then every fragment of both in time order, with
// only the track id and sequence number rewritten: a fragmented MP4 that
// players, editors and mediabunny all open. Nothing is decoded or re-encoded.
//
// The bytes leave the page in chunks, as they are read: on Android to the
// local relay, which writes them into Downloads (`src/video_export.rs`);
// elsewhere into a Blob the browser saves. A long video is hundreds of
// megabytes, which a phone's WebView cannot hold twice over.
(function (root) {
  "use strict";

  const DB_NAME = "shaka_offline_db";
  const MANIFEST_STORE = "manifest-v5";
  const SEGMENT_STORE = "segment-v5";
  // Batched before each write, so a long video is a few dozen requests to the
  // relay rather than one per fragment.
  const CHUNK_BYTES = 8 * 1024 * 1024;
  const TFHD_BASE_DATA_OFFSET = 0x000001;

  // --- MP4 boxes ----------------------------------------------------------

  function view(bytes) {
    return new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  }

  function fourcc(bytes, at) {
    return String.fromCharCode(bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]);
  }

  /// The boxes directly inside `bytes` from `start` to `end`.
  function boxes(bytes, start = 0, end = bytes.length) {
    const data = view(bytes);
    const found = [];
    let at = start;
    while (at + 8 <= end) {
      let size = data.getUint32(at);
      let header = 8;
      if (size === 1) {
        size = Number(data.getBigUint64(at + 8));
        header = 16;
      } else if (size === 0) {
        size = end - at;
      }
      if (size < header || at + size > end) throw new Error("This download is damaged");
      found.push({ type: fourcc(bytes, at + 4), start: at, size, header });
      at += size;
    }
    return found;
  }

  function child(bytes, parent, type) {
    return boxes(bytes, parent.start + parent.header, parent.start + parent.size).find(
      (box) => box.type === type,
    );
  }

  function slice(bytes, box) {
    return bytes.slice(box.start, box.start + box.size);
  }

  function concat(parts) {
    const out = new Uint8Array(parts.reduce((total, part) => total + part.length, 0));
    let at = 0;
    for (const part of parts) {
      out.set(part, at);
      at += part.length;
    }
    return out;
  }

  function box(type, parts) {
    const body = concat(parts);
    const out = new Uint8Array(8 + body.length);
    view(out).setUint32(0, out.length);
    for (let index = 0; index < 4; index++) out[4 + index] = type.charCodeAt(index);
    out.set(body, 8);
    return out;
  }

  /// What the header needs from one track's init segment.
  function readInit(bytes) {
    const top = boxes(bytes);
    const moov = top.find((entry) => entry.type === "moov");
    if (!moov) throw new Error("This download has no MP4 header");
    const ftyp = top.find((entry) => entry.type === "ftyp");
    const mvhd = child(bytes, moov, "mvhd");
    const trak = child(bytes, moov, "trak");
    const mvex = child(bytes, moov, "mvex");
    const trex = mvex && child(bytes, mvex, "trex");
    if (!mvhd || !trak || !trex) throw new Error("This download is not a fragmented MP4");
    const data = view(bytes);
    const version = bytes[mvhd.start + 8];
    const fields = mvhd.start + 12 + (version === 1 ? 16 : 8);
    const timescale = data.getUint32(fields);
    const duration =
      version === 1 ? Number(data.getBigUint64(fields + 4)) : data.getUint32(fields + 4);
    const brands = [];
    if (ftyp) {
      for (let at = ftyp.start + 16; at + 4 <= ftyp.start + ftyp.size; at += 4) {
        brands.push(fourcc(bytes, at));
      }
    }
    return {
      mvhd: slice(bytes, mvhd),
      trak: slice(bytes, trak),
      trex: slice(bytes, trex),
      timescale,
      seconds: timescale ? duration / timescale : 0,
      brands,
    };
  }

  function setTrackId(trak, id) {
    const tkhd = child(trak, boxes(trak)[0], "tkhd");
    if (!tkhd) throw new Error("This download's track has no header");
    const version = trak[tkhd.start + 8];
    view(trak).setUint32(tkhd.start + 12 + (version === 1 ? 16 : 8), id);
    return trak;
  }

  /// One ftyp + moov declaring every track, numbered from 1 in the order given.
  function buildHeader(inits) {
    if (!inits.length) throw new Error("Nothing to export");
    // `dash` describes how the file was served; the result is a plain
    // fragmented MP4.
    const brands = ["isom", "iso6", "mp41"];
    for (const brand of inits.flatMap((init) => init.brands)) {
      if (brand !== "dash" && !brands.includes(brand)) brands.push(brand);
    }
    const ftyp = box("ftyp", [
      new TextEncoder().encode("isom"),
      new Uint8Array([0, 0, 2, 0]),
      ...brands.map((brand) => new TextEncoder().encode(brand.padEnd(4, " ").slice(0, 4))),
    ]);

    // The first track's movie header, lasting as long as the longest track
    // and naming the next free track id.
    const mvhd = inits[0].mvhd.slice();
    const data = view(mvhd);
    const version = mvhd[8];
    const fields = 12 + (version === 1 ? 16 : 8);
    const timescale = inits[0].timescale || 1000;
    const longest = Math.round(Math.max(...inits.map((init) => init.seconds)) * timescale);
    if (version === 1) data.setBigUint64(fields + 4, BigInt(longest));
    else data.setUint32(fields + 4, Math.min(longest, 0xffffffff));
    data.setUint32(mvhd.length - 4, inits.length + 1);

    const traks = inits.map((init, index) => setTrackId(init.trak.slice(), index + 1));
    const trexes = inits.map((init, index) => {
      const trex = init.trex.slice();
      view(trex).setUint32(12, index + 1);
      return trex;
    });
    return concat([ftyp, box("moov", [mvhd, box("mvex", trexes), ...traks])]);
  }

  /// A stored segment's moof + mdat pairs, renumbered for `trackId` and
  /// numbered on from `sequence`, in place. Anything else in the segment
  /// (sidx, styp, emsg) belongs to how it was served and is left out.
  function retagFragment(bytes, trackId, sequence) {
    const parts = [];
    const data = view(bytes);
    for (const entry of boxes(bytes)) {
      if (entry.type === "mdat") {
        parts.push(bytes.subarray(entry.start, entry.start + entry.size));
        continue;
      }
      if (entry.type !== "moof") continue;
      for (const inner of boxes(bytes, entry.start + entry.header, entry.start + entry.size)) {
        if (inner.type === "mfhd") {
          data.setUint32(inner.start + 12, sequence);
          sequence += 1;
        } else if (inner.type === "traf") {
          const tfhd = child(bytes, inner, "tfhd");
          if (!tfhd) throw new Error("This download has a fragment without a track");
          // An absolute offset points into the file the fragment came from.
          // YouTube's never carry one; a source that does cannot be moved
          // without rewriting its sample offsets.
          if (data.getUint32(tfhd.start + 8) & TFHD_BASE_DATA_OFFSET) {
            throw new Error("This download's fragments cannot be moved into a new file");
          }
          data.setUint32(tfhd.start + 12, trackId);
        }
      }
      parts.push(bytes.subarray(entry.start, entry.start + entry.size));
    }
    return { parts, sequence };
  }

  /// Every segment of every track, earliest first. Ties go to the track given
  /// first, so video leads audio at each step.
  function interleave(tracks) {
    const order = [];
    tracks.forEach((track, trackIndex) => {
      track.segments.forEach((segment, segmentIndex) => {
        order.push({ trackIndex, segmentIndex, startTime: Number(segment.startTime) || 0 });
      });
    });
    return order.sort(
      (left, right) =>
        left.startTime - right.startTime ||
        left.trackIndex - right.trackIndex ||
        left.segmentIndex - right.segmentIndex,
    );
  }

  /// Writes the whole file to `sink.write(Uint8Array)`, reading each segment
  /// through `read(key)` only when it is due.
  async function writeMp4(tracks, read, sink) {
    const inits = [];
    for (const track of tracks) {
      inits.push(readInit(new Uint8Array(await read(track.initKey))));
    }
    let pending = [buildHeader(inits)];
    let pendingBytes = pending[0].length;
    let written = 0;
    const flush = async () => {
      if (!pendingBytes) return;
      const chunk = concat(pending);
      pending = [];
      pendingBytes = 0;
      await sink.write(chunk);
      written += chunk.length;
    };
    let sequence = 1;
    for (const step of interleave(tracks)) {
      const segment = tracks[step.trackIndex].segments[step.segmentIndex];
      const bytes = new Uint8Array(await read(segment.dataKey));
      const retagged = retagFragment(bytes, step.trackIndex + 1, sequence);
      sequence = retagged.sequence;
      for (const part of retagged.parts) {
        pending.push(part);
        pendingBytes += part.length;
      }
      if (pendingBytes >= CHUNK_BYTES) await flush();
    }
    await flush();
    return written;
  }

  // --- Shaka's offline storage ---------------------------------------------

  function request(operation) {
    return new Promise((resolve, reject) => {
      operation.onsuccess = () => resolve(operation.result);
      operation.onerror = () => reject(operation.error);
    });
  }

  function openDatabase() {
    // No version: whatever Shaka created. Asking for one could start an
    // upgrade under the player's feet.
    const opening = root.indexedDB.open(DB_NAME);
    return request(opening);
  }

  function get(db, store, key) {
    return request(db.transaction(store, "readonly").objectStore(store).get(key));
  }

  /// The stored tracks of a finished download, video first.
  function tracksOf(manifest) {
    const streams = (manifest?.streams || []).filter(
      (stream) => (stream.type === "video" || stream.type === "audio") && stream.segments?.length,
    );
    streams.sort(
      (left, right) => (left.type === "video" ? 0 : 1) - (right.type === "video" ? 0 : 1),
    );
    return streams.map((stream) => {
      const mimeType = String(stream.mimeType || "");
      if (!/\/mp4$/i.test(mimeType)) {
        throw new Error(
          `Only MP4 downloads can be exported yet, and this one is ${mimeType || "another format"}`,
        );
      }
      const initKey = stream.segments[0].initSegmentKey;
      if (initKey === null || initKey === undefined) {
        throw new Error("This download has no MP4 header");
      }
      return {
        type: stream.type,
        initKey,
        segments: stream.segments.map((segment) => ({
          dataKey: segment.dataKey,
          startTime: segment.startTime,
        })),
      };
    });
  }

  /// The `manifest-v5` key in an offline URI such as
  /// `offline:manifest/idb/v5/3`, or null for another storage version. The
  /// mechanism is registered as `idb`, not as its source's `indexeddb`.
  function manifestKey(uri) {
    const key = /^offline:manifest\/[^/]+\/v5\/(\d+)$/.exec(String(uri || ""))?.[1];
    return key === undefined ? null : Number(key);
  }

  async function exportDownload(videoId, sink) {
    const uri = await root.TawnyDownloads?.offlineUri(videoId);
    if (!uri) throw new Error("This video is not downloaded");
    const key = manifestKey(uri);
    if (key === null) throw new Error("This download was kept in an older format");
    const db = await openDatabase();
    try {
      const manifest = await get(db, MANIFEST_STORE, key);
      if (!manifest || manifest.isIncomplete) throw new Error("This download is not finished");
      const tracks = tracksOf(manifest);
      if (!tracks.length) throw new Error("This download has nothing to export");
      const read = async (dataKey) => {
        const record = await get(db, SEGMENT_STORE, dataKey);
        if (!record?.data) throw new Error("Part of this download is missing");
        return record.data;
      };
      const bytes = await writeMp4(tracks, read, sink);
      return { bytes, audioOnly: !tracks.some((track) => track.type === "video") };
    } finally {
      db.close();
    }
  }

  // --- Where the file goes -------------------------------------------------

  /// Chunks to the phone's local relay, which writes them into the file it
  /// opened in Downloads. The offset lets it refuse a chunk out of place
  /// rather than write a corrupt file.
  function uploadSink(url) {
    let offset = 0;
    return {
      async write(chunk) {
        const response = await fetch(`${url}?offset=${offset}`, {
          method: "PUT",
          body: chunk,
          headers: { "content-type": "application/octet-stream" },
        });
        if (!response.ok) throw new Error(`Could not write the file (${response.status})`);
        offset += chunk.length;
      },
    };
  }

  /// A Blob the browser saves under `fileName`. Kept as nested Blobs so each
  /// chunk can be let go of once it is in one.
  async function saveInBrowser(videoId, fileName, mimeType) {
    let blob = new Blob([], { type: mimeType });
    const result = await exportDownload(videoId, {
      async write(chunk) {
        blob = new Blob([blob, chunk], { type: mimeType });
      },
    });
    const url = URL.createObjectURL(blob);
    const anchor = document.createElement("a");
    anchor.href = url;
    anchor.download = fileName;
    document.body.appendChild(anchor);
    anchor.click();
    anchor.remove();
    setTimeout(() => URL.revokeObjectURL(url), 60_000);
    return result;
  }

  root.TawnyExport = {
    exportToUpload: (videoId, url) => exportDownload(videoId, uploadSink(url)),
    saveInBrowser,
    // For the tests.
    boxes,
    readInit,
    buildHeader,
    retagFragment,
    interleave,
    tracksOf,
    manifestKey,
    writeMp4,
  };
})(typeof window !== "undefined" ? window : globalThis);
