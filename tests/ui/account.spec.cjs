const { test, expect, openApp, expectNoHorizontalScroll } = require("./fixtures/app.cjs");

test("settings keeps local account controls without company services", async ({
  appPage,
  baseURL,
}, testInfo) => {
  if (testInfo.project.name === "mobile-chromium") {
    await appPage.setViewportSize({ width: 390, height: 844 });
  }
  const externalRequests = [];
  const serverOrigin = new URL(baseURL).origin;
  appPage.on("request", (request) => {
    if (
      ["fetch", "xhr"].includes(request.resourceType()) &&
      new URL(request.url()).origin !== serverOrigin
    ) {
      externalRequests.push(request.url());
    }
  });
  await openApp(appPage, "/settings");
  await expect(appPage.locator(".g3-card-title").first()).toHaveText("Account");
  await expect(appPage.locator(".g3-card-title").nth(1)).toHaveText("Server");
  await expect(appPage.getByText("Your data", { exact: true })).toHaveCount(0);
  await expect(appPage.getByText(/g3tech/i)).toHaveCount(0);
  await expect(appPage.getByRole("button", { name: "Delete account", exact: true })).toBeEnabled();
  await expectNoHorizontalScroll(appPage);
  expect(externalRequests).toEqual([]);
  await appPage.screenshot({ path: `target/settings-independent-${testInfo.project.name}.png` });
});

test.describe("Tawny first launch", () => {
  test("requires a complete server URL before connecting", async ({ page }) => {
    await page.addInitScript(() => {
      window.localStorage.removeItem("tawny.backend-url");
    });

    await page.goto("/", { waitUntil: "domcontentloaded" });
    await expect(page.getByRole("heading", { name: "Connect to a Tawny server" })).toBeVisible();
    await expect(page.getByText(/g3tech/i)).toHaveCount(0);

    await page.getByRole("textbox", { name: "Server address" }).fill("tawny.example");
    await page.getByRole("button", { name: "Connect", exact: true }).click();
    await expect(
      page.getByText("Enter a full address, including http:// or https://.", { exact: true }),
    ).toBeVisible();
  });
});

test("deleting a test account finishes immediately and starts a new guest", async ({ appPage }) => {
  await openApp(appPage, "/settings");
  const before = await (await appPage.request.get("/api/v1/auth/account")).json();
  await appPage.getByRole("button", { name: "Delete account", exact: true }).click();
  const dialog = appPage.getByRole("dialog");
  await expect(dialog).toContainText("Other devices will be signed out.");
  const deleted = appPage.waitForResponse(
    (response) => new URL(response.url()).pathname === "/api/v1/auth/delete" && response.ok(),
  );
  await dialog.getByRole("button", { name: "Delete account", exact: true }).click();
  await deleted;
  await expect
    .poll(async () => {
      const response = await appPage.request.get("/api/v1/auth/account");
      return response.ok() ? (await response.json()).id : before.id;
    })
    .not.toBe(before.id);
  await expect(appPage.getByRole("button", { name: "Delete account", exact: true })).toBeEnabled();
});
