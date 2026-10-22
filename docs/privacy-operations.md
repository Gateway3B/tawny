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

Tawny's current deletion is transactional live removal with session retirement
and late-write fences. It does not yet durably publish a recovery event to the
independent ledger. Its app-specific quarantined replay/verification command,
platform wrapper and encrypted offsite drill remain required work. Therefore,
keep an older restored database offline; neither startup session migration nor
a passing health probe proves that past deletions have been reapplied.

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
