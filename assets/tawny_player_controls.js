(function () {
  "use strict";

  const controllers = new WeakMap();
  /// The last metadata given for a media element, applied again on re-attach.
  const lastMetadata = new WeakMap();
  const speedSteps = [0.5, 0.75, 1, 1.25, 1.5, 1.75, 2];
  const quickSpeedSteps = [1, 1.25, 1.5, 2];

  function formatTime(value) {
    if (!Number.isFinite(value) || value < 0) return "0:00";
    const seconds = Math.floor(value % 60);
    const minutes = Math.floor(value / 60) % 60;
    const hours = Math.floor(value / 3600);
    const tail = `${minutes}:${String(seconds).padStart(2, "0")}`;
    return hours > 0
      ? `${hours}:${String(minutes).padStart(2, "0")}:${String(seconds).padStart(2, "0")}`
      : tail;
  }

  function fullscreenElement() {
    return document.fullscreenElement || document.webkitFullscreenElement || null;
  }

  function toggleFullscreen(root) {
    if (fullscreenElement()) {
      const exit = document.exitFullscreen || document.webkitExitFullscreen;
      if (screen.orientation && typeof screen.orientation.unlock === "function") {
        try {
          screen.orientation.unlock();
        } catch (_) {}
      }
      root.querySelector("[data-player-native-orientation-unlock]")?.click();
      return exit ? exit.call(document) : undefined;
    }
    return enterFullscreenFor(root);
  }

  /// Go fullscreen. Rotating to landscape is not decided here but by the
  /// controller once the player *is* fullscreen - see `applyFullscreenOrientation`.
  ///
  /// iOS Safari never implemented fullscreen on ordinary elements, so the video
  /// element's own presentation mode is the fallback there. Without it the
  /// button did nothing at all on a handset.
  function enterFullscreenFor(root) {
    const media = root.querySelector("video");
    const request =
      root.requestFullscreen || root.webkitRequestFullscreen || root.webkitRequestFullScreen;
    const started = request
      ? Promise.resolve(request.call(root, { navigationUI: "hide" })).catch(() =>
          Promise.resolve(request.call(root)).catch(() => Promise.reject()),
        )
      : Promise.reject();
    return started.catch(() => {
      if (media && typeof media.webkitEnterFullscreen === "function") {
        try {
          media.webkitEnterFullscreen();
        } catch (_) {}
      }
    });
  }

  // The element whose controller holds the page's Media Session actions.
  let sessionOwner = null;

  /// Deliberate, mostly vertical, and quick enough to be a flick. Positive
  /// `dy` is downward.
  function isVerticalFlick(dx, dy, elapsedMs) {
    return elapsedMs <= 800 && Math.abs(dy) >= 55 && Math.abs(dy) > Math.abs(dx);
  }

  // The player sits at the very top of the screen, which is also where a
  // pulled-down notification shade starts. A drag that begins in this strip is
  // someone reaching for the status bar, not asking to minimize the video.
  const TOP_DEAD_ZONE_PX = 44;
  const inTopDeadZone = (clientY) => clientY < TOP_DEAD_ZONE_PX;

  // While the watch sheet slides in, the browser hit-tests the transition's
  // snapshots instead of the page, so every touch lands on the document and
  // the player never hears it. That is the half second or more after tapping
  // a video, with the spinner up: just when someone changes their mind. The
  // page has only just opened at its top, so a downward flick then can only
  // mean "put it away", and it does what the stage's own swipe does. The
  // stylesheet stops the browser cancelling the touch to pan the document.
  function presentingWatchSheet() {
    const root = document.documentElement;
    return (
      root.dataset.routeTransition === "present-sheet" &&
      (root.dataset.routeTransitionTo || "").startsWith("/watch/")
    );
  }

  // The expanded stage while no controller owns it: the spinner while the
  // stream resolves, the gap between the video element mounting and the
  // controller attaching, and the failure panel after the element is gone. The
  // controller handles its own swipes once attached, so this stays out of its
  // way. Done here, in the page, rather than in a Rust handler: on a native
  // webview every Rust pointer handler is a blocking round trip, and the drag
  // never reached it reliably.
  function unattachedStage(target) {
    if (!(target instanceof Element)) return false;
    const stage = target.closest("#tawny-player");
    if (!stage || !stage.closest(".persistent-player.expanded")) return false;
    if (target.closest("button, a, input, select, .player-options-menu")) return false;
    const media = document.getElementById("tawny-player-media");
    return !(media && controllers.has(media));
  }

  let sheetFlick = null;
  document.addEventListener(
    "pointerdown",
    (event) => {
      sheetFlick =
        ((event.target === document.documentElement && presentingWatchSheet()) ||
          unattachedStage(event.target)) &&
        !inTopDeadZone(event.clientY)
          ? { pointerId: event.pointerId, x: event.clientX, y: event.clientY, at: Date.now() }
          : null;
    },
    true,
  );
  document.addEventListener(
    "pointerup",
    (event) => {
      const flick = sheetFlick;
      sheetFlick = null;
      if (!flick || flick.pointerId !== event.pointerId) return;
      const dy = event.clientY - flick.y;
      if (!isVerticalFlick(event.clientX - flick.x, dy, Date.now() - flick.at)) return;
      const stage = document.getElementById("tawny-player");
      if (!stage) return;
      // Up is fullscreen, the same as on an attached player. The controller
      // applies the window's bars and rotation when it attaches, because the
      // bridges that do it only exist once a stream has resolved.
      if (dy < 0) {
        if (!fullscreenElement()) enterFullscreenFor(stage);
        return;
      }
      if (fullscreenElement()) {
        const exit = document.exitFullscreen || document.webkitExitFullscreen;
        if (exit) exit.call(document);
        return;
      }
      // Whichever way out the stage has right now: the control bar's once a
      // stream is attached, the stage's Back while it is still resolving.
      stage.querySelector("[data-player-minimize], .player-stage-back")?.click();
    },
    true,
  );
  document.addEventListener("pointercancel", () => (sheetFlick = null), true);

  // A downward flick on the mini bar closes it, the same as its X. Done in the
  // page for the reason above, by clicking the X so there is one way to stop
  // playback.
  let miniFlick = null;
  document.addEventListener(
    "pointerdown",
    (event) => {
      const mini =
        event.target instanceof Element && event.target.closest(".persistent-player.mini");
      miniFlick = mini
        ? { pointerId: event.pointerId, x: event.clientX, y: event.clientY, at: Date.now(), mini }
        : null;
    },
    true,
  );
  document.addEventListener(
    "pointerup",
    (event) => {
      const flick = miniFlick;
      miniFlick = null;
      if (!flick || flick.pointerId !== event.pointerId) return;
      const dy = event.clientY - flick.y;
      if (dy <= 0 || !isVerticalFlick(event.clientX - flick.x, dy, Date.now() - flick.at)) return;
      flick.mini.querySelector(".mini-player-close")?.click();
    },
    true,
  );
  document.addEventListener("pointercancel", () => (miniFlick = null), true);

  function attach(video) {
    if (!video) return;
    const root = video.closest("#tawny-player");
    const controls = root && root.querySelector(".tawny-player-controls");
    if (!root || !controls) return;

    const existing = controllers.get(video);
    if (existing) existing.destroy();

    const abort = new AbortController();
    const signal = abort.signal;
    const progress = controls.querySelector("[data-player-progress]");
    const sponsorNotice = controls.querySelector("[data-player-sponsor-notice]");
    // On the stage rather than in the control bar - see the markup.
    const sponsorSkip = root.querySelector("[data-player-sponsor-skip]");
    let sponsorNoticeTimer = null;
    let sponsorSkipOffer = null;
    const chapterLabel = controls.querySelector("[data-player-chapter-label]");
    const seekPreview = controls.querySelector("[data-player-seek-preview]");
    const previewImage = controls.querySelector("[data-player-preview-image]");
    const previewChapter = controls.querySelector("[data-player-preview-chapter]");
    const previewTime = controls.querySelector("[data-player-preview-time]");
    const currentTime = controls.querySelector("[data-player-current-time]");
    const duration = controls.querySelector("[data-player-duration]");
    const speedLabel = controls.querySelector("[data-player-speed-label]");
    const optionsMenu = controls.querySelector("[data-player-options-menu]");
    const qualityOptions = controls.querySelector("[data-player-quality-options]");
    const playButton = controls.querySelector('[data-player-action="toggle"]');
    const muteButton = controls.querySelector('[data-player-action="mute"]');
    const captionsButton = controls.querySelector('[data-player-action="captions"]');
    const fullscreenButton = controls.querySelector('[data-player-action="fullscreen"]');
    const buffering = root.querySelector(".player-buffering-indicator");
    const feedbackLeft = root.querySelector(".player-seek-feedback-left");
    const feedbackRight = root.querySelector(".player-seek-feedback-right");
    let hideTimer = null;
    let feedbackTimer = null;
    let scrubbing = false;
    let timelineHovering = false;
    let resumeAfterScrub = false;
    let animationFrame = null;
    let chosenHeight = null;
    let captions = [];
    let selectedCaption = null;
    let captionsEnabled = false;
    let chapters = [];
    let sponsorSegments = [];
    let sponsorNotify = true;
    let autoplayEnabled = false;
    // Segments already acted on, by UUID. Without this, seeking back into a
    // skipped sponsor would bounce the playhead straight out again and there
    // would be no way to watch one deliberately.
    const sponsorHandled = new Set();
    let previewFrames = null;
    let playbackIntent = !video.paused && !video.ended;
    // Between attaching and the picture actually moving, the element is
    // paused only because the transport has not called play() yet. Reading
    // that as a pause put the bar up with a Play button over the first frame,
    // and autoplay then contradicted it a second later. Until playback has
    // really started the stage shows the buffering spinner instead.
    let starting = video.autoplay && video.paused && !video.ended;
    let startingFrom = null;
    let playbackRecoveryTimer = null;
    let pipTransition = false;
    let pipTransitionTimer = null;

    const listen = (target, name, handler, options) =>
      target.addEventListener(name, handler, { ...options, signal });

    function setPlaybackIntent(shouldPlay) {
      playbackIntent = Boolean(shouldPlay);
      // The Android PiP actions execute outside this closure. Keeping the
      // intent on the element lets their play/pause command participate in the
      // same lifecycle recovery without mistaking an explicit Pause for a
      // transient Activity pause.
      video.__tawnyPlaybackIntent = playbackIntent;
    }

    function beginPipTransition() {
      pipTransition = true;
      if (pipTransitionTimer) clearTimeout(pipTransitionTimer);
      pipTransitionTimer = setTimeout(() => {
        pipTransition = false;
        pipTransitionTimer = null;
      }, 1400);
    }

    function recoverIntendedPlayback(delay = 0) {
      if (!playbackIntent || video.ended) return;
      if (playbackRecoveryTimer) clearTimeout(playbackRecoveryTimer);
      playbackRecoveryTimer = setTimeout(() => {
        playbackRecoveryTimer = null;
        if (playbackIntent && video.paused && !video.ended) {
          video.play().catch(() => {});
        }
      }, delay);
    }

    // Long enough to read the bar and reach for a control, short enough that the
    // picture is not left with furniture on it.
    const CONTROLS_HIDE_MS = 2000;
    // The pointer has left the player entirely, so there is nothing left to
    // aim at and no reason to wait the full dwell.
    const CONTROLS_LEAVE_HIDE_MS = 450;

    function hideControls() {
      controls.classList.remove("controls-visible");
      root.classList.remove("player-controls-visible");
    }

    function showControls(permanent, delay) {
      controls.classList.add("controls-visible");
      root.classList.add("player-controls-visible");
      if (hideTimer) clearTimeout(hideTimer);
      hideTimer = null;
      // Raised by a tap while starting, the bar still goes away on its own:
      // the pause that would otherwise pin it is not a real one.
      if (permanent || (video.paused && !starting)) return;
      // An open menu holds the controls up - but only while it is open. Closing
      // it has to re-arm this, or the bar stays for the rest of the video.
      if (optionsMenu && !optionsMenu.hidden) return;
      hideTimer = setTimeout(() => {
        if (!scrubbing && !timelineHovering && (optionsMenu?.hidden ?? true)) {
          hideControls();
        }
      }, delay ?? CONTROLS_HIDE_MS);
    }

    function scheduleHide() {
      showControls(false);
    }

    function setStarting(value) {
      starting = value;
      startingFrom = null;
      root.classList.toggle("player-starting", value);
    }

    // Over once the playhead has moved on its own. `playing` alone is too
    // early: Shaka holds the rate at zero until its rebuffering goal is met,
    // and the element reports playing throughout.
    function trackStart() {
      if (!starting || video.paused) return;
      if (startingFrom === null) {
        startingFrom = video.currentTime;
      } else if (video.currentTime - startingFrom > 0.05) {
        // The bar stays down: raising it now would put the controls over a
        // video that has only just started, which is the flash this avoids.
        // A bar a tap raised while starting is already on its hide timer.
        setStarting(false);
      }
    }

    function updatePlaybackState() {
      const playing = !video.paused && !video.ended;
      controls.classList.toggle("is-playing", playing);
      playButton?.setAttribute("aria-label", playing ? "Pause" : "Play");
      if (starting && !video.ended) return;
      if (video.paused || video.ended) showControls(true);
      else scheduleHide();
    }

    function bufferedEnd() {
      let end = 0;
      for (let index = 0; index < video.buffered.length; index += 1) {
        end = Math.max(end, video.buffered.end(index));
      }
      return end;
    }

    function updateChapterLabel(position) {
      if (!chapterLabel) return;
      const chapter = chapterAt(position);
      const title = chapter?.title || "";
      chapterLabel.hidden = !title;
      const chapterTitle = chapterLabel.querySelector("[data-player-chapter-title]");
      if (chapterTitle && chapterTitle.textContent !== title) chapterTitle.textContent = title;
    }

    function updateTimeline() {
      const length = Number.isFinite(video.duration) ? video.duration : 0;
      const position = Number.isFinite(video.currentTime) ? video.currentTime : 0;
      // Compared first: assigning the same text still replaces the node, and
      // this runs every frame the bar is up.
      const positionText = formatTime(position);
      if (currentTime && currentTime.textContent !== positionText) {
        currentTime.textContent = positionText;
      }
      updateChapterLabel(position);
      // The markers are appended imperatively into an element the framework
      // owns, so any re-render of that subtree silently drops them. Rebuilding
      // when they have gone missing is what keeps them on screen.
      applySponsorSegments();
      const lengthText = formatTime(length);
      if (duration && duration.textContent !== lengthText) duration.textContent = lengthText;
      if (!progress || scrubbing) return;
      const played = length > 0 ? Math.min(100, (position / length) * 100) : 0;
      const buffered = length > 0 ? Math.min(100, (bufferedEnd() / length) * 100) : 0;
      progress.value = String(Math.round(played * 100));
      progress.style.setProperty("--player-progress", `${played}%`);
      progress.style.setProperty("--player-buffered", `${Math.max(played, buffered)}%`);
      progress.setAttribute("aria-valuetext", `${formatTime(position)} of ${formatTime(length)}`);
      renderTimeline();
    }

    function effectivePlaybackRate() {
      const rate = Number(video.playbackRate);
      if (Number.isFinite(rate) && rate > 0) return rate;
      const fallback = Number(video.defaultPlaybackRate);
      return Number.isFinite(fallback) && fallback > 0 ? fallback : 1;
    }

    function updateSpeed() {
      const rate = effectivePlaybackRate();
      const value = `${Number(rate.toFixed(2))}×`;
      if (speedLabel) speedLabel.textContent = value;
      controls.querySelectorAll("[data-player-speed]").forEach((button) => {
        button.classList.toggle(
          "selected",
          Math.abs(Number(button.dataset.playerSpeed) - rate) < 0.01,
        );
      });
    }

    function updateMuted() {
      const muted = video.muted || video.volume === 0;
      controls.classList.toggle("is-muted", muted);
      muteButton?.setAttribute("aria-label", muted ? "Unmute" : "Mute");
    }

    function updateCaptions() {
      const tracks = Array.from(video.textTracks || []);
      const selected = tracks.findIndex((track) => track.mode === "showing");
      captionsButton?.classList.toggle("selected", selected >= 0);
      captionsButton?.toggleAttribute("disabled", tracks.length === 0);
      if (captionsButton) {
        captionsButton.title =
          selected >= 0 ? `Captions: ${tracks[selected].label || "On"}` : "Captions off";
      }
    }

    function applyCaptionState() {
      const tracks = Array.from(video.textTracks || []);
      const validSelection =
        Number.isInteger(selectedCaption) &&
        selectedCaption >= 0 &&
        selectedCaption < tracks.length;
      tracks.forEach((track, index) => {
        track.mode =
          captionsEnabled && validSelection && index === selectedCaption ? "showing" : "disabled";
      });
      updateCaptions();
    }

    function chapterAt(position) {
      let active = null;
      for (const chapter of chapters) {
        if (Number(chapter.start_seconds) <= position) active = chapter;
        else break;
      }
      return active;
    }

    /// Width of the gaps punched through the bar, in pixels.
    const TIMELINE_GAP_PX = 3;

    /**
     * Paint the whole timeline as one gradient on the range track.
     *
     * Everything - played, buffered, sponsor colours, and the gaps at chapter
     * and segment boundaries - is a single `linear-gradient`, rather than
     * elements layered over the input. Layering was the obvious approach and it
     * cannot work: the range thumb lives inside the input, so any overlay drawn
     * above the track to colour a segment also covers the thumb.
     *
     * The gaps are `transparent`, not a background-coloured tick, so what shows
     * through is genuinely whatever is behind the bar.
     */
    function renderTimeline() {
      if (!progress) return;
      const length = Number.isFinite(video.duration) ? video.duration : 0;
      if (length <= 0) {
        progress.style.removeProperty("--player-track-image");
        return;
      }

      const width = progress.getBoundingClientRect().width || 0;
      // Below a pixel or two the gaps would be wider than the segments between
      // them, which reads as a dashed line rather than a divided one.
      const gap = width > 0 ? Math.min(1.5, (TIMELINE_GAP_PX / width) * 100) : 0;
      const played = clampPercent((currentPosition() / length) * 100);
      const buffered = Math.max(played, clampPercent((bufferedEnd() / length) * 100));

      // Spans are applied in order, each overriding what came before, so the
      // list reads as layers: base, buffered, played, segment colours, gaps.
      const spans = [
        [0, 100, "rgba(255, 255, 255, 0.25)"],
        [0, buffered, "rgba(255, 255, 255, 0.5)"],
        // A defined token: an unknown custom property makes the whole
        // background invalid at computed-value time, and the browser then
        // paints no track at all rather than falling back - which is why the
        // chapters and segments vanished the moment the playhead left zero,
        // where a played span of no width keeps this stop out of the gradient.
        [0, played, "var(--g3-color-accent)"],
      ];

      for (const segment of sponsorSegments) {
        const start = clampPercent((Number(segment.start_seconds) / length) * 100);
        const end = clampPercent((Number(segment.end_seconds) / length) * 100);
        const color = segment.color || "#00d400";
        // A point segment has no width of its own; give it one so it is
        // visible at all.
        const width_ = Math.max(end - start, gap * 2);
        spans.push([start, Math.min(100, start + width_), color]);
      }

      // Gaps last: they cut through the colours above, including a segment's.
      if (gap > 0) {
        for (const chapter of chapters.slice(1)) {
          const at = clampPercent((Number(chapter.start_seconds) / length) * 100);
          spans.push([at - gap / 2, at + gap / 2, "transparent"]);
        }
        for (const segment of sponsorSegments) {
          const start = clampPercent((Number(segment.start_seconds) / length) * 100);
          const end = clampPercent((Number(segment.end_seconds) / length) * 100);
          spans.push([start - gap / 2, start + gap / 2, "transparent"]);
          spans.push([end - gap / 2, end + gap / 2, "transparent"]);
        }
      }

      progress.style.setProperty("--player-track-image", gradientFrom(spans));
    }

    function clampPercent(value) {
      if (!Number.isFinite(value)) return 0;
      return Math.min(100, Math.max(0, value));
    }

    function currentPosition() {
      return Number.isFinite(video.currentTime) ? video.currentTime : 0;
    }

    /**
     * Flatten overlapping spans into a hard-stop gradient.
     *
     * Every span boundary becomes a breakpoint; each resulting slice takes the
     * colour of the last span covering it. Emitting the spans directly would
     * not work - a gradient has no notion of one stop painting over another.
     */
    function gradientFrom(spans) {
      const edges = new Set([0, 100]);
      for (const [start, end] of spans) {
        edges.add(clampPercent(start));
        edges.add(clampPercent(end));
      }
      const points = [...edges].sort((a, b) => a - b);

      const stops = [];
      for (let index = 0; index < points.length - 1; index++) {
        const from = points[index];
        const to = points[index + 1];
        if (to - from <= 0) continue;
        const middle = (from + to) / 2;
        let color = "transparent";
        for (const [start, end, value] of spans) {
          if (middle >= start && middle < end) color = value;
        }
        stops.push(`${color} ${from.toFixed(3)}% ${to.toFixed(3)}%`);
      }
      return `linear-gradient(to right, ${stops.join(", ")})`;
    }

    /**
     * Act on any segment the playhead has entered.
     *
     * Only `skip` segments move the playhead, and only once each: a viewer who
     * seeks back into one is doing it on purpose.
     */
    /**
     * Offer to skip the segment the playhead is inside, for segments set to
     * "show" rather than "skip".
     *
     * A viewer who asked to see a segment marked on the timeline still wants a
     * way past it; the alternative is scrubbing by eye. The offer is withdrawn
     * as soon as the playhead leaves the segment, and a segment skipped or
     * declined does not ask again.
     */
    function offerSponsorSkip(position) {
      if (!sponsorSkip) return;
      const offer =
        scrubbing || !sponsorSegments.length
          ? null
          : sponsorSegments.find(
              (segment) =>
                segment.action === "show" &&
                !sponsorHandled.has(segment.uuid) &&
                position >= Number(segment.start_seconds) &&
                position < Number(segment.end_seconds),
            ) || null;
      if ((offer?.uuid || null) === (sponsorSkipOffer?.uuid || null)) return;
      sponsorSkipOffer = offer;
      sponsorSkip.hidden = !offer;
      // "Skip" alone on the button. A segment set to "show" is as often an
      // intro, an endcard or a recap as it is a sponsor, and naming the
      // category on the button was wrong more often than it was right. The
      // category still goes to a screen reader, which has room for it.
      if (offer) {
        sponsorSkip.setAttribute(
          "aria-label",
          offer.label ? `Skip ${offer.label}` : "Skip this section",
        );
      }
    }

    function applySponsorSegments() {
      offerSponsorSkip(currentPosition());
      if (!sponsorSegments.length || scrubbing) return;
      const position = currentPosition();
      for (const segment of sponsorSegments) {
        if (segment.action !== "skip" || sponsorHandled.has(segment.uuid)) continue;
        const start = Number(segment.start_seconds);
        const end = Number(segment.end_seconds);
        if (!(position >= start && position < end)) continue;
        sponsorHandled.add(segment.uuid);
        // Never past the end: a segment that runs to the last frame would
        // otherwise end the video rather than skip within it.
        const target = Math.min(end, (video.duration || end) - 0.05);
        if (target > position) video.currentTime = target;
        if (sponsorNotify && segment.message) announce(segment.message);
        break;
      }
    }

    /** A brief line under the controls, so a jump does not look like a fault. */
    function announce(message) {
      if (!sponsorNotice) return;
      sponsorNotice.textContent = message;
      sponsorNotice.classList.add("is-visible");
      if (sponsorNoticeTimer) clearTimeout(sponsorNoticeTimer);
      sponsorNoticeTimer = setTimeout(() => sponsorNotice.classList.remove("is-visible"), 2600);
    }

    function previewFrame(position) {
      const data = previewFrames;
      if (!data || !data.page_urls?.length || !data.duration_per_frame_ms) return null;
      const count = Math.max(1, Number(data.total_count) || 1);
      const frame = Math.min(count - 1, Math.floor((position * 1000) / data.duration_per_frame_ms));
      const columns = Math.max(1, Number(data.frames_per_page_x) || 1);
      const rows = Math.max(1, Number(data.frames_per_page_y) || 1);
      const perPage = columns * rows;
      const page = Math.min(data.page_urls.length - 1, Math.floor(frame / perPage));
      const cell = frame % perPage;
      return {
        url: data.page_urls[page],
        columns,
        rows,
        column: cell % columns,
        row: Math.floor(cell / columns),
      };
    }

    function showScrubPreview(position) {
      if (!seekPreview || !progress) return;
      const length = Number.isFinite(video.duration) ? video.duration : 0;
      const percent = length > 0 ? Math.min(100, Math.max(0, (position / length) * 100)) : 0;
      seekPreview.hidden = false;
      seekPreview.style.left = `clamp(5rem, ${percent}%, calc(100% - 5rem))`;
      if (previewTime) previewTime.textContent = formatTime(position);
      const chapter = chapterAt(position);
      if (previewChapter) {
        previewChapter.textContent = chapter?.title || "";
        previewChapter.hidden = !chapter?.title;
      }
      const frame = previewFrame(position);
      if (previewImage) {
        const url = frame?.url || root.dataset.thumbnail || video.poster || "";
        previewImage.style.backgroundImage = url ? `url(${JSON.stringify(url)})` : "none";
        if (frame) {
          previewImage.style.backgroundSize = `${frame.columns * 100}% ${frame.rows * 100}%`;
          const x = frame.columns > 1 ? (frame.column / (frame.columns - 1)) * 100 : 0;
          const y = frame.rows > 1 ? (frame.row / (frame.rows - 1)) * 100 : 0;
          previewImage.style.backgroundPosition = `${x}% ${y}%`;
        } else {
          previewImage.style.backgroundSize = "cover";
          previewImage.style.backgroundPosition = "center";
        }
      }
    }

    function hideScrubPreview() {
      if (seekPreview) seekPreview.hidden = true;
    }

    function setMetadata(metadata) {
      const next = metadata || {};
      const nextCaptions = next.captions || [];
      captions = nextCaptions;
      selectedCaption = Number.isInteger(next.selectedCaption) ? next.selectedCaption : null;
      captionsEnabled = Boolean(next.captionsEnabled);
      chapters = (next.chapters || [])
        .slice()
        .sort((left, right) => Number(left.start_seconds) - Number(right.start_seconds));
      previewFrames = next.previewFrames || null;
      const nextSegments = (next.sponsorSegments || [])
        .slice()
        .sort((left, right) => Number(left.start_seconds) - Number(right.start_seconds));
      // A different set of segments is a different video, or the viewer changing
      // what they want skipped; either way the "already skipped" memory is stale.
      const changed =
        nextSegments.length !== sponsorSegments.length ||
        nextSegments.some((segment, index) => segment.uuid !== sponsorSegments[index]?.uuid);
      if (changed) sponsorHandled.clear();
      sponsorSegments = nextSegments;
      sponsorNotify = next.sponsorNotify !== false;
      autoplayEnabled = Boolean(next.autoplay);
      applyCaptionState();
      requestAnimationFrame(applyCaptionState);
      setTimeout(applyCaptionState, 100);
      renderTimeline();
    }

    function showSeekFeedback(direction, seconds = 10) {
      const feedback = direction < 0 ? feedbackLeft : feedbackRight;
      if (!feedback) return;
      const label = feedback.querySelector("span");
      if (label) label.textContent = `${seconds} seconds`;
      feedback.classList.remove("show-feedback");
      void feedback.offsetWidth;
      feedback.classList.add("show-feedback");
      if (feedbackTimer) clearTimeout(feedbackTimer);
      feedbackTimer = setTimeout(() => feedback.classList.remove("show-feedback"), 650);
    }

    function seekBy(amount, feedbackSeconds = Math.abs(amount)) {
      const length = Number.isFinite(video.duration) ? video.duration : Infinity;
      video.currentTime = Math.max(0, Math.min(length, video.currentTime + amount));
      showSeekFeedback(amount < 0 ? -1 : 1, feedbackSeconds);
      updateTimeline();
      showControls(false);
    }

    function runSurfaceClickSequenceAction(clientX, clickCount = 2) {
      const bounds = video.getBoundingClientRect();
      if (!bounds.width) return;
      const position = (clientX - bounds.left) / bounds.width;
      const cumulativeSeconds = Math.max(10, (clickCount - 1) * 10);
      if (position < 0.35) {
        seekBy(-10, cumulativeSeconds);
      } else if (position > 0.65) {
        seekBy(10, cumulativeSeconds);
      } else if (clickCount === 2 && matchMedia("(hover: hover) and (pointer: fine)").matches) {
        Promise.resolve(toggleFullscreen(root)).catch(() => {});
      }
    }

    function setSpeed(speed) {
      if (!Number.isFinite(speed) || speed <= 0) return;
      video.playbackRate = speed;
      video.defaultPlaybackRate = speed;
      updateSpeed();
    }

    function cycleQuickSpeed() {
      const current = quickSpeedSteps.findIndex(
        (speed) => Math.abs(speed - effectivePlaybackRate()) < 0.01,
      );
      setSpeed(quickSpeedSteps[(current + 1) % quickSpeedSteps.length]);
    }

    function toggleOptions() {
      if (!optionsMenu) return;
      optionsMenu.hidden = !optionsMenu.hidden;
      controls.classList.toggle("options-open", !optionsMenu.hidden);
      showControls(!optionsMenu.hidden);
      if (!optionsMenu.hidden) refreshQualities();
    }

    function closeOptions() {
      if (!optionsMenu || optionsMenu.hidden) return;
      optionsMenu.hidden = true;
      controls.classList.remove("options-open");
      // The menu was what kept the bar up; hand the dwell back to the timer.
      showControls(false);
    }

    function refreshQualities() {
      if (!qualityOptions || !window.TawnyTransport?.qualityState) return;
      const state = window.TawnyTransport.qualityState(video);
      if (!state) return;
      chosenHeight = state.auto ? null : state.selectedHeight;
      qualityOptions.replaceChildren();
      const choices = [
        { value: null, label: state.activeHeight ? `Auto · ${state.activeHeight}p` : "Auto" },
      ].concat(state.heights.map((height) => ({ value: height, label: `${height}p` })));
      for (const choice of choices) {
        const button = document.createElement("button");
        button.type = "button";
        button.className = "player-option-button";
        button.dataset.playerQuality = choice.value === null ? "auto" : String(choice.value);
        button.textContent = choice.label;
        button.classList.toggle("selected", choice.value === chosenHeight);
        qualityOptions.appendChild(button);
      }
    }

    async function runAction(action) {
      switch (action) {
        case "toggle":
          // Whichever way it goes, this is now the viewer's decision rather
          // than autoplay's.
          if (starting) setStarting(false);
          if (video.paused) {
            setPlaybackIntent(true);
            await video.play().catch(() => {});
          } else {
            setPlaybackIntent(false);
            video.pause();
          }
          break;
        case "back":
          if (fullscreenElement()) {
            await Promise.resolve(toggleFullscreen(root)).catch(() => {});
            // Let Android finish its fullscreen/orientation transition before
            // navigating the player sheet away. Doing both in the same frame
            // can make the system restore the just-removed fullscreen view.
            await new Promise((resolve) => requestAnimationFrame(resolve));
          }
          root.querySelector("[data-player-minimize]")?.click();
          break;
        case "rewind":
          seekBy(-10);
          break;
        case "forward":
          seekBy(10);
          break;
        case "mute":
          video.muted = !video.muted;
          updateMuted();
          break;
        case "captions":
          controls.querySelector("[data-player-caption-toggle]")?.click();
          break;
        case "chapters":
          (
            controls.querySelector("[data-player-chapters-open]") ||
            root.querySelector("[data-player-chapters-open]") ||
            document.querySelector("[data-player-chapters-open]")
          )?.click();
          break;
        case "speed":
          cycleQuickSpeed();
          break;
        case "settings":
          toggleOptions();
          break;
        case "pip": {
          let pipChanged = false;
          try {
            if (document.pictureInPictureElement) {
              await document.exitPictureInPicture();
              pipChanged = true;
            } else if (video.requestPictureInPicture) {
              await video.requestPictureInPicture();
              pipChanged = true;
            } else if (video.webkitSupportsPresentationMode) {
              video.webkitSetPresentationMode("picture-in-picture");
              pipChanged = true;
            }
          } catch (_) {}
          if (!pipChanged) {
            beginPipTransition();
            setPlaybackIntent(!video.paused && !video.ended);
            const portrait = video.videoHeight > video.videoWidth;
            const selector = portrait
              ? "[data-player-native-pip-portrait]"
              : "[data-player-native-pip-landscape]";
            root.querySelector(selector)?.click();
          }
          break;
        }
        case "fullscreen":
          await Promise.resolve(toggleFullscreen(root)).catch(() => {});
          break;
      }
      if (action !== "settings") scheduleHide();
    }

    listen(controls, "click", (event) => {
      const speed = event.target.closest("[data-player-speed]");
      if (speed) {
        setSpeed(Number(speed.dataset.playerSpeed));
        return;
      }
      const quality = event.target.closest("[data-player-quality]");
      if (quality && window.TawnyTransport?.setQuality) {
        const value = quality.dataset.playerQuality;
        window.TawnyTransport.setQuality(video, value === "auto" ? null : Number(value));
        refreshQualities();
        return;
      }
      const button = event.target.closest("[data-player-action]");
      if (button) runAction(button.dataset.playerAction);
    });

    if (progress) {
      // Ending a scrub has to be idempotent and reachable from every exit
      // route. A pointerdown that never changes the value (pressing straight
      // on the thumb) emits no `input` and no `change`, so relying on `change`
      // alone would leave the player paused with a frozen timeline, since
      // updateTimeline() bails out while `scrubbing` is set.
      const positionScrubAt = (clientX) => {
        const bounds = progress.getBoundingClientRect();
        if (!bounds.width) return;
        const ratio = Math.max(0, Math.min(1, (clientX - bounds.left) / bounds.width));
        progress.value = String(Math.round(ratio * 10000));
        const length = Number.isFinite(video.duration) ? video.duration : 0;
        const position = ratio * length;
        if (currentTime) currentTime.textContent = formatTime(position);
        progress.style.setProperty("--player-progress", `${ratio * 100}%`);
        progress.setAttribute("aria-valuetext", `${formatTime(position)} of ${formatTime(length)}`);
        showScrubPreview(position);
      };

      // Hovering must be informational only: unlike a scrub it must neither
      // move the thumb nor replace the displayed playback time. The same
      // preview renderer supplies a storyboard frame when the server has one,
      // with the video's thumbnail as a useful fallback.
      const previewAt = (clientX) => {
        const bounds = progress.getBoundingClientRect();
        if (!bounds.width) return;
        const ratio = Math.max(0, Math.min(1, (clientX - bounds.left) / bounds.width));
        const length = Number.isFinite(video.duration) ? video.duration : 0;
        showScrubPreview(ratio * length);
      };

      const beginScrub = () => {
        if (scrubbing) return;
        scrubbing = true;
        resumeAfterScrub = !video.paused && !video.ended;
        if (resumeAfterScrub) video.pause();
        showControls(true);
      };

      const finishScrub = (commit) => {
        if (!scrubbing) return;
        scrubbing = false;
        const length = Number.isFinite(video.duration) ? video.duration : 0;
        if (commit && length > 0) {
          video.currentTime = (Number(progress.value) / 10000) * length;
        }
        hideScrubPreview();
        if (resumeAfterScrub) video.play().catch(() => {});
        resumeAfterScrub = false;
        updateTimeline();
        scheduleHide();
      };

      listen(progress, "pointerdown", (event) => {
        beginScrub();
        try {
          progress.setPointerCapture(event.pointerId);
        } catch (_) {}
        positionScrubAt(event.clientX);
      });
      listen(progress, "pointermove", (event) => {
        if (scrubbing) {
          positionScrubAt(event.clientX);
        } else if (event.pointerType === "mouse") {
          previewAt(event.clientX);
        }
      });
      listen(progress, "pointerenter", (event) => {
        if (event.pointerType !== "mouse") return;
        timelineHovering = true;
        previewAt(event.clientX);
        showControls(false);
      });
      listen(progress, "pointerleave", (event) => {
        if (event.pointerType !== "mouse" || scrubbing) return;
        timelineHovering = false;
        hideScrubPreview();
        showControls(false, CONTROLS_LEAVE_HIDE_MS);
      });
      listen(progress, "input", () => {
        // Android WebView can hand a range drag directly to the native range
        // control without first surfacing pointerdown. Treat the first input
        // as the start so the clock freezes at the preview position and the
        // media still pauses throughout the gesture.
        beginScrub();
        const length = Number.isFinite(video.duration) ? video.duration : 0;
        const position = (Number(progress.value) / 10000) * length;
        if (currentTime) currentTime.textContent = formatTime(position);
        progress.style.setProperty("--player-progress", `${Number(progress.value) / 100}%`);
        showScrubPreview(position);
      });
      listen(progress, "change", () => finishScrub(true));
      listen(progress, "pointerup", () => finishScrub(true));
      listen(progress, "pointercancel", () => finishScrub(false));
      // Range inputs capture the pointer, but a capture lost to a scroll
      // gesture or a release over another element still has to settle.
      listen(progress, "lostpointercapture", () => finishScrub(true));
      listen(window, "pointerup", () => finishScrub(true));
      listen(progress, "keyup", () => finishScrub(true));
      listen(progress, "blur", () => finishScrub(true));
    }

    // Clicking anywhere on the video toggles playback with a mouse - unless the
    // pointer just travelled far enough to be a swipe, in which case the
    // gesture owns it. A finger, or any pointer that cannot hover, toggles the
    // controls instead: a tap shows them and the next hides them, and only the
    // play button plays or pauses. On a phone the controls are the only way to
    // reach seeking and settings, so a tap that also stopped the video made
    // every look at them cost the viewer their place.
    //
    // Bound on the player root, not the video. A mouse gesture takes pointer
    // capture on the root at pointerdown, and capture retargets the click that
    // follows to the capture element — so a listener on the video never heard
    // a desktop click at all, and only touch (which captures lazily) worked.
    // Anything clickable inside the player owns its own click. Listing only the
    // data-attribute controls missed the buttons that carry a Dioxus handler
    // instead - Back, and the hidden bridges that `runAction` clicks - so their
    // clicks bubbled to the root and toggled playback on the way out.
    const isPlayerChrome = (target) =>
      !!(target instanceof Element) &&
      !!target.closest(
        "button, a, input, select, [role='button'], [data-player-action], [data-player-progress], .player-options-menu",
      );

    listen(root, "click", (event) => {
      // The controls own their own clicks, including the options menu, which
      // must not be torn down by the press that is choosing from it.
      if (isPlayerChrome(event.target)) return;
      closeOptions();
      if (swallowNextClick) {
        swallowNextClick = false;
        return;
      }
      // Pointer releases are the primary signal. click.detail remains the
      // fallback for engines that omit pointer events, and naturally carries
      // triple/quadruple click counts for cumulative edge seeking.
      if (event.detail >= 2) {
        lastClickSequenceActionAt = Date.now();
        handleSurfaceClickSequenceAction(event.clientX, event.detail);
        return;
      }
      if (surfaceClickTimer) clearTimeout(surfaceClickTimer);
      surfaceClickCommitted = false;
      const touchTap = surfaceTapIsTouch;
      surfaceClickTimer = setTimeout(() => {
        surfaceClickTimer = null;
        if (touchTap) {
          // Raised by an earlier tap (or held up by a pause): this tap is the
          // one that puts them away. The capture handler below answers the
          // tap that raises them, so this only runs while they are showing.
          if (controls.classList.contains("controls-visible")) hideControls();
          else showControls(false);
          return;
        }
        surfaceClickCommitted = true;
        runAction("toggle");
        showControls(false);
      }, 280);
    });

    // Vertical swipes: up enters fullscreen, down leaves it, and a downward
    // swipe outside fullscreen shrinks the player to the mini bar.
    let gestureStart = null;
    let swallowNextClick = false;
    let surfaceClickTimer = null;
    let surfaceClickCommitted = false;
    let tapOpensControls = false;
    // Whether the press behind the next surface click was a finger (or a
    // pointer that cannot hover), which decides what the click does.
    let surfaceTapIsTouch = false;
    let surfacePressSequence = null;
    let lastPointerTapAt = 0;
    let lastClickSequenceActionAt = 0;
    const DOUBLE_PRESS_MS = 500;
    const TAP_SLOP = 14;

    // A pointer that can hover has already raised the bar by the time it is
    // pressed; one that cannot has to tap for it.
    const canHover = () => matchMedia("(hover: hover) and (pointer: fine)").matches;

    const surfaceZoneAt = (clientX) => {
      const bounds = video.getBoundingClientRect();
      if (!bounds.width) return "center";
      const position = (clientX - bounds.left) / bounds.width;
      if (position < 0.35) return "left";
      if (position > 0.65) return "right";
      return "center";
    };

    const handleSurfaceClickSequenceAction = (clientX, clickCount) => {
      if (surfaceClickTimer) clearTimeout(surfaceClickTimer);
      surfaceClickTimer = null;
      // If a slow main thread allowed the first tap to toggle already, undo
      // that toggle before applying the double-tap action.
      if (surfaceClickCommitted) runAction("toggle");
      surfaceClickCommitted = false;
      runSurfaceClickSequenceAction(clientX, clickCount);
    };

    const trackGesture = (event) => {
      if (!gestureStart) return;
      const dy = event.clientY - gestureStart.y;
      const dx = event.clientX - gestureStart.x;
      if (!gestureStart.committed) {
        // Claim the gesture only once it is clearly vertical. Until then the
        // page keeps its normal scrolling.
        if (Math.abs(dy) < 10 || Math.abs(dy) <= Math.abs(dx)) return;
        gestureStart.committed = true;
        try {
          root.setPointerCapture(event.pointerId);
        } catch (_) {}
      }
      // Follow the finger the way YouTube does: swiping up grows the video
      // toward its fullscreen size so the gesture feels attached to it.
      const progress = Math.min(1, Math.max(0, -dy / 260));
      root.style.setProperty("--player-swipe-progress", progress.toFixed(3));
      root.classList.toggle("player-swiping", progress > 0);
      // Suppress the page's own scrolling while we own the gesture.
      if (event.cancelable) event.preventDefault();
    };

    const resetGestureVisuals = () => {
      root.style.removeProperty("--player-swipe-progress");
      root.classList.remove("player-swiping");
    };

    // Capture, so this runs before any control's own handler: by the time the
    // click is dispatched the bar has come up, and the browser hit-tests
    // again, so a tap on the middle of the video arrives at the play button
    // that has just appeared there. What matters is whether the bar was up
    // when the finger went down, which is what this was told.
    listen(
      root,
      "click",
      (event) => {
        // The swipe-down reaches the minimize bridge with a scripted click.
        // Taken as the tap that raises the bar, it was stopped here before the
        // bridge heard it, so the controls flashed and the player stayed put.
        if (!tapOpensControls || !event.isTrusted) return;
        tapOpensControls = false;
        // Whatever this click landed on, the press began on the picture with
        // the bar down. The bar can come up before the finger lifts (any
        // playback event raises it), and the click then hit-tests onto the
        // button that appeared under the finger. A press that began on a
        // button clears the flag at pointerdown, so Back and Try again keep
        // their own clicks.
        event.preventDefault();
        event.stopPropagation();
        showControls(false);
      },
      { capture: true },
    );

    listen(root, "pointerdown", (event) => {
      swallowNextClick = false;
      // Capture retargets the later click to `root`. Every interactive piece
      // of chrome must opt out here, not only controls implemented by the JS
      // action dispatcher. In particular the Rust-owned autoplay switch used
      // to be captured as a surface tap, so one press toggled autoplay and
      // play/pause at the same time.
      if (isPlayerChrome(event.target)) {
        gestureStart = null;
        tapOpensControls = false;
        return;
      }
      gestureStart = {
        x: event.clientX,
        y: event.clientY,
        at: Date.now(),
        committed: event.pointerType === "mouse",
        deadZone: !document.fullscreenElement && inTopDeadZone(event.clientY),
      };
      // A finger has no hover, so a tap is the only way to reach the controls
      // - and taking that tap as play/pause stopped the video every time
      // someone went looking for the seek bar. The bar answers the first tap,
      // the video answers the next. Read here rather than at the click,
      // because the press itself raises the bar on the way through.
      surfaceTapIsTouch = event.pointerType !== "mouse" || !canHover();
      tapOpensControls = surfaceTapIsTouch && !controls.classList.contains("controls-visible");
      if (gestureStart.committed) {
        // A mouse drag never competes with scrolling, so capture immediately;
        // a release outside the player still has to report back here.
        try {
          root.setPointerCapture(event.pointerId);
        } catch (_) {}
      }
    });

    listen(root, "pointermove", trackGesture);

    const endGesture = (event) => {
      if (!gestureStart) return;
      const dx = event.clientX - gestureStart.x;
      const dy = event.clientY - gestureStart.y;
      const elapsed = Date.now() - gestureStart.at;
      const startedInDeadZone = gestureStart.deadZone;
      gestureStart = null;
      resetGestureVisuals();
      try {
        root.releasePointerCapture(event.pointerId);
      } catch (_) {}
      const isTap = elapsed <= 800 && Math.abs(dx) <= TAP_SLOP && Math.abs(dy) <= TAP_SLOP;
      if (isTap) {
        const now = Date.now();
        const zone = surfaceZoneAt(event.clientX);
        if (
          surfacePressSequence &&
          now - surfacePressSequence.at <= DOUBLE_PRESS_MS &&
          surfacePressSequence.zone === zone
        ) {
          surfacePressSequence.count += 1;
          surfacePressSequence.at = now;
          swallowNextClick = true;
          handleSurfaceClickSequenceAction(event.clientX, surfacePressSequence.count);
        } else {
          surfacePressSequence = { at: now, zone, count: 1 };
        }
        lastPointerTapAt = now;
        return;
      }
      // Only a tap raises the bar. A drag produces no click to consume this,
      // so left set it would swallow whatever the next real tap was for.
      if (Math.abs(dx) > TAP_SLOP || Math.abs(dy) > TAP_SLOP) tapOpensControls = false;
      if (!isVerticalFlick(dx, dy, elapsed)) return;
      // A swipe is not a tap: keep the click that follows from toggling play.
      swallowNextClick = true;
      const fullscreen = document.fullscreenElement === root;
      if (dy < 0) {
        // Requested straight from the handler: fullscreen needs the user
        // activation this event carries, and awaiting anything first loses it.
        if (!fullscreen) enterFullscreenFor(root);
      } else if (fullscreen) {
        document.exitFullscreen && document.exitFullscreen();
      } else if (!startedInDeadZone && !root.closest(".persistent-player.mini")) {
        // Already minimized, a downward flick is the mini bar's dismiss, which
        // the document-level handler above performs.
        controls.querySelector("[data-player-minimize]")?.click();
      }
    };

    listen(root, "pointerup", endGesture);
    listen(root, "pointercancel", () => {
      gestureStart = null;
      tapOpensControls = false;
      resetGestureVisuals();
    });
    // Native dblclick is an additional fallback. Ignore the synthesized event
    // when its pointerup or click(detail=2) already handled the same presses.
    listen(root, "dblclick", (event) => {
      if (isPlayerChrome(event.target)) return;
      event.preventDefault();
      const now = Date.now();
      if (now - lastPointerTapAt < 250 || now - lastClickSequenceActionAt < 250) return;
      lastClickSequenceActionAt = now;
      handleSurfaceClickSequenceAction(event.clientX, 2);
    });
    listen(root, "pointermove", (event) => {
      // Hovering is a mouse idea. A finger never holds perfectly still, so a
      // touch press sends one of these too - and raising the bar mid-press
      // put a control under the finger that was not there when it went down.
      // The tap then landed on whichever button had appeared, which is how
      // reaching for the controls paused the video.
      if (event.pointerType !== "mouse" || !canHover()) return;
      showControls(false);
    });
    listen(root, "pointerleave", () => {
      timelineHovering = false;
      hideScrubPreview();
      showControls(false, CONTROLS_LEAVE_HIDE_MS);
    });
    listen(root, "keydown", (event) => {
      if (event.target instanceof HTMLInputElement) return;
      const key = event.key.toLowerCase();
      if (key === "escape" && optionsMenu && !optionsMenu.hidden) {
        event.preventDefault();
        closeOptions();
        return;
      }
      const handled = [" ", "k", "j", "l", "arrowleft", "arrowright", "m", "f", "c"];
      if (!handled.includes(key)) return;
      event.preventDefault();
      if (key === " " || key === "k") runAction("toggle");
      else if (key === "j" || key === "arrowleft") seekBy(-10);
      else if (key === "l" || key === "arrowright") seekBy(10);
      else if (key === "m") runAction("mute");
      else if (key === "f") runAction("fullscreen");
      else if (key === "c") runAction("captions");
    });

    listen(video, "play", () => {
      setPlaybackIntent(true);
      updatePlaybackState();
      root.querySelector("[data-player-native-playback-start]")?.click();
    });
    listen(video, "pause", () => {
      updatePlaybackState();
      if (scrubbing) return;
      // Native PiP controls write their intent directly onto the media
      // element before pausing. Read it first so an explicit PiP Pause is not
      // immediately undone by lifecycle recovery.
      if (video.__tawnyPlaybackIntent === false) playbackIntent = false;
      if (playbackIntent) {
        // Shaka can briefly pause the media element while recovering a range
        // request or rebuilding a SourceBuffer. That is not a user Pause, so
        // keep Android's audio session alive and let Shaka resume it. During an
        // actual lifecycle edge, actively ask Chromium to resume as well.
        if (pipTransition || document.visibilityState !== "visible") {
          recoverIntendedPlayback(80);
        }
        return;
      }
      root.querySelector("[data-player-native-playback-stop]")?.click();
    });
    // A headset button or the lock screen pauses the element directly, and
    // while the page is hidden the pause listener above reads that as the
    // browser suspending playback and resumes it at once. Taking the actions
    // over records them as the viewer's intent first. The native apps route
    // these through their own session and set the intent on the element.
    const mediaSession = "mediaSession" in navigator ? navigator.mediaSession : null;
    const sessionActions = {
      play: () => {
        if (starting) setStarting(false);
        setPlaybackIntent(true);
        video.play().catch(() => {});
      },
      pause: () => {
        if (starting) setStarting(false);
        setPlaybackIntent(false);
        video.pause();
      },
      stop: () => {
        setPlaybackIntent(false);
        video.pause();
      },
      seekbackward: (details) => seekBy(-(details?.seekOffset || 10)),
      seekforward: (details) => seekBy(details?.seekOffset || 10),
      seekto: (details) => {
        if (Number.isFinite(details?.seekTime)) video.currentTime = details.seekTime;
      },
    };
    function setSessionActions(enabled) {
      if (!mediaSession) return;
      // The next video can attach before this one is torn down, and its
      // handlers are not this controller's to clear.
      if (!enabled && sessionOwner !== video) return;
      sessionOwner = enabled ? video : null;
      for (const [action, handler] of Object.entries(sessionActions)) {
        // Browsers reject actions they do not know rather than ignoring them.
        try {
          mediaSession.setActionHandler(action, enabled ? handler : null);
        } catch (_) {}
      }
    }
    setSessionActions(true);
    const reportSessionState = () => {
      if (mediaSession) mediaSession.playbackState = playbackIntent ? "playing" : "paused";
    };
    listen(video, "play", reportSessionState);
    listen(video, "pause", reportSessionState);
    listen(video, "timeupdate", trackStart);
    // The transport's play() was refused - a browser that wants a gesture
    // first. The Play button is then exactly what the viewer needs.
    listen(video, "tawnyautoplayblocked", () => {
      setStarting(false);
      updatePlaybackState();
    });
    listen(video, "ended", () => {
      // Advancing is Rust's decision: it owns the queue and the setting. This
      // only reports that the video finished.
      const next = autoplayEnabled && controls.querySelector("[data-player-autoplay-next]");
      if (next?.dataset.hasNext === "true") {
        // Moving straight on. Putting up the paused bar here flashed Play and
        // the controls over the last frame before the next video took over;
        // the stage shows it loading instead. The media session stays active,
        // since playback is carrying on.
        setStarting(true);
        hideControls();
        next.click();
        return;
      }
      setStarting(false);
      setPlaybackIntent(false);
      updatePlaybackState();
      root.querySelector("[data-player-native-playback-stop]")?.click();
      next?.click();
    });
    // The system media controls extrapolate position from the last report, so
    // a seek or speed change has to be reported too. Uses the intent rather
    // than `paused`, which Shaka flips briefly while buffering a seek.
    const reportPlaybackState = () => {
      const bridge = playbackIntent
        ? "[data-player-native-playback-start]"
        : "[data-player-native-playback-stop]";
      root.querySelector(bridge)?.click();
    };
    listen(video, "seeked", reportPlaybackState);
    listen(video, "ratechange", reportPlaybackState);
    listen(video, "durationchange", reportPlaybackState);
    listen(window, "tawnynativepiprequest", beginPipTransition);
    listen(window, "tawnynativepictureinpicturechange", () => {
      // Entering and leaving PiP can each pause the WebView after the native
      // mode callback. Keep a short grace window around both edges.
      beginPipTransition();
      recoverIntendedPlayback(60);
    });
    listen(window, "tawnynativeplaybackresume", () => recoverIntendedPlayback(0));
    listen(video, "enterpictureinpicture", () => {
      beginPipTransition();
      recoverIntendedPlayback(0);
    });
    listen(video, "leavepictureinpicture", () => {
      beginPipTransition();
      recoverIntendedPlayback(0);
    });
    if (sponsorSkip) {
      // The stage toggles playback on a pointer press, and a press that lands
      // on this button is not a press on the video.
      for (const name of ["pointerdown", "pointerup", "mousedown", "mouseup"]) {
        listen(sponsorSkip, name, (event) => event.stopPropagation());
      }
      listen(sponsorSkip, "click", (event) => {
        event.preventDefault();
        event.stopPropagation();
        const offer = sponsorSkipOffer;
        if (!offer) return;
        sponsorHandled.add(offer.uuid);
        const end = Number(offer.end_seconds);
        // Never past the end, for the same reason an automatic skip is not.
        const target = Math.min(end, (video.duration || end) - 0.05);
        if (target > currentPosition()) video.currentTime = target;
        offerSponsorSkip(currentPosition());
      });
    }
    listen(video, "durationchange", updateTimeline);
    listen(video, "durationchange", renderTimeline);
    listen(window, "resize", renderTimeline);
    listen(video, "progress", updateTimeline);
    listen(video, "ratechange", updateSpeed);
    listen(video, "volumechange", updateMuted);
    listen(video, "waiting", () => buffering?.classList.add("buffering-visible"));
    listen(video, "playing", () => buffering?.classList.remove("buffering-visible"));
    listen(video, "canplay", () => buffering?.classList.remove("buffering-visible"));
    listen(video, "loadedmetadata", updateCaptions);
    Array.from(video.querySelectorAll("track[data-tawny-caption]")).forEach((track) => {
      listen(track, "load", applyCaptionState);
    });
    listen(video, "tawnytransportchange", () => setTimeout(refreshQualities, 0));
    listen(video, "tawnyqualitychange", refreshQualities);
    // The iOS fallback puts the video element itself fullscreen, not the
    // root, so anything inside this player counts.
    const isPlayerFullscreen = () => {
      const active = fullscreenElement();
      return Boolean(active && (active === root || root.contains(active)));
    };
    // Rotation used to be requested only from the fullscreen request's own
    // success callback, using the dimensions read before it. So it was skipped
    // whenever the player got to fullscreen any other way - the video-element
    // fallback after a refused request - or before the first frame had given
    // the video a size. Once per fullscreen, from whatever state the player
    // is actually in, and again when the size arrives.
    let landscapeRequested = false;
    const applyFullscreenOrientation = () => {
      if (landscapeRequested || !isPlayerFullscreen()) return;
      if (root.dataset.autoLandscape === "false") return;
      if (!(video.videoWidth > video.videoHeight)) return;
      landscapeRequested = true;
      root.querySelector("[data-player-native-landscape]")?.click();
      // Orientation locking only exists on mobile and rejects when
      // unsupported, so failures are ignored.
      try {
        screen.orientation?.lock?.("landscape")?.catch?.(() => {});
      } catch (_) {}
    };
    let systemBarsHidden = false;
    const syncFullscreen = () => {
      const fullscreen = isPlayerFullscreen();
      if (fullscreen !== systemBarsHidden) {
        systemBarsHidden = fullscreen;
        root
          .querySelector(
            fullscreen
              ? "[data-player-native-system-bars-hide]"
              : "[data-player-native-system-bars-show]",
          )
          ?.click();
      }
      if (fullscreen) {
        applyFullscreenOrientation();
      } else {
        landscapeRequested = false;
        root.querySelector("[data-player-native-orientation-unlock]")?.click();
      }
      root.classList.toggle("is-fullscreen", fullscreen);
      controls.classList.toggle("is-fullscreen", fullscreen);
      fullscreenButton?.setAttribute(
        "aria-label",
        fullscreen ? "Exit fullscreen" : "Enter fullscreen",
      );
      fullscreenButton?.setAttribute("title", fullscreen ? "Exit fullscreen" : "Fullscreen");
      showControls(false);
    };
    listen(document, "fullscreenchange", syncFullscreen);
    listen(document, "webkitfullscreenchange", syncFullscreen);
    listen(video, "webkitendfullscreen", syncFullscreen);
    // `resize` is the media element's own event for a change of intrinsic size.
    listen(video, "resize", applyFullscreenOrientation);
    listen(video, "loadedmetadata", applyFullscreenOrientation);
    // Fullscreen can already be on: a swipe up while the stream was resolving
    // entered it before there was a controller (or the bridges) to hear it.
    if (isPlayerFullscreen()) syncFullscreen();

    root.tabIndex = 0;
    setPlaybackIntent(playbackIntent);
    // The markup renders the bar raised, which is right for everything but a
    // video about to start on its own.
    setStarting(starting);
    if (starting) hideControls();
    updatePlaybackState();
    updateTimeline();
    updateSpeed();
    updateMuted();
    updateCaptions();
    setTimeout(refreshQualities, 0);
    // Drawing the timeline writes text, measures the track and rebuilds its
    // gradient - every frame, which is only worth it while there is a bar to
    // see. Hidden, mini, or mid route transition, it was pure main-thread cost,
    // and during a transition it re-rendered the very player the morph was
    // painting live. Segment skips still run every frame regardless: a skip
    // cannot wait for someone to look at the bar.
    const timelineOnScreen = () =>
      (scrubbing || controls.classList.contains("controls-visible")) &&
      !root.parentElement?.classList.contains("mini") &&
      !document.documentElement.dataset.routeTransition;
    const animateTimeline = () => {
      if (timelineOnScreen()) updateTimeline();
      else applySponsorSegments();
      animationFrame = requestAnimationFrame(animateTimeline);
    };
    animationFrame = requestAnimationFrame(animateTimeline);

    const controller = {
      setMetadata,
      destroy() {
        abort.abort();
        setSessionActions(false);
        if (animationFrame) cancelAnimationFrame(animationFrame);
        if (hideTimer) clearTimeout(hideTimer);
        if (feedbackTimer) clearTimeout(feedbackTimer);
        if (surfaceClickTimer) clearTimeout(surfaceClickTimer);
        if (playbackRecoveryTimer) clearTimeout(playbackRecoveryTimer);
        if (pipTransitionTimer) clearTimeout(pipTransitionTimer);
      },
    };
    controllers.set(video, controller);
    // The metadata is kept rather than consumed. A controller is rebuilt
    // whenever this element is attached again - a retried stream, a quality
    // change - and a fresh one starts with no chapters and no segments, so
    // the timeline lost its divisions and its colours until something
    // upstream happened to send them a second time.
    const known = lastMetadata.get(video);
    if (known) controller.setMetadata(known);
  }

  function detach(video) {
    const controller = controllers.get(video);
    if (!controller) return;
    controller.destroy();
    controllers.delete(video);
  }

  /**
   * Fold an answer about the current video into what is already known.
   *
   * For one video this only ever adds. The page describes a video from more
   * than one source - the copy cached in the library, the details request,
   * the SponsorBlock lookup - and they answer at different times, more than
   * once, and sometimes not at all. An answer carrying nothing is not news
   * that the video has no chapters; it is a source that has not spoken yet,
   * or could not reach anyone. Letting one of those through is what kept
   * taking the timeline's divisions and colours away part way through a
   * video, whichever source happened to be the quiet one that time.
   *
   * Changing video replaces everything, including with nothing. So does
   * turning SponsorBlock off, which is an answer rather than a silence.
   */
  function mergeMetadata(known, incoming) {
    const next = incoming || {};
    if (!known || !next.videoId || next.videoId !== known.videoId) return next;
    const spoken = (answer, heard) => (answer && answer.length ? answer : heard || []);
    return {
      ...next,
      captions: spoken(next.captions, known.captions),
      chapters: spoken(next.chapters, known.chapters),
      previewFrames: next.previewFrames || known.previewFrames || null,
      sponsorSegments:
        next.sponsorEnabled === false ? [] : spoken(next.sponsorSegments, known.sponsorSegments),
    };
  }

  function setMetadata(video, metadata) {
    const merged = mergeMetadata(lastMetadata.get(video), metadata);
    lastMetadata.set(video, merged);
    const controller = controllers.get(video);
    if (controller) controller.setMetadata(merged);
  }

  window.TawnyPlayerControls = { attach, detach, setMetadata };
})();
