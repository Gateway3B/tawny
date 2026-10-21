const { test, expect, openApp, openPlaylistWithVideos } = require("./fixtures/app.cjs");

// Another device is played by the page's own request context: it carries the
// same session cookie, so a write through it is this account changing its
// data somewhere else, which the open page did not see happen.
//
// The checks are triggered with Android's `tawnynativeplaybackresume`, which
// nothing but the sync (and the player's own recovery) listens to, so these
// pass only through it and not through g3-cache's refetch on focus.
const changesChecked = (page) =>
  page.waitForResponse((response) => new URL(response.url()).pathname === "/api/v1/changes");

const resume = (page) =>
  page.evaluate(() => window.dispatchEvent(new Event("tawnynativeplaybackresume")));

test.describe("Changes from another device", () => {
  test("an open playlist shows a video removed elsewhere", async ({ appPage }) => {
    await openPlaylistWithVideos(appPage);
    const cards = appPage.locator(".g3-content-scroll [data-scroll-anchor]");
    await expect(cards).toHaveCount(2);
    // The first check after loading is the baseline. Asked for here so it
    // has certainly landed before anything changes.
    const baseline = changesChecked(appPage);
    await resume(appPage);
    await baseline;

    const response = await appPage.request.post("/api/v1/playlists/membership", {
      data: { playlist_id: "favorites", video_id: "M7lc1UVf-VE", member: false },
    });
    expect(response.ok(), await response.text()).toBe(true);

    await resume(appPage);
    await expect(cards).toHaveCount(1);
  });

  test("a paused video catches up to where another device left it", async ({ appPage }) => {
    // Held open, so the transport never loads a stream; the stand-in below
    // is what is paused.
    await appPage.route("**/api/v1/playback/proxy/**", () => new Promise(() => {}));
    // The first check after loading is the baseline.
    const baseline = changesChecked(appPage);
    await openApp(appPage, "/watch/dQw4w9WgXcQ");
    const media = appPage.locator("#tawny-player-media");
    await expect(media).toBeAttached();
    await media.evaluate((video) => {
      let position = 0;
      Object.defineProperty(video, "paused", { configurable: true, get: () => true });
      Object.defineProperty(video, "readyState", { configurable: true, get: () => 4 });
      Object.defineProperty(video, "currentTime", {
        configurable: true,
        get: () => position,
        set: (value) => {
          position = value;
        },
      });
    });
    await baseline;

    const response = await appPage.request.post("/api/v1/progress", {
      data: {
        progress: [
          { video_id: "dQw4w9WgXcQ", watched: false, progress_seconds: 150, audio_only: false },
        ],
      },
    });
    expect(response.ok(), await response.text()).toBe(true);

    await appPage.evaluate(() => {
      window.__tawnyAttachEval = { phase: "attached", hasMedia: true };
    });
    await resume(appPage);
    await expect.poll(() => media.evaluate((video) => video.currentTime)).toBe(150);
    await expect(appPage.getByText("Picked up at 2:30")).toBeVisible();
  });

  test("a playing video is left where it is", async ({ appPage }) => {
    await appPage.route("**/api/v1/playback/proxy/**", () => new Promise(() => {}));
    // The first check after loading is the baseline.
    const baseline = changesChecked(appPage);
    await openApp(appPage, "/watch/dQw4w9WgXcQ");
    const media = appPage.locator("#tawny-player-media");
    await expect(media).toBeAttached();
    await media.evaluate((video) => {
      Object.defineProperty(video, "paused", { configurable: true, get: () => false });
      Object.defineProperty(video, "readyState", { configurable: true, get: () => 4 });
    });
    await baseline;
    let asked = false;
    appPage.on("request", (request) => {
      if (new URL(request.url()).pathname === "/api/v1/progress/dQw4w9WgXcQ") asked = true;
    });

    await appPage.request.post("/api/v1/progress", {
      data: {
        progress: [
          { video_id: "dQw4w9WgXcQ", watched: false, progress_seconds: 150, audio_only: false },
        ],
      },
    });
    await appPage.evaluate(() => {
      window.__tawnyAttachEval = { phase: "attached", hasMedia: true };
    });
    const checked = changesChecked(appPage);
    await resume(appPage);
    await checked;
    // Long enough for a catch-up to have asked for the position.
    await appPage.waitForTimeout(1000);
    expect(asked).toBe(false);
  });
});

test.describe("Feed scroll", () => {
  test("cards added above the view keep the reader's place", async ({ appPage }) => {
    await openPlaylistWithVideos(appPage);
    const scroller = appPage.locator(".g3-content-scroll").first();
    // Room to scroll, and a stand-in for the second card being the one at the
    // top of the view.
    const before = await scroller.evaluate((element) => {
      const cards = element.querySelectorAll("[data-scroll-anchor]");
      // Chromium's own scroll anchoring would hide the bug: WebKit has none,
      // and a refetch that moves a card to another page's grid defeats it.
      element.style.overflowAnchor = "none";
      const spacer = document.createElement("div");
      spacer.style.height = "3000px";
      element.append(spacer);
      const target = cards[1];
      element.scrollTop =
        target.getBoundingClientRect().top -
        element.getBoundingClientRect().top +
        element.scrollTop;
      element.dispatchEvent(new Event("scroll"));
      return target.getBoundingClientRect().top;
    });
    // A refetch that puts something tall above it.
    await scroller.evaluate((element) => {
      const grid = element.querySelector("[data-scroll-anchor]").parentElement;
      const added = document.createElement("div");
      added.style.height = "400px";
      added.dataset.scrollAnchor = "new-upload";
      grid.prepend(added);
    });
    await expect
      .poll(() =>
        scroller.evaluate(
          (element) =>
            element.querySelectorAll("[data-scroll-anchor]")[2].getBoundingClientRect().top,
        ),
      )
      .toBeCloseTo(before, 0);
  });
});
