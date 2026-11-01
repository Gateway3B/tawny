# Tawny test suite

The suite is split by feedback speed and runtime dependency. Rust unit tests stay beside the code they cover, browser-runtime tests stay in `tests/*.cjs`, the yt-dlp service keeps its `node:test` tests beside that service (`docker/extractor/server.test.mjs`), and end-to-end UI tests live in `tests/ui`.

## Commands

| Command | Coverage |
| --- | --- |
| `npm test` | JavaScript runtime tests and yt-dlp service tests |
| `npm run test:rust` | Web-safe Rust tests plus server-feature Rust tests |
| `npm run test:ui` | Playwright UI suite on phone and desktop Chromium |
| `npm run test:ui:mobile` | Phone-sized UI suite only |
| `npm run test:ui:desktop` | Desktop-sized UI suite only |
| `npm run test:all` | Unit, Rust, and UI layers |

Playwright reuses a Dioxus server already listening on port 8080. When no server is running, its configuration starts `dx serve` for the duration of the suite. Set `PLAYWRIGHT_BASE_URL` to test an already-running app at another address, or `PLAYWRIGHT_PORT` to change the managed server port.

Browser cases run one at a time so local cache and sync-state mutations remain deterministic.

Install the browser once with `npx playwright install chromium`.
The yt-dlp service tests run on Node alone; they replace yt-dlp and the PO-token provider with stand-ins, so neither needs to be installed.

## UI case inventory

- Responsive shell: phone bottom tabs, desktop rail, selected state, and horizontal overflow.
- Primary routes: Feed, Playlists, Search, and Subscriptions.
- Auxiliary routes: Settings presentation and back navigation.
- Local library: playlist validation, creation, persistence within the page, and modal dismissal.
- Search: input and media/channel filter availability at both viewport sizes.
- Failure diagnostics: uncaught page errors fail the test; console errors, screenshots, video, and traces are retained with failures.

Network-backed catalog and playback tests should use a controlled mock server before they are added. The default suite intentionally avoids depending on YouTube availability.
