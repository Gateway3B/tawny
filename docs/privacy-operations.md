# Tawny privacy operations

This records implemented controls and rollout work. It does not certify legal
compliance or claim that B2, DigitalOcean or a telemetry destination has been
configured for Tawny.

## Runtime logs

The server installs a fixed logging filter before Dioxus starts. Dependency
tracing events and surrounding span fields are excluded, and `RUST_LOG` cannot
turn them back on. Request events contain only the router's source-defined
pattern, response status and elapsed milliseconds. They omit requested paths,
query strings, route parameters, headers, cookies, bodies and account IDs.

Service failures use a source-defined enum of fixed diagnostic codes. Playback
resolution retains only source count and cumulative phase timings. It omits
video/channel IDs, provider URLs, token material and raw error chains. Panic
handling emits a fixed code instead of printing its payload or backtrace.
Detailed diagnosis must use synthetic local fixtures rather than enabling raw
logging over live accounts.

The reviewed Python extractor emits only fixed service codes. It suppresses
HTTP access messages and the default request-error traceback/address. Raw
extractor stderr is still returned privately to Tawny for error classification;
it is never printed by either service. Unexpected handler/startup errors also
produce fixed codes. These controls do not promise to intercept every external
process's independent output.

The pull-only production Compose file disables retained output for the database
and upstream PO-token provider, whose diagnostics have not been reviewed. Health
probes and the app's fixed dependency-failure codes remain available. Recreate
containers to apply logging-driver changes. Development Compose keeps its local
diagnostic behavior and must use synthetic data.

Tawny/extractor local json-file logs remain limited to five 10 MiB files per
container. This bounds disk usage; it does not set an age limit. For the shared
host, use g3-platform's reviewed journal policy and rotation/vacuum timer
(14 days plus timer delay), then verify the remote destination's retention.
g3-platform's optional `compose.tawny.yml` now includes Tawny and its extraction
sidecars. Its remote allowlist accepts only Tawny's `request_complete` fields;
extractor messages, service codes and playback timings are not forwarded. The
overlay remains opt-in and no remote trace exporter is enabled. Production
credentials, image compatibility and destination retention still need review.

Inventory and remove or expire historical Docker logs, journal files, forwarded
copies, proxy/CDN logs and provider diagnostics separately. New source redaction
cannot erase previously retained viewing IDs or error payloads. A log copy is
not covered just because the primary database has been erased.

## Shared database identity

Tawny supports explicit `SURREALDB_AUTH_LEVEL=database` with a per-app EDITOR.
The standalone root mode remains a compatibility default, so a shared deployment
must select database mode and supply its exact namespace/database credentials.
There is no root fallback after a scoped login failure. See the
[database workflow](../database/README.md) for migration and the isolated
WebSocket test. The optional platform overlay and
`g3-platform/docs/tawny.md` describe the shared inventory/migration. Source changes do not enable it or provision/migrate
a production database.

## Backups and recovery

Follow the shared platform's encrypted B2 backup and independent-ledger design.
Use separate scoped credentials and off-host decryption keys, inventory existing
snapshots and manual dumps, and verify lifecycle processing at the provider.
The [account-data inventory](privacy-data.md) identifies live tables, device
copies, retained session fences and exceptional request handling.

Tawny implements optional durable deletion recording and a deletion-only
quarantined replay/verification command. The g3-platform hook is prepared;
a packaged Linux image and local encrypted synthetic drill now pass as described
below. Published-image review and live encrypted offsite verification remain
required. Keep an older restored database offline; neither startup session migration nor
a passing health probe proves that past deletions have been reapplied.

## Optional recovery-first deletion

`G3_PRIVACY_RECOVERY=disabled` preserves immediate transactional primary removal
and the earlier null response. `required` saves one UUID request and increasing
subject revision from the authenticated account before any removal. It requires
`G3_PRIVACY_LEDGER_URL` ending in `/events` and a separate app-scoped
`G3_PRIVACY_LEDGER_TOKEN`. These are server-only values, never client build args
or public catalog content. HTTPS is required outside the reviewed private
container/loopback addresses. Unknown modes or incomplete settings fail startup.

A worker claims a five-minute lease. It sends a minimal deletion event with a
hashed non-reused account reference, validates the exact event digest returned
by the independent ledger, and durably checkpoints that acknowledgment before
primary data is removed. Ledger failure, a mismatched receipt, a lost lease or
unreviewed account/request fields leaves the request pending. Primary removal
and its checkpoint commit together. Retries reuse the same event and resume
from a persisted acknowledgment or primary checkpoint. Retry delays grow up to
one hour; the displayed attempt counter is capped at 20. No error text, account
ID, event body or ledger credential enters the routine diagnostic log.

