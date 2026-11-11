const { test, expect, openPlaylistWithVideos } = require("./fixtures/app.cjs");

async function prepareVideo(page) {
  if (page.viewportSize().width < 768) {
    await page.setViewportSize({ width: 390, height: 844 });
  }
  await openPlaylistWithVideos(page);
  const response = await page.request.post("/api/v1/videos/by-id", {
    data: { ids: ["M7lc1UVf-VE"] },
  });
  expect(response.ok()).toBe(true);
  const [video] = await response.json();
  await page.route("**/api/v1/playback/*?*", (route) =>
    route.fulfill({ status: 503, body: "Playback omitted in metadata recovery test" }),
  );
  return {
    video,
    channel: null,
    description: "Recovered description from YouTube",
    like_count: 0,
    dislike_count: 0,
    captions: [],
    chapters: [],
    preview_frames: null,
    related_videos: [],
    comments: {
      comments: [
        {
          id: "recovered-comment",
          author: "A viewer",
          author_avatar_url: null,
          author_channel_id: null,
          text: "Recovered comment from YouTube",
          published_at: "1 day ago",
          like_count: 0,
          reply_count: 0,
          pinned: false,
          hearted: false,
          creator: false,
        },
      ],
      next_page: null,
      disabled: false,
      remote_available: true,
    },
    remote_available: true,
  };
}

function incomplete(details) {
  return {
    ...details,
    description: "",
    comments: { comments: [], next_page: null, disabled: false, remote_available: false },
    remote_available: false,
  };
}

async function openVideo(page) {
  await page.locator('.video-card-frame[data-scroll-anchor="M7lc1UVf-VE"] .g3-card-action').click();
  await expect(page).toHaveURL(/\/watch\/M7lc1UVf-VE/);
  await page.getByRole("button", { name: /^Comments ·/ }).click();
}

test("details and comments recover after a transient extraction failure", async ({
  appPage,
}, testInfo) => {
  const details = await prepareVideo(appPage);
  let requests = 0;
  await appPage.route("**/api/v1/videos/M7lc1UVf-VE", (route) =>
    route.fulfill({ json: ++requests === 1 ? incomplete(details) : details }),
  );
  await openVideo(appPage);
  await expect(appPage.getByText("Recovered comment from YouTube", { exact: true })).toBeVisible();
  await expect(
    appPage.getByText("Recovered description from YouTube", { exact: true }),
  ).toHaveCount(1);
  expect(requests).toBe(2);
  await expect(appPage.getByText("Comments need a connected video source.")).toHaveCount(0);
  await appPage.screenshot({ path: `test-results/details-recovered-${testInfo.project.name}.png` });
});

test("exhausted retries offer recovery without reopening the video", async ({ appPage }) => {
  const details = await prepareVideo(appPage);
  let recovered = false;
  let requests = 0;
  await appPage.route("**/api/v1/videos/M7lc1UVf-VE", (route) => {
    requests += 1;
    return route.fulfill({ json: recovered ? details : incomplete(details) });
  });
  await openVideo(appPage);
  const sheet = appPage.getByRole("dialog", { name: "Comments", exact: true });
  await expect(sheet.getByText("Comments unavailable", { exact: true })).toBeVisible({
    timeout: 15_000,
  });
  expect(requests).toBe(3);
  await expect(appPage.getByText("Description unavailable", { exact: true })).toHaveCount(1);
  recovered = true;
  await sheet.getByRole("button", { name: "Try again", exact: true }).click();
  await expect(sheet.getByText("Recovered comment from YouTube", { exact: true })).toBeVisible();
  await expect(
    appPage.getByText("Recovered description from YouTube", { exact: true }),
  ).toHaveCount(1);
  expect(requests).toBe(4);
});

test("failed comments refresh independently of an available description", async ({ appPage }) => {
  const details = await prepareVideo(appPage);
  let requests = 0;
  await appPage.route("**/api/v1/videos/M7lc1UVf-VE", (route) =>
    route.fulfill({
      json: ++requests === 1 ? { ...details, comments: incomplete(details).comments } : details,
    }),
  );
  await openVideo(appPage);
  await expect(appPage.getByText("Recovered comment from YouTube", { exact: true })).toBeVisible();
  expect(requests).toBe(2);
});
