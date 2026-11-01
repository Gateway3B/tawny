const { test, expect, openPlaylistWithVideos } = require("./fixtures/app.cjs");

test("player controls update without replacing media when minimized and reopened", async ({
  appPage,
}, testInfo) => {
  if (testInfo.project.name === "mobile-chromium") {
    await appPage.setViewportSize({ width: 390, height: 844 });
  }
  // This exercises component lifetime, independently of YouTube availability.
  await appPage.route("**/api/v1/playback/*?*", (route) =>
    route.fulfill({
      json: {
        primary: {
          protocol: "Progressive",
          url: "/__player_lifetime_test.mp4",
          mime_type: "video/mp4",
          po_token: null,
          expires_at: null,
        },
        fallback_url: "/__unused_embed_fallback",
      },
    }),
  );
  await appPage.route("**/__player_lifetime_test.mp4", () => new Promise(() => {}));
  await openPlaylistWithVideos(appPage);
  await appPage.locator(".video-card-frame .g3-card").first().click();
  await expect(appPage).toHaveURL(/\/watch\//);

  const media = appPage.locator("#tawny-player-media");
  await expect(media).toBeVisible();
  const original = await media.elementHandle();
  const autoplay = appPage.locator(".player-autoplay-button");
  const before = await autoplay.getAttribute("aria-pressed");
  await autoplay.dispatchEvent("click");
  await expect(autoplay).toHaveAttribute("aria-pressed", String(before !== "true"));

  // The bridge belongs to the extracted controls, while the media belongs to
  // the persistent parent. Its handler dismisses the watch sheet.
  await appPage.locator("[data-player-minimize]").dispatchEvent("click");
  await expect(appPage.locator(".persistent-player")).toHaveClass(/\bmini\b/);
  expect(
    await original.evaluate((video) => video === document.querySelector("#tawny-player-media")),
  ).toBe(true);

  await appPage.locator(".mini-player-copy").click();
  await expect(appPage.locator(".persistent-player")).toHaveClass(/\bexpanded\b/);
  await expect(autoplay).toHaveAttribute("aria-pressed", String(before !== "true"));
  expect(
    await original.evaluate((video) => video === document.querySelector("#tawny-player-media")),
  ).toBe(true);
  await expect(appPage.getByRole("group", { name: "Video controls", exact: true })).toHaveCount(1);
});
