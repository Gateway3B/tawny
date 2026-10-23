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
Private legal review and exact-set receipts are implemented in the subsequent
legal milestone below. A bounded private account download follows in the data milestone. Terms
authorization, independent deletion recovery and offsite restore replay remain work. Those need Tawny's
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

## Private legal review

About shows the published `tawny`/`en` document set, exact revisions, effective
dates and change summaries. An authenticated guest or registered account can
explicitly acknowledge a notice or accept the applicable terms. The backend
resolves the current set itself and re-fetches before recording an action; a
stale digest, wrong account or unreviewed consent purpose is rejected. Notice
acknowledgment does not satisfy a terms requirement or grant processing consent.

Receipts live only in this app server's `brand_legal_receipt` table, with a
server timestamp, account owner, account-derived subject digest, app, set
digest and action. `expected_owner` is a review binding; the session chooses
the account. Private status/actions are protected under `/api/v1/auth/` with
no-store responses. Legal actions are limited to three per minute per session.
No private status or receipt enters the shared public catalog or its cache.

The receipt transaction touches the account row so it conflicts with concurrent
deletion. Primary deletion erases the owner's receipts in the same transaction
as its library/account. Later receipt writes fail instead of recreating an
account or leaving orphaned evidence. The new table is part of the app's
automatically embedded schema and owner-table deletion inventory. Backups
still contain historical receipt copies until the operator's retention or
future independent replay removes them. A bounded owner-projected account download is now implemented; see
[the data inventory](privacy-data.md) for scope and remaining operator work.

A dismissible change notice is keyed to account and document-set digest. It
never counts dismissal as acceptance. About and Settings stay accessible, and
store-update notices take precedence so the two do not compete for focus.
Public update notices also remain mounted on the backend setup screen. A
read-only public copy of legal documents can be read without an account or
backend connection when the catalog is available/saved; it provides no private
action or acceptance claim.

No shared legal set has been published by this implementation. Server operators
must review applicability, including their own processing/hosting obligations,
before publishing documents for users. No consent purposes or backend terms
enforcement are enabled. This is a notice/receipt pilot, not a compliance claim.

Validation: 84 web and 168 server tests pass (one existing live YouTube test
ignored), with strict Clippy for web/server/mobile/desktop on the host. Tests
cover two-account isolation, exact revisions, terms-versus-notice semantics,
rejected purposes, erasure, late writes and twelve concurrent deletion/receipt
interleavings. Read-only legal markup was inspected at phone/wide dimensions
in both modes/themes with published/missing synthetic content. No actual legal
text was published or accepted. Hydrated alerts/acknowledgments, real native
flows, database-editor WebSocket permissions and production/offsite recovery
remain unverified for this milestone.

## Account-data download

Settings now offers an authenticated, bounded account-data copy, including
legal receipts. [The data inventory](privacy-data.md) defines each section,
credential exclusions, paging and size limits, web/mobile delivery and remaining
operator responsibilities. The latest checks pass 85 web/173 server tests, strict
checks for all four host targets, and actual WASM/Android strict checks. The new
card was inspected in both modes/themes at phone/wide sizes. Actual file/share
delivery and independent deletion/offsite recovery still need verification.

## Live erasure and stored sessions

Private writes now conflict transactionally with account removal, and the
reviewed account-tracked `g3-auth` source is pinned at `9f68e70c`. Startup indexes
known legacy sessions before serving, deletion removes their payloads with the
account, and private retirement fences reject late session saves. Unknown
legacy formats hold startup/deletion for review. Database guards reject writes
from an older untracked session adapter. See [the inventory](privacy-data.md)
for retained fence data, device/process limits and recovery work. These source
changes do not configure B2, deploy the service or make an old restore safe.

## Optional backend terms checks

`G3_LEGAL_ENFORCEMENT` defaults to `disabled`. After reviewed document
publication and rollout validation, `required` checks protected API activity
against the signed-in account's acceptance of the exact current document set.
This includes guest accounts and registration of an existing guest. Future
effective dates are respected. A notice acknowledgment, dismissal, another
account's acceptance or an older set cannot authorize activity or processing
consent. Missing documents, catalog/database failures and unverifiable state
keep protected activity closed. Unknown modes refuse normal startup.

Protected requests use a validated public catalog for at most 60 seconds before
requiring a refresh; failure cannot authorize activity from an expired snapshot.
requests share a ten-second failure cooldown, including forced review refreshes,
so an outage cannot queue repeated slow fetches. Recovery must revalidate content.
Normal authentication still guards private endpoints. Sign-in/guest bootstrap, account
lookup, sign-out, legal review/actions and account export/deletion remain
available without accepting changed terms. About and Settings remain public.
Successful acceptance refreshes cached reads that the gate may have refused.
Gate errors are `private, no-store` and do not expose private receipt details.

Health, WebSub and playback proxy routes retain their existing sessionless
boundaries: callback signatures and already issued short-lived proxy tokens
still supply their own authorization. New playback sessions pass through the
protected API. This setting neither deletes device caches nor revokes an
already issued media capability. Keep the mode consistent across replicas and
use a reviewed image containing the gate before enabling it.

The in-process HTTP/session test uses the actual tracked session adapter and
receipt transactions. It verifies account isolation, exact acceptance, changed
sets, catalog failure, essential access and session retirement after deletion.
No server, policy publication, consent purpose or deployment is enabled here.
Hydrated acceptance/retry, native interactions and a packaged image containing
this newer gate remain rollout checks; the earlier packaged deletion-recovery
evidence applies to its recorded source snapshot.

Validation passes 210 ordinary server tests, 87 web tests and strict checks for
server/web/mobile/desktop on the host. The separate database-editor fixture and
live YouTube test remain opt-in; neither is included in that ordinary count.
Nine JavaScript, ten extractor and twelve CI helper tests pass. Both Compose
files resolve using synthetic settings. Web lint exits successfully with its
existing 50 generated-CSS warnings; formatting passes. No UI markup changed.
The real loopback catalog test covers the 60-second expiry boundary, failure
cooldown, retry/recovery, forced refresh and a backwards clock. The actual
WebAssembly strict check also passes.
