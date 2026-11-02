// Videos kept on this device for watching without a connection.
//
// Shaka's offline storage does the work: it fetches a manifest's segments
// through the same proxy the player streams from and keeps them in this
// origin's IndexedDB, and the player loads the `offline:` URI it hands back
// as it would any manifest. So a download is the player's own adaptive
// stream, narrowed to one video and one audio track, and plays through the
// same transport, controls and Shaka build.
//
// Downloads live in the page, not in Rust: a native build's Rust event
// handlers block the WebView, and Shaka's storage is only reachable from here.
// Rust starts, lists and removes them through `window.TawnyDownloads`.
(function () {
  "use strict";

  // In-flight and failed downloads, by video id. A finished one is only in
  // Shaka's storage, so a reload never shows a stale copy of it here.
  const jobs = new Map();
  let notifyTimer = null;

  const AUDIO_ONLY = "audio";

  function userAgent() {
    return window.navigator?.userAgent || "";
  }

  // The same codec order the player asks Shaka for, so a download is in the
  // format this device plays best rather than whatever came first.
  function platformVideoCodecs(agent) {
    if (/Macintosh|iPhone|iPad|iPod/i.test(agent)) return ["avc1", "hvc1", "hev1", "vp09", "av01"];
    if (/Android/i.test(agent)) return ["avc1", "vp09", "av01", "hvc1", "hev1"];
    return ["vp09", "av01", "avc1", "hvc1", "hev1"];
  }

  function platformAudioCodecs(agent) {
    return /Macintosh|iPhone|iPad|iPod|Android/i.test(agent) ? ["mp4a", "opus"] : ["opus", "mp4a"];
  }

  function codecsOf(track) {
    const match = /codecs=["']?([^"']+)["']?/i.exec(track.mime_type || "");
    return match ? match[1].trim().toLowerCase() : "";
  }

  function codecRank(track, order) {
    const codec = codecsOf(track);
    const index = order.findIndex((family) => codec.startsWith(family));
    return index < 0 ? order.length : index;
  }

  // The shorter side, so a portrait Short's 1080x1920 counts as 1080p, as the
  // quality menu reads it.
  function resolution(track) {
    return track.width && track.height ? Math.min(track.width, track.height) : track.height || 0;
  }

  function playable(track) {
    const MediaSource = window.MediaSource || window.ManagedMediaSource;
    if (!MediaSource || typeof MediaSource.isTypeSupported !== "function") return true;
    try {
      return MediaSource.isTypeSupported(track.mime_type);
    } catch (_) {
      return true;
    }
  }

  function indexed(track) {
    return Boolean(track.init_range && track.index_range);
  }

  /// The one video and one audio track to keep for `quality`: a height, or
  /// "best", or "audio". Null when the source has nothing usable.
  function pickTracks(tracks, quality, options = {}) {
    const agent = options.userAgent ?? userAgent();
    const preferredLanguage = String(options.preferredAudioLanguage || "en").toLowerCase();
    const usable = (tracks || []).filter((track) => indexed(track) && playable(track));
    const audios = usable.filter((track) => track.kind === "Audio");
    const videos = usable.filter((track) => track.kind === "Video");
    if (!audios.length) return null;

    // The original audio, not a dub: YouTube marks it as the default. Failing
    // that, the viewer's language, then whatever there is.
    const language = (track) => (track.language || "und").toLowerCase();
    const main =
      audios.find((track) => track.is_default) ||
      audios.find((track) => language(track).startsWith(preferredLanguage)) ||
      audios[0];
    const audioOrder = platformAudioCodecs(agent);
    const audio = audios
      .filter(
        (track) => language(track) === language(main) && (track.label || "") === (main.label || ""),
      )
      .sort(
        (left, right) =>
          codecRank(left, audioOrder) - codecRank(right, audioOrder) ||
          (right.bitrate || 0) - (left.bitrate || 0),
      )[0];

    if (quality === AUDIO_ONLY) return { video: null, audio };
    if (!videos.length) return null;

    // The codec first, then the height within it, as Shaka's
    // `preferredVideoCodecs` narrows what the player streams. A phone streams
    // AVC, which stops at 1080p; keeping YouTube's 4K VP9 for it instead
    // would be a file the player never chooses and the phone may not decode
    // smoothly.
    const videoOrder = platformVideoCodecs(agent);
    const family =
      videoOrder
        .map((codec) => videos.filter((track) => codecsOf(track).startsWith(codec)))
        .find((group) => group.length) || videos;
    const cap = quality === "best" ? Number.POSITIVE_INFINITY : Number(quality);
    const within = family.filter((track) => resolution(track) <= cap);
    // Nothing that small: the smallest there is, rather than nothing.
    const lowest = Math.min(...family.map(resolution));
    const pool = within.length ? within : family.filter((track) => resolution(track) === lowest);
    const height = Math.max(...pool.map(resolution));
    const video = pool
      .filter((track) => resolution(track) === height)
      .sort(
        (left, right) =>
          (right.fps || 0) - (left.fps || 0) || (right.bitrate || 0) - (left.bitrate || 0),
      )[0];
    return { video, audio };
  }

  function qualityLabel(picked) {
    if (!picked.video) return "Audio only";
    return `${resolution(picked.video)}p`;
  }

  function notify() {
    if (notifyTimer) return;
    // Progress arrives once per segment, several times a second; the screen
    // only needs a few updates a second.
    notifyTimer = setTimeout(() => {
      notifyTimer = null;
      window.dispatchEvent(new CustomEvent("tawnydownloadschange"));
    }, 250);
  }

  function supported() {
    return Boolean(window.shaka?.offline?.Storage?.support?.());
  }

  async function waitForShaka() {
    for (let attempt = 0; attempt < 200 && !(window.shaka && window.TawnyTransport); attempt++) {
      await new Promise((resolve) => setTimeout(resolve, 25));
    }
    if (!window.shaka || !window.TawnyTransport) throw new Error("The player has not loaded yet");
  }

  function retryParameters() {
    return {
      maxAttempts: 5,
      baseDelay: 1000,
      backoffFactor: 2,
      fuzzFactor: 0.5,
      timeout: 0,
      stallTimeout: 15000,
      connectionTimeout: 15000,
    };
  }

  function newStorage(callbacks) {
    const storage = new window.shaka.offline.Storage();
    storage.configure({
      manifest: { retryParameters: retryParameters() },
      streaming: { retryParameters: retryParameters() },
      offline: {
        usePersistentLicense: false,
        progressCallback: callbacks.progress || (() => {}),
        trackSelectionCallback: callbacks.selectTracks || ((tracks) => tracks),
      },
    });
    return storage;
  }

  async function storedContents() {
    if (!supported()) return [];
    const storage = newStorage({});
    try {
      return (await storage.list()).filter((content) => content.appMetadata?.videoId);
    } finally {
      await storage.destroy().catch(() => {});
    }
  }

  // The artwork, kept with the download so the list has a picture offline.
  // Best effort: a host that refuses CORS just leaves it out.
  async function thumbnailDataUrl(url) {
    if (!url) return null;
    try {
      const response = await fetch(url, { cache: "force-cache" });
      if (!response.ok) return null;
      const blob = await response.blob();
      if (blob.size > 512 * 1024 || !blob.type.startsWith("image/")) return null;
      return await new Promise((resolve) => {
        const reader = new FileReader();
        reader.onload = () => resolve(typeof reader.result === "string" ? reader.result : null);
        reader.onerror = () => resolve(null);
        reader.readAsDataURL(blob);
      });
    } catch (_) {
      return null;
    }
  }

  function storageError(error) {
    const code = error?.code;
    if (code === 7001) return "Cancelled";
    // QUOTA_EXCEEDED, from IndexedDB or the browser's storage manager.
    if (code === 9014 || error?.name === "QuotaExceededError") {
      return "There is not enough storage on this device";
    }
    if (error?.category === 1) {
      return "The download was cut off. Check the connection and try again.";
    }
    return error?.message || (code ? `Shaka error ${code}` : String(error));
  }

  // A failed or cancelled store can leave segments behind under an
  // incomplete record; nothing will ever finish it, so it goes.
  async function purgeIncomplete(videoId) {
    const storage = newStorage({});
    try {
      for (const content of await storage.list()) {
        if (content.appMetadata?.videoId === videoId && content.isIncomplete) {
          await storage.remove(content.offlineUri).catch(() => {});
        }
      }
    } finally {
      await storage.destroy().catch(() => {});
    }
  }

  /// Starts keeping `videoId` on this device. Resolves once the download is
  /// under way, not when it finishes; `list()` follows it from there.
  async function start(request) {
    await waitForShaka();
    if (!supported()) throw new Error("This browser cannot keep videos offline");
    const { videoId, quality, video } = request;
    if (jobs.get(videoId)?.state === "downloading") return;
    if ((await offlineUri(videoId)) !== null) return;

    const options = { serverUrl: request.serverUrl };
    const session = window.TawnyTransport.normalizePlaybackSession(request.session, options);
    const sources = [session.primary, ...(session.alternatives || [])].filter(Boolean);
    const adaptive = sources.find((source) => (source.tracks || []).length);
    const hls = sources.find((source) => window.TawnyTransport.transportKind(source) === "hls");

    let manifestUri;
    let manifestMime;
    let label;
    let selectTracks;
    let objectUrl = null;
    if (adaptive) {
      const picked = pickTracks(adaptive.tracks, quality, request);
      if (!picked) throw new Error("No track of this video can be kept offline");
      const tracks = [picked.video, picked.audio].filter(Boolean);
      const manifest = window.TawnyTransport.buildDashManifest(
        { ...adaptive, tracks },
        !picked.video,
      );
      objectUrl = URL.createObjectURL(new Blob([manifest], { type: "application/dash+xml" }));
      manifestUri = objectUrl;
      manifestMime = "application/dash+xml";
      label = qualityLabel(picked);
    } else if (hls) {
      // A video yt-dlp only offers as HLS: Shaka picks the variant closest
      // under the asked height from the master playlist.
      manifestUri = hls.url;
      manifestMime = hls.mime_type || "application/x-mpegurl";
      label = quality === AUDIO_ONLY ? "Audio only" : quality === "best" ? "Best" : `${quality}p`;
      selectTracks = (tracks) => {
        const variants = tracks.filter((track) => track.type === "variant");
        if (!variants.length) return tracks;
        const height = (track) => resolution(track);
        const cap =
          quality === AUDIO_ONLY
            ? 0
            : quality === "best"
              ? Number.POSITIVE_INFINITY
              : Number(quality);
        const within = variants.filter((track) => height(track) <= cap);
        const pool = within.length
          ? within
          : variants.filter((track) => height(track) === Math.min(...variants.map(height)));
        const chosen = pool.sort(
          (left, right) => height(right) - height(left) || right.bandwidth - left.bandwidth,
        )[0];
        if (quality !== AUDIO_ONLY) label = `${height(chosen)}p`;
        return [chosen];
      };
    } else {
      throw new Error("This video is not offered in a format that can be kept offline");
    }

    const job = {
      videoId,
      state: "downloading",
      progress: 0,
      bytes: 0,
      quality: label,
      video,
      thumbnail: null,
      error: null,
      operation: null,
    };
    jobs.set(videoId, job);
    notify();

    // Asked for once a download is wanted, so the browser does not clear
    // these videos when it runs short of space. Refusal changes nothing else.
    window.navigator?.storage?.persist?.().catch?.(() => {});

    const storage = newStorage({
      progress: (content, progress) => {
        job.progress = Math.max(job.progress, Number(progress) || 0);
        job.bytes = Number(content?.size) || job.bytes;
        notify();
      },
      selectTracks,
    });
    job.thumbnail = await thumbnailDataUrl(video?.thumbnail_url);
    const run = async () => {
      try {
        job.operation = storage.store(
          manifestUri,
          {
            videoId,
            quality: label,
            video,
            thumbnail: job.thumbnail,
            storedAt: Date.now(),
          },
          manifestMime,
        );
        await job.operation.promise;
        jobs.delete(videoId);
      } catch (error) {
        const cancelled = error?.code === 7001;
        if (cancelled) {
          jobs.delete(videoId);
        } else {
          job.state = "failed";
          job.error = storageError(error);
          console.warn("Tawny download failed", videoId, error);
        }
        await purgeIncomplete(videoId).catch(() => {});
      } finally {
        job.operation = null;
        if (objectUrl) URL.revokeObjectURL(objectUrl);
        await storage.destroy().catch(() => {});
        notify();
      }
    };
    run();
  }

  /// Every download this device knows of: kept, under way, or failed.
  async function list() {
    const entries = new Map();
    let contents = [];
    try {
      contents = await storedContents();
    } catch (error) {
      console.warn("Tawny could not read its downloads", error);
    }
    for (const content of contents) {
      const meta = content.appMetadata;
      entries.set(meta.videoId, {
        video_id: meta.videoId,
        // A record still marked incomplete with no download running here was
        // cut off by the app closing.
        state: content.isIncomplete ? "failed" : "done",
        progress: content.isIncomplete ? 0 : 1,
        bytes: Number(content.size) || 0,
        quality: meta.quality || "",
        video: meta.video || null,
        thumbnail: meta.thumbnail || null,
        error: content.isIncomplete ? "The download was interrupted" : null,
        stored_at: Number(meta.storedAt) || 0,
      });
    }
    for (const job of jobs.values()) {
      entries.set(job.videoId, {
        video_id: job.videoId,
        state: job.state,
        progress: job.progress,
        bytes: job.bytes,
        quality: job.quality || "",
        video: job.video || null,
        thumbnail: job.thumbnail,
        error: job.error,
        stored_at: Number.MAX_SAFE_INTEGER,
      });
    }
    return Array.from(entries.values()).sort((left, right) => right.stored_at - left.stored_at);
  }

  /// Cancels a download under way, or deletes a kept one.
  async function remove(videoId) {
    const job = jobs.get(videoId);
    if (job?.operation) {
      await job.operation.abort().catch(() => {});
    }
    jobs.delete(videoId);
    if (supported()) {
      const storage = newStorage({});
      try {
        for (const content of await storage.list()) {
          if (content.appMetadata?.videoId === videoId) {
            await storage.remove(content.offlineUri);
          }
        }
      } finally {
        await storage.destroy().catch(() => {});
      }
    }
    notify();
  }

  /// The `offline:` URI of a finished download of `videoId`, or null.
  async function offlineUri(videoId) {
    if (!window.shaka) return null;
    try {
      const content = (await storedContents()).find(
        (stored) => stored.appMetadata.videoId === videoId && !stored.isIncomplete,
      );
      return content ? content.offlineUri : null;
    } catch (_) {
      return null;
    }
  }

  window.TawnyDownloads = {
    start,
    list,
    remove,
    offlineUri,
    pickTracks,
    supported,
  };
})();
