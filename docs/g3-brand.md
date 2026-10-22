# Shared g3tech information

Tawny renders `g3-brand` beneath its existing `AppWrapper`; it inherits Tawny's
appearance and iOS/Material mode. Settings and the initial server setup screen
link to `/about-g3tech`. The page is public and remains available after server
connection failure. Opening it keeps the page and player beneath the sheet.
The normal guest-session bootstrap still runs; the catalog receives no account
identifier, session credential or private receipt.

## Public catalog and updates

The default catalog is `https://catalog.g3tech.net`. To change it, export
`G3_CATALOG_ORIGIN` in the Rust build environment. This is compiled public
configuration, not a runtime server secret; putting it only in `.env` does not
change a compiled client. Docker and mobile CI explicitly supply the value.
`G3_RELEASE_CHANNEL` accepts `production`, `beta`, or `development`. Unknown
values select development. An omitted value selects development for debug
builds and production for release builds.

The client validates snapshots, retains the last valid one offline, refreshes
after an hour with ETags, and checks on foreground, page restore, reconnect and
a minute clock. Automatic attempts are throttled to once a minute. Manual
refresh is available on About. An unavailable catalog never blocks help.
Only public catalog data and optional update dismissals are stored, using
Tawny-specific keys. Web/desktop use webview localStorage and Android/iOS use
the native key-value store. Storage failures only lose the saved snapshot.

Native notices compare the installed package's real version/build metadata.
Missing metadata, expired policies, mismatched channels, unpublished or not
broadly available releases cannot require an update. Optional reminders snooze
for 24 hours per exact policy; required reminders cannot inherit a snooze.
About and Settings remain accessible. Web and desktop do not receive store
requirements. The catalog is not backend authorization or permission to
publish a release policy. No real store IDs or policies are invented here.
Validated external links open via the system browser/store on Android/iOS;
web and desktop use external anchors (handled by Dioxus desktop).

## Private builds

The manifest and portable lockfile pin private `g3-brand` revision
`07d3e99391fcafabfcc6b5e0a044ac9ab4f54d7d` and the reviewed public native
revision `1896034867ae692db0503c38136cfbcfc804448a`. No sibling checkout is
needed for portable builds. A local parent Cargo configuration may override
sources, so validate from outside that configuration's directory scope with
an explicit `--manifest-path` and `--locked`.

CI needs a GitHub App installed on **g3techhq/g3-brand** with contents read
permission. Configure repository variable `G3_DEPENDENCIES_APP_CLIENT_ID` and
secret `G3_DEPENDENCIES_APP_PRIVATE_KEY` in the Tawny repository. Its current
workflows publish to `ghcr.io/gateway3b`; if Tawny and g3-brand have different
owners, organization secrets do not automatically cross that boundary.
Provide the app credentials to this repository explicitly. No repository or
registry migration is implied.

Trusted same-repository PR jobs fetch pinned dependencies with a short-lived,
one-repository token before long compilation. Fork PRs run formatting and
Node/Python quality checks without private-source access; Rust jobs wait for
a trusted branch. No Cargo source/target caches or source-bearing Docker
builder caches are exported. The extractor has no private dependency access.

Local Cargo users can use their normal Git credential manager with access to
g3-brand. For a local Docker source build, export a short-lived read token as
`G3_BRAND_READ_TOKEN` and run the existing production Compose build. Compose
passes it only as the `g3_brand_token` BuildKit secret. Direct builds use
`docker build --secret id=g3_brand_token,env=G3_BRAND_READ_TOKEN .`. The helper
answers only for the exact HTTPS g3-brand repository. Fetch uses the locked
revisions; compilation is locked and offline. Only the app bundle enters the
fresh runtime stage. Never put the token in `.env`, build arguments, image
labels, source files or runtime environment. Do not export builder caches.

## Remaining rollout

This milestone provides public discovery and native update presentation.
It does not implement Tawny's private legal receipts, terms authorization,
privacy export/deletion recovery or offsite restore replay. Those need Tawny's
own account/data inventory and reviewed policy configuration. The catalog
service, hostname, published app listings, store distribution and CI app
credentials still need deployment/configuration. No policy was published and
no app, container or crate was released by this change.

Validation results are recorded with the implementation commit. Static markup
fixtures check host themes and offline states; they cannot prove hydration,
actual native package metadata, foreground lifecycle, system-browser opening,
player continuity or a live catalog request. Those require the running app
and real native bundles. Do not equate static previews with those checks.

## Validation for the catalog milestone

- 81 web tests and 157 server tests pass; the existing live YouTube test
  remains ignored. Nine Node checks and six extractor Python tests pass.
- Strict Clippy passes for web, server, mobile and desktop on the host, plus
  real WASM and Android targets. The five credential-helper protocol tests,
  workflow validation, Dockerfile checks, Compose validation, formatting,
  spelling and whitespace checks pass. Existing styling lint warnings remain.
- Static browser fixtures were inspected at 390×844 and 1440×900 in iOS and
  Material, light and dark, including saved, unavailable and loading states.
  Keyboard navigation reaches the external listing link. No dx server was
  started. A temporary preview server was stopped afterward.
- A hydrated app flow, installed Android/iOS package, macOS/iOS compile,
  live GitHub App credential fetch and complete Linux image build have not
  been verified by these checks.

## Account review prerequisite

Registration and deletion require `expected_owner`, matching the account the
form showed before submission or confirmation. The server still chooses the
account from the authenticated session; this argument never selects a target.
Missing bindings from older clients fail with a refresh/update message. Deploy
compatible clients before enabling these handlers for existing native users.

Account forms are keyed by account ID so a switch discards credential drafts.
Confirmations freeze that ID before awaiting input and check it again before
submitting. Old responses cannot adopt an account, clear a different account's
viewer, or display success after a switch. Device-cache cleanup finishes before
forgetting the session unmounts the form; reload runs in the root scope and
survives that unmount. Sign-out failures are shown instead of being ignored.
Browser cookie races and concurrent persistent-cache ownership operations
still need a hydrated multi-tab check; these guards do not claim to settle
those framework/storage races.

A registration that resumes after password hashing only changes an existing
guest with no email. It cannot overwrite another registration or recreate a
deleted account. Account HTTP results and errors explicitly use
`Cache-Control: private, no-store`, outside the session/authorization layers.
The deletion confirmation distinguishes primary server removal from backups
managed by the server operator. Independent recovery and retention are still
separate, unfinished Tawny work.

The account-review milestone passes 82 web tests, 162 server tests (one
existing live YouTube test ignored), and strict Clippy for all four host
feature sets. Database tests cover missing/mismatched review bindings,
owner isolation, a late registration after deletion, and preservation of a
completed registration. An HTTP test verifies no-store headers on account
successes, failures and missing endpoints through the CDN guard. Hydrated
confirmation/account-switch and real-device cache cleanup remain unverified.