The shared primary erase program retires sessions and verifies playlist relation
removal. A queued request cannot use the legacy direct-purge path. A database
event also rejects profile deletion while recovery recording is pending, and
late registration cannot add credentials over a saved deletion request. Existing
library writes are not frozen while recording is pending; the primary transaction
and existing write fences serialize their removal. Retire older app instances
before relying on this release, and do not disable the worker over pending work.

The private final state is `primary_verified`, not expiry of every retained copy.
It clears the raw account reference from the request and retains minimal request,
subject/revision, digest, state and timestamp evidence. The private session fence
still holds its non-reused account reference. These are retained pseudonymous
operational records, with a separately reviewed retention/access schedule.
Account downloads include only the caller's request ID, state and timestamps;
ledger digests, leases, subjects, errors and future fields are excluded.

Current clients distinguish a saved request from immediate primary removal.
They show saved/pending, keep account access while work is queued and do not
report deletion complete. A changed account ignores the older response. The
queue survives closing the app and restarting the server. Public status proofs,
completion/deadline notifications and hydrated multi-device behavior remain
work; an account download can preserve its private request reference before
primary removal. Deploy clients that understand queued responses before
activating required mode. No production recovery mode is enabled by this change.

Do not activate a launch deletion promise until the scoped independent ledger,
B2 lifecycle, app-specific replay wrapper and an encrypted end-to-end recovery
drill have been verified. This queue does not erase old backups, operational
logs, browser/device copies or an external processor by itself.

## Validation

Local Rust tests capture real HTTP request telemetry and inject private SDK/span
canaries. Extractor tests make local HTTP requests for failed extraction,
unknown paths and unexpected exceptions, and check startup failures. They verify
that useful failure classification still reaches the private caller while raw
request/error details stay out of captured logs. No production log or backup
was read, uploaded, expired or deleted by these tests.

The logging milestone passes 184 server tests and 85 web tests (one existing
live YouTube test ignored), strict checks on all four host builds, nine Node
tests and ten Python extractor tests. Production Compose validates with
synthetic settings. No UI or target-specific client behavior changed here.

Queue milestone validation: 197 server and 87 web tests pass (the existing
live YouTube and explicitly isolated WebSocket tests are ignored in the ordinary
suite). Strict checks pass for all four host builds and actual WASM/Android
targets. The separate disposable database-editor WebSocket drill also passes
with the new queue/schema. Tests cover exact/mismatched ledger receipts, stable
request identity, retries/checkpoint resume, owner isolation, lost leases,
unknown fields including a field arriving during the ledger await, blocked
legacy deletion/late registration and minimal private export. Pending notices
were inspected at 390×844 and 1440×900 in iOS/Material, light/dark. These were
static rendered previews; no app server was started, so hydrated interactions,
server UI diagnostics and real devices remain unverified.


## Quarantined primary replay

The server binary accepts `privacy-replay --apply` or `privacy-replay --verify`
before loading `.env`, initializing the normal app, serving traffic or starting
pollers. It reads one strict bounded event on stdin. Only Tawny deletion events
with a UUID request, positive bounded revision and lowercase SHA-256 subject are
supported. Duplicate/unknown fields and extra deletion payloads are rejected.
The destination must be the named scratch container's exact WebSocket address,
root scope, namespace `restore`, database `tawny`, with
`G3_PRIVACY_RESTORE=quarantined-v1`. These markers constrain the command; only the
platform wrapper's Docker inspection proves the actual quarantine. Do not use
this command against an exposed or production database.

The wrapper authenticates independently recovered ledger objects first. Replay
receives no ledger/provider credentials and makes no provider or ledger calls.
It reinstalls source-defined schema without the ordinary startup data repairs;
backup metadata hashes are not accepted as evidence that guards still exist.
Unexpected tables, fields, events, indexes or function definitions require
review. Original ownership is resolved from the profile, owned rows, tracked
sessions, retained session fence or an existing request. A missing profile can
be recovered from surviving ownership. No surviving identity remains a hold.

The independent event must match any existing request's identity, revision and
acknowledgment digest. Conflicts cannot replace saved evidence. Primary removal
uses the same transaction as the live queue, retires sessions, advances allowed
checkpoints and removes the raw account reference after verification. Repeating
an exact event is safe, including after an interrupted primary checkpoint. It
preserves other accounts, anonymous sessions, shared catalog facts and unowned
demo rows. Missing guards, unrecognized sessions and unattributable orphaned
private rows keep the restore closed for review; they are not silently discarded.
`--verify` checks actual rows, the exact receipt and current definitions without
repairing them. The command prints only fixed success/failure labels and has a
four-minute limit. Its successful result concerns this primary restore, not the
expiry of logs, B2 versions, device copies or all processors.

