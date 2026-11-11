const { test, expect } = require("@playwright/test");
const fs = require("node:fs");
const path = require("node:path");
const memory = fs.readFileSync(path.resolve("assets/tawny_scroll_memory.js"), "utf8");

async function openFeed(page) {
  if (page.viewportSize().width < 768) await page.setViewportSize({ width: 390, height: 844 });
  await page.setContent(`<style>
    .g3-content-scroll { height:600px; overflow:auto; overflow-anchor:none; position:relative }
    .video-grid { position:relative; display:grid; gap:16px }
    .video-card-frame { height:180px }
  </style><div class="g3-content-scroll"><div class="g3-refresher"><div class="video-grid"></div></div></div>`);
  await page.evaluate(() => {
    window.card = (id) => {
      const node = document.createElement("article");
      node.className = "video-card-frame";
      node.dataset.scrollAnchor = String(id);
      node.textContent = `Video ${id}`;
      return node;
    };
    document
      .querySelector(".video-grid")
      .append(...Array.from({ length: 36 }, (_, i) => window.card(i)));
  });
  await page.addScriptTag({ content: memory.replace("__PATH__", '"/"') });
  await page.addScriptTag({ path: path.resolve("assets/tawny_video_grid.js") });
  await page.locator(".g3-content-scroll").evaluate((element) => {
    element.scrollTop = 1960;
  });
  await expect(page.locator(".g3-content-scroll")).toHaveAttribute("data-scroll-preserving", "");
}

test("staged refreshes preserve the visible card throughout every animation frame", async ({
  page,
}) => {
  await openFeed(page);
  const before = await page
    .locator('[data-scroll-anchor="10"]')
    .evaluate((node) => node.getBoundingClientRect().top);
  await page.evaluate(async () => {
    window.offsets = [];
    let running = true;
    const sample = () => {
      window.offsets.push(
        document.querySelector('[data-scroll-anchor="10"]').getBoundingClientRect().top,
      );
      if (running) requestAnimationFrame(sample);
    };
    requestAnimationFrame(sample);
    for (let i = 0; i < 5; i++) {
      document.querySelector(".video-grid").prepend(window.card(`new-${i}`));
      // Independent cached pages return on different frames.
      await new Promise((resolve) => setTimeout(resolve, 50));
    }
    await new Promise((resolve) => setTimeout(resolve, 300));
    running = false;
  });
  const offsets = await page.evaluate(() => window.offsets);
  expect(offsets.length).toBeGreaterThan(10);
  expect(Math.max(...offsets.map((offset) => Math.abs(offset - before)))).toBeLessThanOrEqual(1);
  expect(
    await page
      .locator(".video-grid")
      .evaluate((grid) => grid.getAnimations({ subtree: true }).length),
  ).toBe(0);
});

for (const initialCount of [4, 12])
  test(`returning to a feed waits for ${initialCount} initial cards to fill the saved viewport`, async ({
    page,
  }) => {
    await openFeed(page);
    const before = await page
      .locator('[data-scroll-anchor="10"]')
      .evaluate((node) => node.getBoundingClientRect().top);
    const result = await page.evaluate(async (initialCount) => {
      const api = window[Symbol.for("tawny.scroll-memory")];
      api.arrive("/watch/example", false);
      const old = document.querySelector(".g3-refresher");
      window.returningContent = old.cloneNode(false);
      window.returningContent.innerHTML = '<div class="video-grid"></div>';
      old.replaceWith(window.returningContent);
      document
        .querySelector(".video-grid")
        .append(...Array.from({ length: initialCount }, (_, i) => window.card(i)));
      api.arrive("/", true);
      await new Promise((resolve) => setTimeout(resolve, 400));
      const pendingTop = document.querySelector(".g3-content-scroll").scrollTop;
      document
        .querySelector(".video-grid")
        .append(
          ...Array.from({ length: 36 - initialCount }, (_, i) => window.card(i + initialCount)),
        );
      await new Promise((resolve) => setTimeout(resolve, 250));
      return {
        pendingTop,
        finalTop: document.querySelector(".g3-content-scroll").scrollTop,
        minHeight: window.returningContent.style.minHeight,
      };
    }, initialCount);
    expect(result.pendingTop).toBe(1960);
    expect(result.finalTop).toBe(1960);
    expect(result.minHeight).toBe("");
    expect(
      await page
        .locator('[data-scroll-anchor="10"]')
        .evaluate((node) => node.getBoundingClientRect().top),
    ).toBe(before);
  });

test("scroll anchors ignore an ancestor's temporary animation transform", async ({ page }) => {
  await openFeed(page);
  await page.evaluate(async () => {
    document.querySelector(".g3-refresher").style.transform = "translateY(48px)";
    document.querySelector(".video-grid").prepend(window.card("new"));
    await new Promise(requestAnimationFrame);
  });
  expect(await page.locator(".g3-content-scroll").evaluate((element) => element.scrollTop)).toBe(
    2156,
  );
});

test("refresh notices above the grid preserve the same card when they appear and disappear", async ({
  page,
}) => {
  await openFeed(page);
  const before = await page
    .locator('[data-scroll-anchor="10"]')
    .evaluate((node) => node.getBoundingClientRect().top);
  for (const show of [true, false]) {
    await page.evaluate(async (show) => {
      if (show) {
        const notice = document.createElement("p");
        notice.id = "duration-notice";
        notice.style.height = "60px";
        notice.textContent = "Some video durations are still being filled in.";
        document.querySelector(".g3-refresher").prepend(notice);
      } else document.getElementById("duration-notice").remove();
      await new Promise(requestAnimationFrame);
    }, show);
    expect(
      await page
        .locator('[data-scroll-anchor="10"]')
        .evaluate((node) => node.getBoundingClientRect().top),
    ).toBe(before);
  }
});
