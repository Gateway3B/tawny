# Android input and scroll performance

## November 6, 2026 pass

The touch path stays inside the WebView. Rust receives completed actions;
pointer movement and settling do not require synchronous Rust event handlers.

- g3-ui swipe rows measure direction, action widths and the visible starting
  transform at press time. Moves share one animation-frame paint. A fresh press
  clears the preceding swipe's click guard, and lost capture cancels instead of
  activating the action.
- g3-ui's refresher owns its visual state in its gesture script. The application's
  `refreshing` flag blocks another pull and exposes `aria-busy`; background
  fetching does not reveal the indicator or translate the content.
- Scroll memory uses layout coordinates, excluding card FLIP, page-transition
  and refresher transforms. It ignores its own scroll corrections, watches only
  card structure in the active scroller, and disables browser scroll anchoring
  so there is one owner of the position.
- A returning cached feed reserves its previous height until its anchor and
  enough content below it have mounted. Grid rearrangement animations are
  skipped while a saved card anchor is being preserved.
- Feed refreshes run outside the page's lifetime. Navigating away while a
  request is pending no longer cancels it and strands the shared busy flag.

### Evidence and repeatable checks

The Android comparison used emulator-5554, Android 15, an x86_64 debug WebView,
and three runs of one horizontal card swipe before and after the change. The
isolated fixture used production g3-ui JavaScript and CSS inside Tawny's WebView;
it did **not** measure the Rust callback or network refresh path. Computed-style
queries fell from 62 per gesture to 2 at press time, with none during movement
or release. All six swipes activated exactly once. Task timings overlapped, so
these short emulator captures do not establish an FPS or end-to-end speedup.

Local traces and results are retained in
`test-results/android-perf-2026-11-06/` (ignored by Git).

Regression commands:

```sh
# g3-ui: trusted touch input in both modes at 390 and 1440 pixels
just swipe-browser-test
just script-test

# Tawny: staged refreshes, deep returns, animated ancestors and grid motion
npx playwright test tests/ui/scroll-memory.spec.cjs tests/ui/video_grid_motion.spec.cjs
```

The staged-refresh test samples every animation frame and allows at most one
pixel of movement of the reader's anchored card. Restoration is tested both
when the anchor is missing initially and when it is present before the rest
of the viewport can fit.

Final validation passed: Tawny's `just check`, `just test` and `just lint-strict`;
g3-ui's `just pre-push`; trusted gesture/refresher browser checks in both modes;
and feed, grid-motion, navigation and refresh-lifetime browser regressions at
phone and desktop sizes. The final debug APK was installed on emulator-5554;
a real pull completed with HTTP 200 and the "Feed refreshed" toast, and tab
navigation returned to Feed without a native crash. Existing Biome warnings
remain outside this change.

### dioxus-motion comparison

The published 0.4.0 implementation drives `use_motion` from a Rust async loop,
updates reactive motion values, and uses Tokio timers on native platforms. A
style derived from those values still goes through Dioxus rendering and its
native edit transport. It does not replace the renderer's synchronous event
bridge or Android runtime lock. It also polls inactive native motion handles
at a 33 ms interval.

This is useful for Rust-driven spring, tween and value animation. It does not
remove the bridge from finger tracking. Tawny's gesture scripts, CSS settling,
and browser animation APIs keep those frames local to the WebView; keep that
division for touch gestures and scroll preservation.

Sources: [motion driver](https://docs.rs/dioxus-motion/0.4.0/src/dioxus_motion/lib.rs.html),
[reactive state updates](https://docs.rs/dioxus-motion/0.4.0/src/dioxus_motion/manager.rs.html),
[native timers](https://docs.rs/dioxus-motion/0.4.0/src/dioxus_motion/animations/platform.rs.html).

Virtualization can reduce work for a very large feed, but it does not repair
competing scroll corrections. If introduced later, its spacer heights and
item identities must preserve the same layout-based card anchor.