The actual old logical snapshot is exercised in the existing disposable
WebSocket drill, which exports and imports a synthetic pre-request database,
then applies/verifies the independent event twice. The fixture also checks
preservation and delayed library/session saves. No production backup is used.
The packaged Linux image/local encrypted drill are covered below. Live offsite
recovery and the published registry digest remain separate work, as do public
status proofs, corrections, consent withdrawals and shared-host rollout.


Replay milestone validation: 205 ordinary server and 87 web tests pass, with
strict checks for server, web, mobile and desktop. The separate actual WebSocket
old-snapshot export/import and replay drill passes. Nine Node, ten extractor
Python and eight CI Python tests pass. Recovery tests cover duplicate/unknown
events, exact destinations, repeated apply, each saved checkpoint, conflicting
request/revision/digest evidence, current/legacy metadata, unknown fields/tables,
missing profiles, delayed saves and orphan/guard verification. The disposable
containers and network were removed. No UI changed in this replay milestone;
no final Linux image, real device or offsite provider test is claimed here.


The SDK's default RustyPipe file reporter is also disabled. It otherwise writes
raw request/response reports beneath the extractor directory independently of
tracing filters or journal expiry. Existing report files and cache/BotGuard
copies remain an operator inventory task; see the account-data inventory. No
historical file was scanned or removed by this source change.

Report-configuration validation: 205 ordinary server tests and strict checks for
all four host builds pass. The pinned SDK's documented `no_reporter` setting
is applied at the single production client construction point. This source
check does not inspect old report files or make a live YouTube request.


## Packaged Linux image and encrypted fixture

The full Linux web/server build fetched the private dependency through an
ephemeral BuildKit secret and built successfully with the locked dependencies.
The server code includes `1aaa8c1` (SDK report files disabled). The first runtime
image was `sha256:91dfca57788300a1aa85177202fc69f0c88b2d06ab1a5da77a247724e10d6664`.
The packaged startup drill found that SurrealKit also reads
`database/setup.surql`: an image without it attempts a write under its read-only
working directory. The production Dockerfile now copies that metadata file.

The minimal runtime repair was applied to that built image as
`sha256:3344bb9ffeba436a9ace99d3546ee961a85ac213d6181d30aaf50d00266809aa`.
Its compiled server hash, entrypoint, environment and working directory match
the original build; the added setup file is the runtime packaging correction.
This local image passes actual age encryption/decryption, exact old-backup
import, failed premature verification, repeated independent deletion replay,
conflicting-revision refusal, late-write rejection and preservation of another
account, anonymous sessions, shared video facts and unowned demo rows. The
packaged server then becomes healthy as UID/GID 10001 with a read-only root
filesystem, temporary app-data storage and database-scoped credentials.
All fixture containers/networks are removed; no existing service is reused.

Reproduce after building the app image locally:

```sh
docker build --file ci/privacy-crypto.Dockerfile --tag tawny-privacy-crypto:ci ci
python ci/privacy_linux_drill.py --image YOUR_LOCAL_APP_IMAGE --crypto-image tawny-privacy-crypto:ci
```

The publishing workflow builds and loads a candidate, runs this packaged-server/
encrypted-backup gate for the app, then tags/pushes that same tested local image.
It does not rebuild between verification and publication. The crypto helper has
no private app source or credentials. The source-bearing builder cache stays on
the runner; the runtime image copies only the packaged app and metadata setup.
Build concurrency defaults to two Cargo jobs. Twelve CI helper tests and workflow
syntax checks pass. The changed workflow has not run on GitHub or published an
image, and the GitHub App's actual installation/access still needs verification.

These are local image IDs, not reviewed published registry digests for platform
configuration. The drill uses synthetic data and local encryption on an isolated
network, not B2. Actual B2 history/lifecycle/recovery, custody, destination log
expiry, extractor/provider runtime compatibility, public playback, hydrated UI
and real native devices remain separate checks. No production recovery mode,
policy, cloud credential, DNS or deployment is changed by this exercise.

The subsequent full Linux build from `d38f65b` includes backend effective-terms
checks and the production metadata packaging fix directly. Local image ID:
`sha256:7b1370d550871696c8f54c9e24425051dd9d242facacb7bd36bb8298b21e4b5a`.
It passes the encrypted old-backup drill plus actual packaged Dioxus API checks.
The fixture enables required terms against an unavailable loopback catalog:
ordinary authenticated activity returns 503 while guest/session bootstrap,
account lookup, account export and primary deletion remain available. Responses
are private/no-store, and deletion revokes the session. The same non-root,
read-only runtime uses scoped database credentials and no external network.
The publishing gate now requires this additional check before pushing its tested
candidate. All owned fixtures are removed. This closes the newer gate's local
packaging gap, while hydrated acceptance, actual provider/offsite behavior and a
reviewed registry digest remain separate rollout evidence.
