const {
  test,
  expect,
  openApp,
  expectNoHorizontalScroll,
  expectResponsiveNavigation,
} = require("./fixtures/app.cjs");

const primaryTabs = ["Feed", "Playlists", "Search", "Subscriptions"];

test.describe("Tawny application shell", () => {
  test.beforeEach(async ({ appPage }) => {
    await openApp(appPage);
  });

  // A tab's root page has no title in its bar; the section name beside the
  // brand mark is its one heading, rather than an empty one.
  test("each tab's root page has one heading naming it", async ({ appPage }) => {
    for (const [path, name] of [
      ["/", "Feed"],
      ["/playlists", "Playlists"],
      ["/explore", "Search"],
      ["/subscriptions", "Subscriptions"],
    ]) {
      await openApp(appPage, path);
      const headings = appPage.locator("h1");
      await expect(headings).toHaveCount(1);
      await expect(headings).toHaveText(name, { ignoreCase: true });
    }
  });

  test("adapts primary navigation to the viewport", async ({ appPage }) => {
    await expectResponsiveNavigation(appPage, primaryTabs);
    await expectNoHorizontalScroll(appPage);
    await expect(
      appPage
        .getByRole("navigation", { name: "Primary navigation" })
        .getByRole("button", { name: "Feed", exact: true }),
    ).toHaveAttribute("aria-current", "page");
  });

  test("navigates every primary destination and preserves selected state", async ({ appPage }) => {
    const destinations = [
      ["Playlists", /\/playlists$/],
      ["Search", /\/explore$/],
      ["Subscriptions", /\/subscriptions$/],
      ["Feed", /\/$/],
    ];

    for (const [label, route] of destinations) {
      const tab = appPage
        .getByRole("navigation", { name: "Primary navigation" })
        .getByRole("button", { name: label, exact: true });
      await tab.click();
      await expect(appPage).toHaveURL(route);
      await expect(tab).toHaveAttribute("aria-current", "page");
      await expectNoHorizontalScroll(appPage);
    }
  });

  test("opens an auxiliary screen and returns to the active section", async ({ appPage }) => {
    await appPage.getByRole("button", { name: "Settings", exact: true }).click();
    await expect(appPage).toHaveURL(/\/settings$/);
    await expect(appPage.getByText("Video speed", { exact: true })).toBeVisible();
    await expect(appPage.getByText("Duration filters", { exact: true })).toBeVisible();
    await expectNoHorizontalScroll(appPage);

    await appPage.getByRole("button", { name: "Back", exact: true }).click();
    await expect(appPage).toHaveURL(/\/$/);
  });
  test("tapping Feed on the feed goes back to the top", async ({ appPage }, testInfo) => {
    const wide = testInfo.project.name === "desktop-chromium";
    const scroller = appPage.locator(".g3-content-scroll").first();
    // A fresh account's feed may be too short to scroll.
    await scroller.evaluate((element) => {
      const spacer = document.createElement("div");
      spacer.style.height = "4000px";
      element.appendChild(spacer);
      element.scrollTop = 1500;
    });
    expect(await scroller.evaluate((element) => element.scrollTop)).toBeGreaterThan(0);

    // A phone pulls down to refresh, so only the wide layout refreshes here.
    let refreshed = false;
    appPage.on("request", (request) => {
      if (new URL(request.url()).pathname === "/api/v1/feed/refresh") refreshed = true;
    });
    await appPage
      .getByRole("navigation", { name: "Primary navigation" })
      .getByRole("button", { name: "Feed", exact: true })
      .click();
    await expect.poll(() => scroller.evaluate((element) => element.scrollTop)).toBe(0);
    await appPage.waitForTimeout(300);
    expect(refreshed).toBe(wide);
    await expect(appPage).toHaveURL(/\/$/);
  });
});
