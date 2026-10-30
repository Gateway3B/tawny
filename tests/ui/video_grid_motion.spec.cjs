const { test, expect } = require("@playwright/test");
const path = require("node:path");

test.beforeEach(async ({ page }, testInfo) => {
  if (testInfo.project.name === "mobile-chromium") {
    await page.setViewportSize({ width: 390, height: 844 });
  }
});

async function openGrid(page) {
  await page.setContent(`
    <style>
      .video-grid { display: grid; position: relative; align-content: start;
        grid-template-columns: repeat(auto-fit, minmax(280px, 1fr)); gap: 24px; }
      .video-card-frame { height: 180px; position: relative; }
    </style>
    <div class="video-grid">${Array.from(
      { length: 6 },
      (_, i) => `<article class="video-card-frame" data-id="${i}">${i}</article>`,
    ).join("")}</div>
  `);
  await page.addScriptTag({ path: path.resolve("assets/tawny_video_grid.js") });
}

test("video cards close a removed gap and make room when restored", async ({ page }) => {
  await openGrid(page);
  const positions = await page
    .locator(".video-card-frame")
    .evaluateAll((nodes) => nodes.map((node) => ({ x: node.offsetLeft, y: node.offsetTop })));
  const moving = await page.evaluate(async () => {
    window.removedCard = document.querySelector(".video-card-frame");
    window.removedCard.remove();
    await new Promise(queueMicrotask);
    const next = document.querySelector(".video-card-frame");
    return next.getAnimations()[0]?.effect.getKeyframes()[0].transform;
  });
  expect(moving).toBe(
    `translate(${positions[1].x - positions[0].x}px, ${positions[1].y - positions[0].y}px)`,
  );
  await expect
    .poll(() =>
      page
        .locator(".video-card-frame")
        .first()
        .evaluate((node) => node.getAnimations().length),
    )
    .toBe(0);
  const restoring = await page.evaluate(async () => {
    const next = document.querySelector(".video-card-frame");
    document.querySelector(".video-grid").prepend(window.removedCard);
    await new Promise(queueMicrotask);
    return next.getAnimations().length;
  });
  expect(restoring).toBe(1);
  await expect(page.locator(".video-card-frame")).toHaveCount(6);
});

test("video grid honors reduced motion", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await openGrid(page);
  const animations = await page.evaluate(async () => {
    document.querySelector(".video-card-frame").remove();
    await new Promise(queueMicrotask);
    return document.getAnimations().length;
  });
  expect(animations).toBe(0);
});

test("rapid removals settle without leftover transforms or grid height", async ({ page }) => {
  await openGrid(page);
  for (let i = 0; i < 3; i++) {
    await page
      .locator(".video-card-frame")
      .first()
      .evaluate((node) => node.remove());
    await page.waitForTimeout(40);
  }
  await expect.poll(() => page.evaluate(() => document.getAnimations().length)).toBe(0);
  const settled = await page.locator(".video-grid").evaluate((grid) => ({
    height: grid.getBoundingClientRect().height,
    bottom: Math.max(...[...grid.children].map((node) => node.offsetTop + node.offsetHeight)),
    transforms: [...grid.children].map((node) => getComputedStyle(node).transform),
  }));
  expect(settled.height).toBe(settled.bottom);
  expect(settled.transforms).toEqual(["none", "none", "none"]);
});
