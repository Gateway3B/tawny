const { test, expect, openApp } = require("./fixtures/app.cjs");
const fs = require("node:fs");

const thumbnail = `data:image/svg+xml;base64,${fs.readFileSync("assets/icon/icon.svg").toString("base64")}`;
const video = (index) => ({
  id: `feed-${String(index).padStart(6, "0")}`,
  title: `Feed video ${index + 1}`,
  channel_id: "test-channel",
  channel_name: "Test channel",
  thumbnail_url: thumbnail,
  published_at: "2026-11-01T12:00:00Z",
  duration_seconds: 600,
  view_count: "100 views",
  progress_seconds: 0,
  watched: false,
  is_live: false,
});

async function expectContinuousRows(page, columns) {
  const cards = await page.locator(".video-grid .video-card-frame").evaluateAll((nodes) =>
    nodes.map((node) => {
      const rect = node.getBoundingClientRect();
      return { top: rect.top, left: rect.left, width: rect.width, height: rect.height };
    }),
  );
  expect(cards).toHaveLength(48);
  for (let index = 1; index < cards.length; index++) {
    expect(cards[index].height).toBeGreaterThan(100);
    if (index % columns !== 0) {
      expect(
        Math.abs(cards[index].top - cards[index - 1].top),
        `blank slot before video ${index + 1}`,
      ).toBeLessThan(1);
      expect(cards[index].left).toBeGreaterThan(cards[index - 1].left);
    } else {
      expect(cards[index].top).toBeGreaterThan(cards[index - 1].top);
      expect(Math.abs(cards[index].left - cards[0].left)).toBeLessThan(1);
    }
  }
}

test("feed batches fill one grid without blank slots at page boundaries", async ({
  appPage,
}, testInfo) => {
  const wide = testInfo.project.name === "desktop-chromium";
  await appPage.setViewportSize(wide ? { width: 1920, height: 1080 } : { width: 390, height: 844 });
  await appPage.route(/\/api\/v1\/feed(?:\?|$)/, async (route) => {
    const page = Number(route.request().postDataJSON().page);
    await route.fulfill({
      json: {
        videos: page < 2 ? Array.from({ length: 24 }, (_, index) => video(page * 24 + index)) : [],
        has_more: page === 0,
        without_duration: 0,
        unknown_durations: [],
      },
    });
  });
  await openApp(appPage);
  await expect
    .poll(() => appPage.locator(".video-grid .video-card-frame").count())
    .toBeGreaterThanOrEqual(24);
  await appPage.locator(".g3-content-scroll").evaluate((node) => {
    node.scrollTop = node.scrollHeight;
  });
  await expect(appPage.locator(".video-grid .video-card-frame")).toHaveCount(48);
  await expect
    .poll(() =>
      appPage
        .locator(".video-grid")
        .evaluateAll((nodes) =>
          nodes.reduce((count, node) => count + node.getAnimations({ subtree: true }).length, 0),
        ),
    )
    .toBe(0);
  await expect
    .poll(() =>
      appPage
        .locator(".video-grid")
        .first()
        .evaluate((node) => getComputedStyle(node).gridTemplateColumns.split(" ").length),
    )
    .toBe(wide ? 5 : 1);
  await expectContinuousRows(appPage, wide ? 5 : 1);
  await appPage
    .locator('[data-scroll-anchor="feed-000020"]')
    .evaluate((node) => node.scrollIntoView({ block: "start" }));
  await appPage.screenshot({ path: `target/feed-continuous-${testInfo.project.name}.png` });
  await appPage.getByRole("button", { name: "Settings", exact: true }).click();
  await appPage.getByRole("button", { name: "Back", exact: true }).click();
  await expect(appPage.locator(".video-grid .video-card-frame")).toHaveCount(48);
  await expect(appPage.locator(".video-grid .video-card-frame").first()).toBeVisible();
  await expectContinuousRows(appPage, wide ? 5 : 1);
  if (wide) {
    await appPage.setViewportSize({ width: 1440, height: 900 });
    await expectContinuousRows(appPage, 3);
    await appPage.screenshot({ path: "target/feed-continuous-1440.png" });
  }
});
