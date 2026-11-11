const { test, expect, openApp } = require("./fixtures/app.cjs");

async function sideButton(page, button) {
  const input = await page.context().newCDPSession(page);
  const name = button === 3 ? "back" : "forward";
  try {
    await input.send("Input.dispatchMouseEvent", {
      type: "mousePressed",
      button: name,
      buttons: button === 3 ? 8 : 16,
      x: 180,
      y: 250,
      clickCount: 1,
    });
    await input.send("Input.dispatchMouseEvent", {
      type: "mouseReleased",
      button: name,
      buttons: 0,
      x: 180,
      y: 250,
      clickCount: 1,
    });
  } finally {
    await input.detach();
  }
}

test("mouse Back and Forward traverse the same page history without duplicate steps", async ({
  appPage,
}) => {
  await openApp(appPage);
  await appPage
    .getByRole("navigation", { name: "Primary navigation" })
    .getByRole("button", { name: "Playlists", exact: true })
    .click();
  await expect(appPage).toHaveURL(/\/playlists$/);
  await appPage.getByRole("button", { name: "Settings", exact: true }).click();
  await expect(appPage).toHaveURL(/\/settings$/);
  await sideButton(appPage, 3);
  await expect(appPage).toHaveURL(/\/playlists$/);
  await sideButton(appPage, 4);
  await expect(appPage).toHaveURL(/\/settings$/);
  await sideButton(appPage, 3);
  await expect(appPage).toHaveURL(/\/playlists$/);
  await sideButton(appPage, 3);
  await expect(appPage).toHaveURL(/\/$/);
  await sideButton(appPage, 4);
  await expect(appPage).toHaveURL(/\/playlists$/);
  await appPage.screenshot({ path: "target/mouse-navigation-history.png" });
});
