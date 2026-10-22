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
Tawny is not yet in that deployment's service/collector inventory; this change
adds no remote exporter or trace stream. Its `request_complete` format matches
the existing collector's approved field projection. Service codes and playback
timings require separate review before any remote forwarding is enabled.

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
WebSocket test. This source change does not add Tawny to the platform inventory
or provision/migrate a production database.

## Backups and recovery

Follow the shared platform's encrypted B2 backup and independent-ledger design.
Use separate scoped credentials and off-host decryption keys, inventory existing
snapshots and manual dumps, and verify lifecycle processing at the provider.
The [account-data inventory](privacy-data.md) identifies live tables, device
copies, retained session fences and exceptional request handling.

Tawny implements optional durable deletion recording and a deletion-only
quarantined replay/verification command. The g3-platform hook is prepared;
a reviewed final Linux image and encrypted/offsite drill remain required work.
Keep an older restored database offline; neither startup session migration nor
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
The final Linux image and encrypted/offsite drill remain separate work, as do
public status proofs, corrections, consent withdrawals and shared deployment.


Replay milestone validation: 205 ordinary server and 87 web tests pass, with
strict checks for server, web, mobile and desktop. The separate actual WebSocket
old-snapshot export/import and replay drill passes. Nine Node, ten extractor
Python and eight CI Python tests pass. Recovery tests cover duplicate/unknown
events, exact destinations, repeated apply, each saved checkpoint, conflicting
request/revision/digest evidence, current/legacy metadata, unknown fields/tables,
missing profiles, delayed saves and orphan/guard verification. The disposable
containers and network were removed. No UI changed in this replay milestone;
no final Linux image, real device or offsite provider test is claimed here.
