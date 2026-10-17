const { test: base, expect } = require("@playwright/test");

const test = base.extend({
  appPage: async ({ page, baseURL }, use, testInfo) => {
    const pageErrors = [];
    const consoleErrors = [];

    // The browser client intentionally makes a first launch choose its server
    // before it can use the library. Seed the same local server Playwright is
    // exercising before any document code runs, so interaction specs test the
    // app rather than the setup gate. The first-launch flow has its own spec.
    await page.addInitScript((serverUrl) => {
      window.localStorage.setItem("tawny.backend-url", serverUrl);
    }, baseURL);

    page.on("pageerror", (error) => pageErrors.push(error.stack || error.message));
    page.on("console", (message) => {
      if (message.type() === "error") consoleErrors.push(message.text());
    });
    // Keep the normal motion setting. The shared g3 route/modal components
    // resolve state changes from their transition lifecycle, so forcing
    // reduced motion can leave an interaction waiting for an event that no
    // longer fires. Accessibility coverage for reduced motion belongs in a
    // component-level test with an explicit completion assertion.

    await use(page);

    if (consoleErrors.length) {
      await testInfo.attach("browser-console-errors", {
        body: consoleErrors.join("\n"),
        contentType: "text/plain",
      });
    }
    expect(pageErrors, `Uncaught browser errors:\n${pageErrors.join("\n")}`).toEqual([]);
  },
});

async function openApp(page, path = "/") {
  // The full-stack route is server-rendered first, then the browser restores
  // its guest session and viewer state. A visible shell alone is not proof
  // that Dioxus has attached event handlers, so wait for the bootstrap
  // request rather than racing a click against hydration.
  const viewerLoaded = page.waitForResponse(
    (response) =>
      response.status() === 200 &&
      /\/api\/v1\/viewer(?:\?|$)/.test(new URL(response.url()).pathname),
  );
  await page.goto(path, { waitUntil: "domcontentloaded" });
  await expect(page.locator(".g3-app-shell")).toBeVisible();
  await viewerLoaded;
}

async function expectNoHorizontalScroll(page) {
  const sizes = await page.evaluate(() => ({
    viewport: document.documentElement.clientWidth,
    document: document.documentElement.scrollWidth,
    body: document.body.scrollWidth,
  }));
  expect(Math.max(sizes.document, sizes.body)).toBeLessThanOrEqual(sizes.viewport + 1);
}

async function expectResponsiveNavigation(page, labels) {
  const viewport = page.viewportSize();
  const tablist = page.getByRole("navigation", { name: "Primary navigation" });
  await expect(tablist).toBeVisible();

  const tablistBox = await tablist.boundingBox();
  const boxes = [];
  for (const label of labels) {
    const tab = tablist.getByRole("button", { name: label, exact: true });
    await expect(tab).toBeVisible();
    boxes.push(await tab.boundingBox());
  }

  expect(viewport).not.toBeNull();
  expect(tablistBox).not.toBeNull();
  expect(boxes.every(Boolean)).toBe(true);

  if (viewport.width >= 768) {
    // A rail down the left edge. g3-ui rails start expanded, labels beside
    // the icons, and collapse to icons from the toggle at their foot.
    await expect(
      tablist.getByRole("button", { name: "Collapse navigation", exact: true }),
    ).toHaveAttribute("aria-expanded", "true");
    expect(tablistBox.width).toBeLessThan(viewport.width / 4);
    expect(
      Math.max(...boxes.map((box) => box.x)) - Math.min(...boxes.map((box) => box.x)),
    ).toBeLessThan(8);
  } else {
    expect(tablistBox.width).toBeGreaterThan(viewport.width * 0.8);
    expect(
      Math.max(...boxes.map((box) => box.y)) - Math.min(...boxes.map((box) => box.y)),
    ).toBeLessThan(8);
  }
}

module.exports = { test, expect, openApp, expectNoHorizontalScroll, expectResponsiveNavigation };
