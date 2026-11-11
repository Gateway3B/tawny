const { test, expect, openApp } = require("./fixtures/app.cjs");

test("a refresh completes after leaving Feed and does not leave the next visit busy", async ({
  appPage,
}) => {
  let release;
  const completed = new Promise((resolve) => {
    release = resolve;
  });
  await appPage.route("**/api/v1/feed/refresh", async (route) => {
    await completed;
    await route.fulfill({
      json: { imported: 3, refreshed_channels: 1, failed_channels: 0, sources: ["RSS"] },
    });
  });
  await openApp(appPage);
  const feedRefresherId = await appPage.locator(".g3-refresher").getAttribute("id");
  const refreshStarted = appPage.waitForRequest(
    (request) => new URL(request.url()).pathname === "/api/v1/feed/refresh",
  );
  await appPage.evaluate(() => {
    const root = document.querySelector(".g3-refresher");
    root.dispatchEvent(
      new PointerEvent("pointerdown", {
        pointerType: "mouse",
        button: 0,
        clientX: 180,
        clientY: 250,
      }),
    );
    window.dispatchEvent(
      new PointerEvent("pointermove", { pointerType: "mouse", clientX: 180, clientY: 370 }),
    );
    window.dispatchEvent(new PointerEvent("pointerup", { pointerType: "mouse" }));
  });
  await refreshStarted;
  await appPage
    .getByRole("navigation", { name: "Primary navigation" })
    .getByRole("button", { name: "Playlists", exact: true })
    .click();
  await expect(appPage).toHaveURL(/\/playlists$/);
  await expect(appPage.locator(`#${feedRefresherId}`)).toHaveCount(0);
  release();
  await expect(appPage.getByText("New videos added", { exact: true })).toBeVisible();
  await expect(appPage.getByText(/Feed refreshed|RSS/)).toHaveCount(0);
  await appPage
    .getByRole("navigation", { name: "Primary navigation" })
    .getByRole("button", { name: "Feed", exact: true })
    .click();
  await expect(appPage).toHaveURL(/\/$/);
  await expect(appPage.locator(".g3-refresher")).toHaveAttribute("data-refreshing", "false");
});
