const { test, expect, openApp, expectNoHorizontalScroll } = require("./fixtures/app.cjs");

test.describe("Tawny library interactions", () => {
  test("validates and creates a local playlist", async ({ appPage }) => {
    await openApp(appPage, "/playlists");
    await appPage.getByRole("button", { name: "New playlist", exact: true }).first().click();

    const dialog = appPage.getByRole("dialog");
    const create = dialog.getByRole("button", { name: "Create", exact: true });
    await expect(dialog.getByRole("heading", { name: "New playlist", exact: true })).toBeVisible();
    await expect(create).toBeDisabled();

    await dialog.getByLabel("Playlist name", { exact: true }).fill("UI test playlist");
    await expect(create).toBeEnabled();
    await create.click();

    await expect(dialog).toBeHidden();
    const card = appPage.locator(".g3-card", { hasText: "UI test playlist" });
    await expect(card.locator(".g3-card-subtitle")).toHaveText("0 videos");

    await appPage.getByText("UI test playlist", { exact: true }).click();
    await expect(
      appPage.locator("header").getByText("UI test playlist", { exact: true }),
    ).toBeVisible();
    await expect(appPage.getByText("Add videos from the buttons on a card.")).toBeVisible();
    await expect(appPage.getByRole("button", { name: "Play all", exact: true })).toBeDisabled();
    await expectNoHorizontalScroll(appPage);
  });

  test("renames and reorders a playlist from its card menu", async ({ appPage }) => {
    await openApp(appPage, "/playlists");
    const names = () => appPage.locator(".g3-card-title").allInnerTexts();
    // The cards follow the viewer request a moment later.
    await expect(appPage.locator(".g3-card-title").nth(1)).toBeVisible();
    const before = await names();
    expect(before.length).toBeGreaterThan(1);
    const first = before[0];

    // Delete is on the cover, top right, and the dots open the menu.
    const card = appPage.locator(".g3-card", { hasText: first }).first();
    await expect(card.getByRole("button", { name: `Delete ${first}` })).toBeVisible();
    await card.getByRole("button", { name: `Actions for ${first}` }).click();
    const menu = appPage.getByRole("dialog").or(appPage.locator(".g3-sheet")).last();
    await menu.getByText("Move down", { exact: true }).click();
    await expect.poll(async () => (await names())[1]).toBe(first);

    await appPage
      .locator(".g3-card", { hasText: first })
      .first()
      .getByRole("button", { name: `Actions for ${first}` })
      .click();
    await appPage.getByText("Rename", { exact: true }).last().click();
    const dialog = appPage.getByRole("dialog");
    await dialog.getByLabel("Playlist name", { exact: true }).fill("Renamed by test");
    await dialog.getByRole("button", { name: "Rename", exact: true }).click();
    await expect(appPage.getByText("Renamed by test", { exact: true })).toBeVisible();
    // The closed actions sheet stays mounted, hidden, under the old name.
    await expect.poll(names).not.toContain(first);
  });

  test("starts with useful duration defaults and a differentiated light theme", async ({
    appPage,
  }) => {
    await openApp(appPage, "/settings");

    // Style and theme both follow the device until the viewer picks.
    const style = appPage.getByRole("radiogroup", { name: "Style" });
    const theme = appPage.getByRole("radiogroup", { name: "Theme" });
    await expect(style.getByRole("radio", { name: "Auto", exact: true })).toBeChecked();
    await expect(theme.getByRole("radio", { name: "Auto", exact: true })).toBeChecked();
    await expect
      .poll(() =>
        appPage.locator(".g3-app-shell").evaluate((node) => getComputedStyle(node).colorScheme),
      )
      .toBe("light dark");

    await expect(appPage.getByLabel("Video · Short max (minutes)")).toHaveValue("10");
    await expect(appPage.getByLabel("Video · Medium max (minutes)")).toHaveValue("35");

    await appPage.getByRole("radio", { name: "Light", exact: true }).click();
    await expect
      .poll(() =>
        appPage
          .locator(".g3-app-shell")
          .evaluate((element) =>
            getComputedStyle(element).getPropertyValue("--g3-color-bg").trim(),
          ),
      )
      .toBe("#d8c5a8");

    const themeLayers = await appPage.locator(".g3-app-shell").evaluate((element) => {
      const styles = getComputedStyle(element);
      return [
        "--g3-color-bg",
        "--g3-color-bg-secondary",
        "--g3-color-card",
        "--g3-color-surface",
        "--g3-color-control",
      ].map((token) => styles.getPropertyValue(token).trim());
    });
    expect(new Set(themeLayers).size).toBe(themeLayers.length);
    expect(themeLayers).not.toContain("#ffffff");

    const sponsorCard = appPage.locator(".g3-card").filter({ hasText: "SponsorBlock" }).first();
    const surfaces = await sponsorCard.evaluate((card) => ({
      card: getComputedStyle(card).backgroundColor,
      item: getComputedStyle(card.querySelector(".g3-list .g3-item")).backgroundColor,
    }));
    expect(surfaces.item).not.toBe(surfaces.card);
    await expectNoHorizontalScroll(appPage);
  });

  // Real touch input, not a mouse: a finger's pointer starts out captured by
  // the element it pressed, and the row taking capture over from it once used
  // to end the gesture, so the card never moved.
  test("a finger swipes a card and runs its action", async ({ appPage }, testInfo) => {
    test.skip(testInfo.project.name !== "mobile-chromium", "touch behavior");
    await openApp(appPage, "/playlists/deep-dives");

    const row = appPage.locator(".video-card-swipe").first();
    await expect(row).toBeVisible();
    // The gesture script starts once a row mounts.
    await appPage.waitForFunction(() => window[Symbol.for("g3-ui.swipe")]);
    const box = await row.boundingBox();
    const cdp = await appPage.context().newCDPSession(appPage);
    const touch = (type, x, y) =>
      cdp.send("Input.dispatchTouchEvent", {
        type,
        touchPoints: type === "touchEnd" ? [] : [{ x, y, id: 1 }],
      });
    const x = box.x + box.width * 0.3;
    const y = box.y + 80;
    const offset = () =>
      row.evaluate(
        (element) => parseFloat(element.style.getPropertyValue("--g3-swipe-offset")) || 0,
      );

    await touch("touchStart", x, y);
    for (let step = 1; step <= 16; step += 1) {
      await touch("touchMove", x + step * 10, y);
    }
    await expect.poll(offset).toBeGreaterThan(100);
    await touch("touchEnd", 0, 0);

    await expect(appPage.locator(".g3-toast")).toHaveAttribute("data-state", "open");
    await expect.poll(offset).toBe(0);
  });

  test("replaces a desktop action toast and disables desktop card swipes", async ({
    appPage,
  }, testInfo) => {
    test.skip(testInfo.project.name !== "desktop-chromium", "desktop pointer behavior");
    // A fresh account's feed can be empty; every account starts with this.
    await openApp(appPage, "/playlists/deep-dives");

    // A mouse reaches the swipe actions through the thumbnail's edge buttons,
    // which stand in for g3-ui's "Show actions" button on a fine pointer.
    const frame = appPage.locator(".video-card-frame").first();
    const row = frame.locator(".g3-swipe-item");
    const action = frame.locator(".video-card-edge-action-end");
    // Computed, not inline: the gesture script writes the offset only once a
    // row moves, and the stylesheet supplies the resting 0px.
    const offset = () =>
      row.evaluate((element) =>
        getComputedStyle(element).getPropertyValue("--g3-swipe-offset").trim(),
      );
    await expect.poll(offset).toBe("0px");

    await action.click();
    const toast = appPage.locator(".g3-toast");
    await expect(toast).toHaveAttribute("data-state", "open");
    const firstToast = await toast.elementHandle();

    await action.click();
    await expect.poll(() => firstToast.evaluate((element) => element.isConnected)).toBe(false);

    const box = await row.boundingBox();
    await appPage.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    await appPage.mouse.down();
    await appPage.mouse.move(box.x + box.width / 2 + 160, box.y + box.height / 2);
    await appPage.mouse.up();
    await expect.poll(offset).toBe("0px");
  });

  test("prevents native dragging of desktop card thumbnails", async ({ appPage }, testInfo) => {
    test.skip(testInfo.project.name !== "desktop-chromium", "desktop pointer behavior");
    // A fresh account's feed can be empty; every account starts with this.
    await openApp(appPage, "/playlists/deep-dives");

    const thumbnails = appPage.locator(".g3-card .g3-card-media img");
    const thumbnail = thumbnails.first();
    await expect(thumbnail).toBeVisible();
    await expect
      .poll(() => thumbnail.evaluate((image) => getComputedStyle(image).webkitUserDrag))
      .toBe("none");

    await thumbnail.evaluate((image) => {
      image.dataset.dragStarted = "false";
      image.addEventListener("dragstart", () => {
        image.dataset.dragStarted = "true";
      });
    });
    const box = await thumbnail.boundingBox();
    await appPage.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    await appPage.mouse.down();
    await appPage.mouse.move(box.x + box.width / 2 + 120, box.y + box.height / 2, { steps: 8 });
    await appPage.mouse.up();
    await expect(thumbnail).toHaveAttribute("data-drag-started", "false");
  });

  test("repeated player edge clicks accumulate ten-second seeks", async ({ appPage }, testInfo) => {
    test.skip(testInfo.project.name !== "desktop-chromium", "desktop pointer behavior");
    // Straight to a watch page, as the media-session spec does, with the
    // stream held open: the test sets the time itself, and a stream that
    // loaded part way through would move it.
    await appPage.route("**/api/v1/playback/proxy/**", () => new Promise(() => {}));
    await openApp(appPage, "/watch/dQw4w9WgXcQ");

    const player = appPage.locator("#tawny-player");
    const media = appPage.locator("#tawny-player-media");
    await expect(player).toBeVisible();
    await expect(media).toBeVisible();
    await expect(appPage.getByRole("button", { name: "PiP", exact: true })).toHaveClass(
      /g3-btn-sm/,
    );

    await media.evaluate((video) => {
      // Keep this deterministic without replacing the actual player DOM: a
      // remote media response is irrelevant to testing its input wiring.
      Object.defineProperty(video, "duration", { configurable: true, value: 120 });
      Object.defineProperty(video, "currentTime", {
        configurable: true,
        writable: true,
        value: 60,
      });
      window.TawnyPlayerControls.attach(video);
    });

    const box = await player.boundingBox();
    await appPage.mouse.dblclick(box.x + 50, box.y + box.height / 2);
    await expect.poll(() => media.evaluate((video) => video.currentTime)).toBe(50);

    await appPage.waitForTimeout(600);
    await appPage.mouse.dblclick(box.x + box.width - 50, box.y + box.height / 2);
    await expect.poll(() => media.evaluate((video) => video.currentTime)).toBe(60);

    // Some embedded browsers emit two ordinary clicks without incrementing
    // click.detail or producing dblclick. Pointer presses must still seek.
    await appPage.waitForTimeout(600);
    await appPage.evaluate(
      ({ x, y }) => {
        const target = document.elementFromPoint(x, y);
        for (let index = 0; index < 2; index += 1) {
          target.dispatchEvent(
            new PointerEvent("pointerdown", {
              bubbles: true,
              clientX: x,
              clientY: y,
              pointerId: 42,
              pointerType: "touch",
            }),
          );
          target.dispatchEvent(
            new PointerEvent("pointerup", {
              bubbles: true,
              clientX: x,
              clientY: y,
              pointerId: 42,
              pointerType: "touch",
            }),
          );
          target.dispatchEvent(
            new MouseEvent("click", {
              bubbles: true,
              clientX: x,
              clientY: y,
              detail: 1,
            }),
          );
        }
      },
      { x: box.x + 50, y: box.y + box.height / 2 },
    );
    await expect.poll(() => media.evaluate((video) => video.currentTime)).toBe(50);

    const clickEdge = async (x, count) => {
      await appPage.waitForTimeout(600);
      for (let index = 0; index < count; index += 1) {
        await appPage.mouse.click(x, box.y + box.height / 2);
        await appPage.waitForTimeout(60);
      }
    };

    await clickEdge(box.x + box.width - 50, 3);
    await expect.poll(() => media.evaluate((video) => video.currentTime)).toBe(70);
    await expect(appPage.locator(".player-seek-feedback-right span")).toHaveText("20 seconds");

    await clickEdge(box.x + 50, 4);
    await expect.poll(() => media.evaluate((video) => video.currentTime)).toBe(40);
    await expect(appPage.locator(".player-seek-feedback-left span")).toHaveText("30 seconds");
  });

  test("exposes search and filter controls without overflowing", async ({ appPage }) => {
    await openApp(appPage, "/explore");
    await expect(appPage.getByPlaceholder("Search videos and channels")).toBeVisible();
    for (const label of ["All", "Videos", "Channels"]) {
      await expect(appPage.getByRole("radio", { name: label, exact: true })).toBeVisible();
    }
    await expectNoHorizontalScroll(appPage);
  });
  test("a media-session pause in the background stays paused", async ({ appPage }) => {
    // Headset buttons and the lock screen reach the page as Media Session
    // actions. Before the player claimed them, a pause while hidden looked
    // like the browser suspending playback and was resumed at once.
    await appPage.addInitScript(() => {
      window.__sessionHandlers = {};
      if (!navigator.mediaSession) return;
      navigator.mediaSession.setActionHandler = (action, handler) => {
        window.__sessionHandlers[action] = handler;
      };
    });
    // Held open, so the transport never loads a stream and starts it
    // mid-test; the stand-in element below is what plays.
    await appPage.route("**/api/v1/playback/proxy/**", () => new Promise(() => {}));
    // Straight to a watch page: a fresh account's feed has no card to open.
    await openApp(appPage, "/watch/dQw4w9WgXcQ");
    const media = appPage.locator("#tawny-player-media");
    await expect(media).toBeAttached();

    await media.evaluate((video) => {
      let paused = true;
      Object.defineProperty(video, "paused", { configurable: true, get: () => paused });
      video.play = () => {
        paused = false;
        video.dispatchEvent(new Event("play"));
        return Promise.resolve();
      };
      video.pause = () => {
        paused = true;
        video.dispatchEvent(new Event("pause"));
      };
      window.TawnyPlayerControls.attach(video);
      Object.defineProperty(document, "visibilityState", {
        configurable: true,
        get: () => "hidden",
      });
    });

    await appPage.evaluate(() => window.__sessionHandlers.play());
    await expect.poll(() => media.evaluate((video) => video.paused)).toBe(false);

    // The browser suspending playback is still undone...
    await media.evaluate((video) => video.pause());
    await expect.poll(() => media.evaluate((video) => video.paused)).toBe(false);

    // ...but the viewer's pause is kept.
    await appPage.evaluate(() => window.__sessionHandlers.pause());
    await appPage.waitForTimeout(400);
    expect(await media.evaluate((video) => video.paused)).toBe(true);
  });
});
