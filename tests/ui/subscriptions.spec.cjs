const { test, expect, openApp } = require("./fixtures/app.cjs");

const channel = {
  id: "UC-tawny-unsubscribe",
  name: "Unsubscribe test channel",
  handle: "@unsubscribe-test",
  avatar_url: null,
  subscriber_count: "100",
  subscribed: true,
  description: "",
  banner_url: null,
  subscription_content: "all",
};
async function subscribe(page) {
  const response = await page.request.post("/api/v1/subscriptions", {
    data: { changes: [{ channel, subscribed: true, content: "all" }] },
  });
  expect(response.ok(), await response.text()).toBe(true);
}
const card = (page) => page.locator(".g3-card", { hasText: channel.name });

test.beforeEach(async ({ appPage }, testInfo) => {
  if (testInfo.project.name === "mobile-chromium")
    await appPage.setViewportSize({ width: 390, height: 844 });
});

test("unsubscribe survives its card and page disappearing and confirms only after saving", async ({
  appPage,
}) => {
  await openApp(appPage);
  await subscribe(appPage);
  await openApp(appPage, "/subscriptions");
  await expect(card(appPage)).toBeVisible();
  let release;
  const pending = new Promise((resolve) => {
    release = resolve;
  });
  await appPage.route("**/api/v1/subscriptions", async (route) => {
    await pending;
    await route.continue();
  });
  const started = appPage.waitForRequest(
    (request) => new URL(request.url()).pathname === "/api/v1/subscriptions",
  );
  await card(appPage).getByRole("button", { name: "Subscribed", exact: true }).click();
  await started;
  await expect(card(appPage)).toHaveCount(0);
  await expect(appPage.getByText("Unsubscribed", { exact: true })).toHaveCount(0);
  await appPage
    .getByRole("navigation", { name: "Primary navigation" })
    .getByRole("button", { name: "Feed", exact: true })
    .click();
  const saved = appPage.waitForResponse(
    (response) => new URL(response.url()).pathname === "/api/v1/subscriptions" && response.ok(),
  );
  release();
  await saved;
  await expect(appPage.getByText("Unsubscribed", { exact: true })).toBeVisible();
  const viewer = await (await appPage.request.get("/api/v1/viewer")).json();
  expect(viewer.subscriptions.some((found) => found.id === channel.id)).toBe(false);
  await openApp(appPage, "/subscriptions");
  await expect(card(appPage)).toHaveCount(0);
  await appPage.reload();
  await expect(appPage.locator(".g3-app-shell")).toBeVisible();
  await expect(card(appPage)).toHaveCount(0);
});

test("a refused unsubscribe restores the subscription without reporting success", async ({
  appPage,
}, testInfo) => {
  await openApp(appPage);
  await subscribe(appPage);
  await openApp(appPage, "/subscriptions");
  await expect(card(appPage)).toBeVisible();
  await appPage.route("**/api/v1/subscriptions", (route) =>
    route.fulfill({ status: 500, body: "Save failed" }),
  );
  await card(appPage).getByRole("button", { name: "Subscribed", exact: true }).click();
  await expect(
    appPage.getByText("Couldn't unsubscribe. Please try again.", { exact: true }),
  ).toBeVisible();
  await expect(card(appPage)).toBeVisible();
  await expect(appPage.getByText("Unsubscribed", { exact: true })).toHaveCount(0);
  await appPage.screenshot({ path: `target/unsubscribe-failure-${testInfo.project.name}.png` });
});

test("channel-page unsubscribe can be retried after failure and stays saved", async ({
  appPage,
}, testInfo) => {
  await openApp(appPage);
  await subscribe(appPage);
  const media = (tab) => ({
    channel_id: channel.id,
    tab,
    videos: [],
    next_page: null,
    source: "saved",
  });
  await appPage.route(`**/api/v1/channels/${channel.id}?*`, (route) =>
    route.fulfill({
      json: {
        channel,
        videos: media("Videos"),
        shorts: media("Shorts"),
        live: media("Live"),
        remote_available: true,
      },
    }),
  );
  await openApp(appPage, `/channel/${channel.id}`);
  let failed = true;
  await appPage.route("**/api/v1/subscriptions", (route) =>
    failed ? route.fulfill({ status: 500, body: "Save failed" }) : route.continue(),
  );
  await appPage.getByRole("button", { name: "Subscribed", exact: true }).click();
  await expect(
    appPage.getByText("Couldn't unsubscribe. Please try again.", { exact: true }),
  ).toBeVisible();
  await expect(appPage.getByRole("button", { name: "Subscribed", exact: true })).toBeVisible();
  await expect(appPage.getByText("Unsubscribed", { exact: true })).toHaveCount(0);
  failed = false;
  const saved = appPage.waitForResponse(
    (response) => new URL(response.url()).pathname === "/api/v1/subscriptions" && response.ok(),
  );
  await appPage.getByRole("button", { name: "Subscribed", exact: true }).click();
  await saved;
  await expect(appPage.getByRole("button", { name: "Subscribe", exact: true })).toBeVisible();
  await expect(appPage.getByText("Unsubscribed", { exact: true })).toBeVisible();
  const viewer = await (await appPage.request.get("/api/v1/viewer")).json();
  expect(viewer.subscriptions.some((found) => found.id === channel.id)).toBe(false);
  await appPage.screenshot({ path: `target/unsubscribe-success-${testInfo.project.name}.png` });
});
