(function () {
  "use strict";

  const runtimes = new WeakMap();
  const resumePositions = new Map();
  const transportEvents = [];
  let nextGeneration = 1;

  function recordTransportEvent(event) {
    const entry = { at: Date.now(), ...event };
    transportEvents.push(entry);
    if (transportEvents.length > 100) transportEvents.shift();
    window.__tawnyTransportDebug = entry;
    return entry;
  }

  // Shaka packs the useful part of a network failure into `data`: the URI it
  // asked for, then the HTTP status. A bare code like 1002 says only "bad HTTP
  // status", which cannot distinguish a gated stream from a host the device
  // cannot reach — the difference that matters once the app runs on a handset
  // and the server is somewhere else.
  function describeShakaError(detail) {
    if (!detail) return null;
    const data = Array.isArray(detail.data) ? detail.data : [];
    const uri = data.find((value) => typeof value === "string" && value.includes("/"));
    const status = data.find((value) => Number.isInteger(value) && value >= 100 && value < 600);
    return {
      code: detail.code,
      category: detail.category,
      httpStatus: status ?? null,
      uri: uri ?? null,
    };
  }

  function playbackPosition(video) {
    const position = Number(video.currentTime);
    return Number.isFinite(position) && position > 0 ? position : 0;
  }

  function rememberedPosition(video, options) {
    const requested = Number(options.startTime);
    if (Number.isFinite(requested) && requested > 0) return requested;
    const current = playbackPosition(video);
    if (current > 0) return current;
    return options.videoId ? resumePositions.get(options.videoId) || 0 : 0;
  }

  function rememberPosition(video, options) {
    if (!options.videoId) return;
    const position = playbackPosition(video);
    // Within the last few seconds the video is over; remembering that spot
    // would reopen it at the end, and straight on to the next video.
    const duration = Number(video.duration);
    if (Number.isFinite(duration) && duration > 0 && position + 15 >= duration) {
      resumePositions.delete(options.videoId);
    } else if (position > 0) {
      resumePositions.set(options.videoId, position);
    }
  }

  function emit(video, name, detail) {
    video.dispatchEvent(new CustomEvent(name, { detail }));
  }

  function sourceLabel(source) {
    return {
      protocol: source.protocol,
      quality: source.quality_label || "Auto",
    };
  }

  function playbackServerBase(options) {
    const configured = String(options?.serverUrl || "").replace(/\/+$/, "");
    const location = window.location;
    const assetHosted =
      location &&
      (location.hostname === "dioxus.index.html" ||
        !["http:", "https:"].includes(location.protocol));
    if (assetHosted && configured) return configured;
    if (location && ["http:", "https:"].includes(location.protocol)) {
      return location.origin;
    }
    return configured;
  }

  function normalizePlaybackUrl(value, options) {
    if (!value) return value;
    const base = playbackServerBase(options);
    if (!base) return value;
    try {
      const parsed = new URL(value, base);
      if (!parsed.pathname.startsWith("/api/v1/playback/proxy/")) return value;
      return new URL(`${parsed.pathname}${parsed.search}`, `${base}/`).href;
    } catch (_) {
      return value;
    }
  }

  function normalizePlaybackSource(source, options) {
    if (!source) return source;
    return {
      ...source,
      url: normalizePlaybackUrl(source.url, options),
      tracks: (source.tracks || []).map((track) => ({
        ...track,
        url: normalizePlaybackUrl(track.url, options),
      })),
    };
  }

  function normalizePlaybackSession(session, options) {
    return {
      ...session,
      primary: normalizePlaybackSource(session.primary, options),
      alternatives: (session.alternatives || []).map((source) =>
        normalizePlaybackSource(source, options),
      ),
    };
  }

  function splitMime(value) {
    const mime = value || "application/octet-stream";
    const match = mime.match(/^\s*([^;]+)(?:;\s*codecs=["']?([^"']+)["']?)?/i);
    return {
      mime: match ? match[1].trim() : mime,
      codecs: match && match[2] ? match[2].trim() : "",
    };
  }

  function xml(value) {
    return String(value)
      .replaceAll("&", "&amp;")
      .replaceAll('"', "&quot;")
      .replaceAll("<", "&lt;")
      .replaceAll(">", "&gt;");
  }

  function attribute(name, value) {
    return value === null || value === undefined || value === "" ? "" : ` ${name}="${xml(value)}"`;
  }

  function representation(track, id) {
    const type = splitMime(track.mime_type);
    const init = track.init_range;
    const index = track.index_range;
    if (!init || !index) return "";
    return [
      `<Representation id="${id}" bandwidth="${track.bitrate || 1}"`,
      attribute("mimeType", type.mime),
      attribute("codecs", type.codecs),
      attribute("width", track.width),
      attribute("height", track.height),
      attribute("frameRate", track.fps),
      ">",
      `<BaseURL>${xml(track.url)}</BaseURL>`,
      `<SegmentBase indexRange="${index.start}-${index.end}" indexRangeExact="true">`,
      `<Initialization range="${init.start}-${init.end}"/>`,
      "</SegmentBase>",
      "</Representation>",
    ].join("");
  }

  function codecFamily(track) {
    const type = splitMime(track.mime_type);
    return (type.codecs.split(",", 1)[0] || "unknown").trim().toLowerCase().split(".", 1)[0];
  }

  function adaptationKey(track) {
    const type = splitMime(track.mime_type);
    return `${type.mime.toLowerCase()}|${codecFamily(track)}`;
  }

  function audioAdaptationKey(track) {
    const type = splitMime(track.mime_type);
    // AAC-LC (mp4a.40.2) and HE-AAC (mp4a.40.5) share a family name but not a
    // decoder configuration. Advertising them as interchangeable lets ABR
    // bounce between the two, rebuilding Android's audio decoder and producing
    // the exact pops/glitches heard at segment boundaries.
    return `${type.mime.toLowerCase()}|${type.codecs.toLowerCase()}`;
  }

  function groupTracks(tracks, keyForTrack) {
    const groups = new Map();
    for (const track of tracks) {
      const key = keyForTrack(track);
      if (!groups.has(key)) groups.set(key, []);
      groups.get(key).push(track);
    }
    return Array.from(groups.values());
  }

  function compatibleAudioTracks(tracks) {
    const identity = (track) => `${track.language || "und"}|${track.label || ""}`;
    const aacLc = new Set(
      tracks
        .filter((track) => splitMime(track.mime_type).codecs.toLowerCase() === "mp4a.40.2")
        .map(identity),
    );
    // Do not create Shaka variants for HE-AAC when the same language has an
    // AAC-LC stream. Shaka may move between AdaptationSets while recovering a
    // request, so grouping alone cannot prevent Android from alternating the
    // incompatible decoder profiles.
    return tracks.filter((track) => {
      const codec = splitMime(track.mime_type).codecs.toLowerCase();
      return codec !== "mp4a.40.5" || !aacLc.has(identity(track));
    });
  }

  // `audioOnly` drops the video AdaptationSets entirely rather than just hiding
  // the picture, so an audio-only session actually stops fetching video segments.
  function generatedDash(source, audioOnly) {
    const tracks = source.tracks || [];
    const videos = audioOnly ? [] : tracks.filter((track) => track.kind === "Video");
    const audios = compatibleAudioTracks(tracks.filter((track) => track.kind === "Audio"));
    if (!audios.length || (!audioOnly && !videos.length)) {
      throw new Error(
        audioOnly
          ? "Audio-only playback needs an audio track"
          : "Adaptive playback needs both video and audio tracks",
      );
    }

    const durationMs = tracks.reduce(
      (duration, track) => Math.max(duration, track.duration_ms || 0),
      0,
    );
    const duration = Math.max(0.001, durationMs / 1000);
    // A DASH AdaptationSet promises that all of its representations can be
    // switched seamlessly. YouTube returns AVC, AV1, and VP9 tracks together,
    // often in both MP4 and WebM containers. Mixing those into one set makes
    // the ABR manager rebuild SourceBuffers whenever it reevaluates quality.
    const videoXml = groupTracks(videos, adaptationKey)
      .map((group, groupIndex) => {
        const reps = group
          .map((track, index) => representation(track, `v${groupIndex}-${index}`))
          .join("");
        return [
          `<AdaptationSet id="v${groupIndex}" contentType="video" segmentAlignment="true">`,
          reps,
          "</AdaptationSet>",
        ].join("");
      })
      .join("");

    const audioGroups = groupTracks(audios, (track) => {
      const identity = `${track.language || "und"}|${track.label || ""}`;
      return `${identity}|${audioAdaptationKey(track)}`;
    });
    const audioXml = audioGroups
      .map((group, groupIndex) => {
        const language = group[0].language || "und";
        const isDefault = group.some((track) => track.is_default);
        const reps = group
          .map((track, index) => representation(track, `a${groupIndex}-${index}`))
          .join("");
        return [
          `<AdaptationSet id="a${groupIndex}" contentType="audio" segmentAlignment="true"`,
          attribute("lang", language === "und" ? null : language),
          ">",
          isDefault ? '<Role schemeIdUri="urn:mpeg:dash:role:2011" value="main"/>' : "",
          reps,
          "</AdaptationSet>",
        ].join("");
      })
      .join("");

    return [
      '<?xml version="1.0" encoding="UTF-8"?>',
      '<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static"',
      ' profiles="urn:mpeg:dash:profile:isoff-on-demand:2011,urn:webm:dash:profile:webm-on-demand:2012"',
      ` minBufferTime="PT1.5S" mediaPresentationDuration="PT${duration}S">`,
      '<Period id="0" start="PT0S">',
      videoXml,
      audioXml,
      "</Period></MPD>",
    ].join("");
  }

  function transportKind(source) {
    const mime = (source.mime_type || "").toLowerCase();
    if ((source.tracks || []).length) return "generated-dash";
    if (source.protocol === "Dash" || mime.includes("dash+xml")) return "dash";
    if (source.protocol === "Hls" || mime.includes("mpegurl")) return "hls";
    if (source.protocol === "Progressive") return "progressive";
    if (source.protocol === "Sabr" && window.TawnySabrAdapter) return "sabr";
    return "unsupported";
  }

  async function destroyRuntime(video) {
    const runtime = runtimes.get(video);
    if (!runtime) return;
    rememberPosition(video, runtime.options || {});
    runtimes.delete(video);
    runtime.cancelled = true;
    if (runtime.errorHandler) {
      video.removeEventListener("error", runtime.errorHandler, true);
    }
    if (runtime.positionHandler) {
      video.removeEventListener("timeupdate", runtime.positionHandler);
    }
    if (runtime.endedHandler) {
      video.removeEventListener("ended", runtime.endedHandler);
    }
    if (runtime.player) {
      try {
        await runtime.player.destroy();
      } catch (_) {}
    }
    if (runtime.objectUrl) URL.revokeObjectURL(runtime.objectUrl);
    if (!runtime.player) {
      video.pause();
      video.removeAttribute("src");
      video.load();
    }
  }

  function configureShaka(player, preferredAudioLanguage) {
    const retry = {
      maxAttempts: 4,
      baseDelay: 500,
      backoffFactor: 2,
      fuzzFactor: 0.35,
      // No cap on the whole request. A 4K segment is ~8 MB, so a 15 s cap
      // aborted any download under ~4.3 Mbit/s even while bytes were still
      // arriving, then retried the same segment from zero. Wedged requests are
      // caught by the stall and connection timeouts instead.
      timeout: 0,
      stallTimeout: 5000,
      connectionTimeout: 8000,
    };
    const applePlatform = /Macintosh|iPhone|iPad|iPod/i.test(
      (window.navigator && window.navigator.userAgent) || "",
    );
    const androidPlatform = /Android/i.test((window.navigator && window.navigator.userAgent) || "");
    player.configure({
      manifest: { retryParameters: retry },
      // Shaka filters the manifest to one codec family before ABR starts.
      // Quality may still change, but the underlying decoder/container does not.
      preferredVideoCodecs: applePlatform
        ? ["avc1", "hvc1", "hev1", "vp09", "av01"]
        : androidPlatform
          ? ["avc1", "vp09", "av01", "hvc1", "hev1"]
          : ["vp09", "av01", "avc1", "hvc1", "hev1"],
      // AVC/AAC is the most consistently hardware-decoded pair across
      // Android System WebView versions, so prefer it there before the more
      // device-dependent WebM and AV1 representations.
      preferredAudioCodecs: applePlatform || androidPlatform ? ["mp4a", "opus"] : ["opus", "mp4a"],
      // Dubbed uploads ship a dozen or more audio languages and the manifest
      // order is not a preference - without this Shaka simply takes the first
      // one, which is how an English video ended up playing in Italian. Shaka
      // falls back to the first track when the viewer speaks none of them, so
      // this can only improve the guess. The chip stays the way to override it.
      preferredAudioLanguage: preferredAudioLanguage || "en",
      streaming: {
        retryParameters: retry,
        bufferingGoal: 40,
        rebufferingGoal: 2,
        bufferBehind: 30,
        safeSeekOffset: 1,
        stallEnabled: true,
        stallThreshold: 2,
        // Let Chromium's media pipeline hold its audio clock during a stall.
        // Shaka's usual 100 ms seek-forward recovery creates an audible pop
        // each time a constrained Android decoder briefly falls behind.
        stallSkip: 0,
      },
      abr: {
        enabled: true,
        defaultBandwidthEstimate: 2_500_000,
        switchInterval: 4,
      },
    });
  }

  function waitForMetadata(video) {
    if (video.readyState >= HTMLMediaElement.HAVE_METADATA) return Promise.resolve();
    return new Promise((resolve, reject) => {
      const done = () => {
        cleanup();
        resolve();
      };
      const failed = () => {
        cleanup();
        reject(new Error("The media element rejected the stream"));
      };
      const cleanup = () => {
        video.removeEventListener("loadedmetadata", done);
        video.removeEventListener("error", failed);
      };
      video.addEventListener("loadedmetadata", done, { once: true });
      video.addEventListener("error", failed, { once: true });
    });
  }

  async function loadNative(video, source, runtime, startTime) {
    const mime = source.mime_type || "";
    if (source.protocol === "Hls" && !video.canPlayType(mime)) {
      throw new Error("Native HLS is unavailable");
    }
    video.src = source.url;
    await waitForMetadata(video);
    if (runtime.cancelled) throw new Error("Playback changed");
    if (startTime > 0 && Number.isFinite(video.duration)) {
      video.currentTime = Math.min(startTime, Math.max(0, video.duration - 0.25));
    }
  }

  async function loadShaka(video, source, runtime, kind, startTime, options) {
    if (!window.shaka || !window.shaka.Player.isBrowserSupported()) {
      throw new Error("Media Source playback is unavailable");
    }
    const player = new window.shaka.Player();
    runtime.player = player;
    const audioOnly = Boolean(options?.audioOnly);
    const preferredAudioLanguage = String(options?.preferredAudioLanguage || "en").trim() || "en";
    configureShaka(player, preferredAudioLanguage);
    const pendingRanges = new Map();
    const networking = player.getNetworkingEngine();
    networking.registerRequestFilter((_requestType, request) => {
      const range = request.headers?.Range || request.headers?.range;
      const uri = request.uris?.[0];
      if (!range || !uri) return;
      const queue = pendingRanges.get(uri) || [];
      queue.push(range);
      pendingRanges.set(uri, queue);
    });
    networking.registerResponseFilter((_requestType, response) => {
      const uri = response.originalUri || response.uri;
      const queue = uri && pendingRanges.get(uri);
      const contentRange = response.headers?.["content-range"] || null;
      const served = /^bytes (\d+)-(\d+)\//.exec(contentRange || "");
      const servedRange = served ? `bytes=${served[1]}-${served[2]}` : null;
      // Initialization, index, and media requests for one representation share
      // a URI and run concurrently. Pair by the server's Content-Range rather
      // than completion order, which is intentionally nondeterministic.
      const matchingIndex = servedRange && queue ? queue.indexOf(servedRange) : -1;
      const range = matchingIndex >= 0 ? queue.splice(matchingIndex, 1)[0] : queue?.shift();
      if (!range) return;
      if (!queue.length) pendingRanges.delete(uri);
      const match = /^bytes=(\d+)-(\d+)$/.exec(range);
      const received = response.data?.byteLength;
      if (!match || !Number.isFinite(received)) return;
      const start = Number(match[1]);
      const end = Number(match[2]);
      const expected = end - start + 1;
      if (received !== expected) {
        let path = uri;
        try {
          path = new URL(uri).pathname;
        } catch (_) {}
        console.warn(
          "Tawny range mismatch",
          JSON.stringify({
            path,
            start,
            end,
            expected,
            received,
            status: response.status ?? null,
            contentRange,
            contentLength: response.headers?.["content-length"] || null,
          }),
        );
      }
    });
    await player.attach(video);

    let uri = source.url;
    let mime = source.mime_type || undefined;
    if (kind === "generated-dash") {
      const manifest = generatedDash(source, Boolean(audioOnly));
      runtime.objectUrl = URL.createObjectURL(
        new Blob([manifest], { type: "application/dash+xml" }),
      );
      uri = runtime.objectUrl;
      mime = "application/dash+xml";
    }
    await player.load(uri, startTime > 0 ? startTime : null, mime);
    if (runtime.cancelled) throw new Error("Playback changed");
    player.addEventListener("variantchanged", () => {
      if (!runtime.cancelled) emit(video, "tawnyqualitychange", qualityState(video));
    });
    player.addEventListener("error", (event) => {
      const severity = event.detail && event.detail.severity;
      const critical = window.shaka.util.Error.Severity.CRITICAL;
      if (!runtime.cancelled && severity === critical) {
        runtime.lastError = {
          category: event.detail.category,
          code: event.detail.code,
          data: event.detail.data,
          severity,
        };
        const described = describeShakaError(event.detail);
        recordTransportEvent({
          phase: "shaka-error",
          ...described,
          position: playbackPosition(video),
        });
        const networkCategory = window.shaka.util.Error.Category.NETWORK;
        const isNetworkError = event.detail.category === networkCategory;
        const at = playbackPosition(video);
        if (isNetworkError && runtime.inPlaceRetries < 3) {
          runtime.inPlaceRetries += 1;
          const retrying = player.retryStreaming(0.25);
          recordTransportEvent({
            phase: "retrying-stream",
            position: at,
            attempt: runtime.inPlaceRetries,
            retrying,
            error: runtime.lastError,
          });
          if (retrying) return;
        }
        video.dispatchEvent(new Event("error"));
      }
    });
  }

  async function loadOne(video, source, runtime, options) {
    const kind = transportKind(source);
    const startTime = rememberedPosition(video, options);
    if (kind === "progressive") return loadNative(video, source, runtime, startTime);
    if (kind === "hls" && (!window.shaka || !window.shaka.Player.isBrowserSupported())) {
      return loadNative(video, source, runtime, startTime);
    }
    if (kind === "dash" || kind === "hls" || kind === "generated-dash") {
      return loadShaka(video, source, runtime, kind, startTime, options);
    }
    if (kind === "sabr") {
      return window.TawnySabrAdapter.load(video, source, runtime, {
        ...options,
        startTime,
      });
    }
    throw new Error(`Unsupported playback protocol: ${source.protocol}`);
  }

  function canRefresh(options) {
    return options.refreshUrl && (options.refreshAttempts || 0) < 1;
  }

  async function refreshPlaybackSession(video, options, generation, position) {
    recordTransportEvent({
      phase: "refreshing-session",
      position,
    });
    const response = await fetch(options.refreshUrl, { cache: "no-store" });
    if (!response.ok) {
      throw new Error(`Playback refresh failed with HTTP ${response.status}`);
    }
    const refreshedSession = await response.json();
    return loadSources(
      video,
      refreshedSession,
      {
        ...options,
        startTime: position,
        refreshAttempts: (options.refreshAttempts || 0) + 1,
      },
      generation,
      0,
    );
  }

  async function loadSources(video, session, options, generation, startIndex) {
    session = normalizePlaybackSession(session, options);
    const sources = [session.primary, ...(session.alternatives || [])].filter(
      (source) => source && source.protocol !== "EmbedFallback",
    );
    let lastError = new Error("No in-app playback source was available");
    const resumeTime = rememberedPosition(video, options);
    const recoveryOptions = { ...options, startTime: resumeTime };

    for (let index = startIndex; index < sources.length; index += 1) {
      if (video.__tawnyGeneration !== generation) throw new Error("Playback changed");
      await destroyRuntime(video);
      const source = sources[index];
      recordTransportEvent({
        phase: "loading",
        index,
        protocol: source.protocol,
        quality: source.quality_label || null,
      });
      const runtime = {
        cancelled: false,
        player: null,
        objectUrl: null,
        errorHandler: null,
        positionHandler: null,
        endedHandler: null,
        lastError: null,
        inPlaceRetries: 0,
        lastProgressPosition: resumeTime,
        options: recoveryOptions,
      };
      runtimes.set(video, runtime);
      runtime.errorHandler = (event) => event.stopImmediatePropagation();
      video.addEventListener("error", runtime.errorHandler, {
        once: true,
        capture: true,
      });
      try {
        await loadOne(video, source, runtime, recoveryOptions);
        video.removeEventListener("error", runtime.errorHandler, true);
        video.playbackRate = options.playbackRate || 1;
        video.defaultPlaybackRate = options.playbackRate || 1;
        emit(video, "tawnytransportchange", sourceLabel(source));
        recordTransportEvent({
          phase: "playing",
          index,
          protocol: source.protocol,
          quality: source.quality_label || null,
        });
        runtime.positionHandler = () => {
          rememberPosition(video, recoveryOptions);
          const position = playbackPosition(video);
          if (position >= runtime.lastProgressPosition + 8) {
            runtime.lastProgressPosition = position;
            runtime.inPlaceRetries = 0;
          }
        };
        runtime.endedHandler = () => {
          if (recoveryOptions.videoId) resumePositions.delete(recoveryOptions.videoId);
        };
        video.addEventListener("timeupdate", runtime.positionHandler);
        video.addEventListener("ended", runtime.endedHandler, { once: true });
        video.play().catch((error) => {
          // Refused rather than interrupted: the controls stop waiting for a
          // start that is not coming and offer Play instead.
          if (error?.name === "NotAllowedError") emit(video, "tawnyautoplayblocked");
        });

        runtime.errorHandler = (event) => {
          event.stopImmediatePropagation();
          if (runtime.cancelled || video.__tawnyGeneration !== generation) return;
          const position = Math.max(
            playbackPosition(video),
            rememberedPosition(video, recoveryOptions),
          );
          rememberPosition(video, recoveryOptions);
          recordTransportEvent({
            phase: "recovering",
            index,
            protocol: source.protocol,
            position,
            error: runtime.lastError,
          });
          const nextOptions = { ...recoveryOptions, startTime: position };
          const recovery = canRefresh(nextOptions)
            ? refreshPlaybackSession(video, nextOptions, generation, position)
            : loadSources(video, session, nextOptions, generation, index + 1);
          recovery.catch((error) => {
            emit(video, "tawnytransporterror", { message: String(error) });
            video.dispatchEvent(new CustomEvent("tawnytransportexhausted"));
            video.dispatchEvent(new Event("error"));
          });
        };
        video.addEventListener("error", runtime.errorHandler, {
          once: true,
          capture: true,
        });
        return sourceLabel(source);
      } catch (error) {
        lastError = error;
        recordTransportEvent({
          phase: "source-failed",
          index,
          protocol: source.protocol,
          error: String(error),
        });
        await destroyRuntime(video);
      }
    }
    if (canRefresh(options) && video.__tawnyGeneration === generation) {
      return refreshPlaybackSession(video, options, generation, resumeTime);
    }
    throw lastError;
  }

  // A video that starts while the page is hidden - the screen locked while it
  // loaded, or autoplay moving on in the background - is paused by Chrome on
  // Android, which only lets a video play hidden if it was already playing
  // when the page went away. Resuming it gets paused again, and what comes out
  // is audio stuttering until the screen is back on. Audio alone is always
  // allowed to play hidden, so that is what starts, and the picture is loaded
  // back in at the same place once there is a screen to show it on.
  function startsHidden(options) {
    return document.visibilityState === "hidden" && !options.audioOnly;
  }

  function cancelRestoreWhenVisible(video) {
    video.__tawnyCancelRestore?.();
    video.__tawnyCancelRestore = null;
  }

  function restoreVideoWhenVisible(video, session, options, generation) {
    // A listener that only removes itself once the page is visible would hold
    // this video and its session for as long as the page stayed hidden, and
    // forever if the generation moved on first. Whoever replaces or detaches
    // this generation takes it down instead.
    cancelRestoreWhenVisible(video);
    const onVisible = () => {
      if (document.visibilityState !== "visible") return;
      cancelRestoreWhenVisible(video);
      if (video.__tawnyGeneration !== generation || !video.isConnected) return;
      const resume = !video.paused;
      recordTransportEvent({ phase: "restoring-video", position: playbackPosition(video) });
      loadSources(
        video,
        session,
        { ...options, audioOnly: false, startTime: playbackPosition(video) },
        generation,
        0,
      )
        .then(() => {
          if (!resume) video.pause();
        })
        .catch((error) => {
          emit(video, "tawnytransporterror", { message: String(error) });
          video.dispatchEvent(new Event("error"));
        });
    };
    document.addEventListener("visibilitychange", onVisible);
    video.__tawnyCancelRestore = () => document.removeEventListener("visibilitychange", onVisible);
  }

  async function attach(video, session, options) {
    if (!video) throw new Error("Player element is missing");
    const generation = nextGeneration++;
    video.__tawnyGeneration = generation;
    cancelRestoreWhenVisible(video);
    const attachOptions = options || {};
    attachOptions.startTime = rememberedPosition(video, attachOptions);
    recordTransportEvent({
      phase: "attaching",
      generation,
      position: attachOptions.startTime,
    });
    if (startsHidden(attachOptions)) {
      restoreVideoWhenVisible(video, session, attachOptions, generation);
      return loadSources(video, session, { ...attachOptions, audioOnly: true }, generation, 0);
    }
    return loadSources(video, session, attachOptions, generation, 0);
  }

  async function detach(video) {
    if (!video) return;
    video.__tawnyGeneration = nextGeneration++;
    cancelRestoreWhenVisible(video);
    await destroyRuntime(video);
  }

  function qualityState(video) {
    const runtime = runtimes.get(video);
    if (!runtime?.player || typeof runtime.player.getVariantTracks !== "function") {
      return null;
    }
    const variants = runtime.player
      .getVariantTracks()
      .filter(
        (track) =>
          track.height &&
          track.allowedByApplication !== false &&
          track.allowedByKeySystem !== false,
      );
    const active = variants.find((track) => track.active);
    const resolution = (track) =>
      track.width && track.height ? Math.min(track.width, track.height) : track.height;
    const heights = Array.from(new Set(variants.map(resolution))).sort(
      (left, right) => right - left,
    );
    return {
      auto: runtime.manualQuality == null,
      selectedHeight: runtime.manualQuality,
      activeHeight: active ? resolution(active) : null,
      heights,
    };
  }

  // Some uploads carry dubbed audio in several languages, and YouTube picks
  // one for you - not always the original, and not always one you speak. Shaka
  // already models these as separate audio languages on the variants; this just
  // reads them out and lets the player pick.
  function audioTrackState(video) {
    const runtime = runtimes.get(video);
    if (!runtime?.player || typeof runtime.player.getVariantTracks !== "function") {
      return null;
    }
    const variants = runtime.player
      .getVariantTracks()
      .filter(
        (track) => track.allowedByApplication !== false && track.allowedByKeySystem !== false,
      );
    const active = variants.find((track) => track.active);
    const seen = new Map();
    for (const track of variants) {
      const language = track.language || "und";
      if (seen.has(language)) continue;
      seen.set(language, {
        language,
        label: track.label || languageLabel(language),
        roles: track.audioRoles || [],
      });
    }
    return {
      active: active ? active.language || "und" : null,
      tracks: Array.from(seen.values()),
    };
  }

  // `Intl.DisplayNames` knows the endonyms; the raw tag is a poor label but a
  // truthful fallback when it does not recognise one.
  const languageNames = (() => {
    try {
      return new Intl.DisplayNames(undefined, { type: "language" });
    } catch (_) {
      return null;
    }
  })();

  function languageLabel(language) {
    if (!language || language === "und") return "Default";
    try {
      return languageNames?.of(language) || language;
    } catch (_) {
      return language;
    }
  }

  function setAudioTrack(video, language) {
    const runtime = runtimes.get(video);
    if (!runtime?.player || typeof runtime.player.selectAudioLanguage !== "function") {
      return false;
    }
    runtime.player.selectAudioLanguage(language);
    emit(video, "tawnyaudiotrackchange", audioTrackState(video));
    // Selecting a language rebuilds the variant list, so the quality chip has
    // to be told as well or it keeps advertising the old one.
    emit(video, "tawnyqualitychange", qualityState(video));
    return true;
  }

  function setQuality(video, height) {
    const runtime = runtimes.get(video);
    if (!runtime?.player) return false;
    const requested = Number(height);
    if (height === null || !Number.isFinite(requested)) {
      runtime.manualQuality = null;
      runtime.player.configure({ abr: { enabled: true } });
      emit(video, "tawnyqualitychange", qualityState(video));
      return true;
    }
    const resolution = (track) =>
      track.width && track.height ? Math.min(track.width, track.height) : track.height;
    const variants = runtime.player
      .getVariantTracks()
      .filter(
        (track) =>
          resolution(track) === requested &&
          track.allowedByApplication !== false &&
          track.allowedByKeySystem !== false,
      );
    if (!variants.length) return false;
    const active = runtime.player.getVariantTracks().find((track) => track.active);
    const sameLanguage = active
      ? variants.filter((track) => track.language === active.language)
      : variants;
    const candidates = sameLanguage.length ? sameLanguage : variants;
    const selected = candidates.sort((left, right) => right.bandwidth - left.bandwidth)[0];
    runtime.manualQuality = requested;
    runtime.player.configure({ abr: { enabled: false } });
    runtime.player.selectVariantTrack(selected, true, 2);
    emit(video, "tawnyqualitychange", qualityState(video));
    return true;
  }

  if (window.shaka) window.shaka.polyfill.installAll();
  window.TawnyTransport = {
    attach,
    detach,
    rememberedPosition(videoId) {
      return resumePositions.get(videoId) || 0;
    },
    buildDashManifest: generatedDash,
    events: transportEvents,
    normalizePlaybackSession,
    normalizePlaybackUrl,
    playbackServerBase,
    qualityState,
    audioTrackState,
    setAudioTrack,
    setQuality,
    transportKind,
  };
})();
