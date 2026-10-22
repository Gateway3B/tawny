# Tawny account data

This inventory describes the account-data download pilot. It is not a claim
that backup expiry, operational logs or external systems
have been reconciled after an account is deleted.

| Storage | Owner and download scope | Primary deletion |
| --- | --- | --- |
| app_user | Session-derived account; display name, email, guest status and account timestamps | Remove account row |
| user_subscription | Owner; channel ID, follow state, content choice and timestamp | Remove owned rows |
| subscription_group | Owner; group ID, name, channel references and timestamp | Remove owned rows |
| playlist | Owner; playlist ID, name, description, pinned/order, video references and timestamp | Remove owned rows |
| playlist_item | Source playlist's owner; playlist/video reference, position and added time | Remove owned playlist relations first |
| library_state | Owner; queue, history and timestamp | Remove owned row |
| video_progress | Owner; video ID, watched/progress/audio preference and timestamp | Remove owned rows |
| subscription | Source account; legacy channel reference and timestamp | Remove owned relations |
| brand_legal_receipt | Owner; app, document-set digest, action, purpose and receipt timestamp | Remove owned rows with account/library |
| sessions | Authentication payloads/cookies; excluded from downloads | Startup indexes known legacy payloads and removes orphan sessions; primary deletion removes indexed sessions and irreversibly retires the identity |
| channel, video, video_detail_cache | Shared public YouTube metadata; download only the account's reference IDs | Preserve shared metadata; recompute channel interest |
| Device preferences/caches/downloaded files | Per-device storage, excluded from the server download | Account flows clear the ordinary read cache; device settings and saved files need user handling |
| Server caches, extractor data, operational logs and backups | Operator-managed systems, outside the self-service JSON projection | Separate inventory, retention and recovery verification still required |

`/api/v1/auth/export` is an authenticated POST with an `expected_owner` review
binding. The server derives the account independently, uses fixed field lists,
orders rows by record ID, and reads pages of at most 128 records. The UI ignores
responses for a switched account. These downloads bypass the ordinary device
read cache and public catalog. Account middleware marks responses/errors
`private, no-store`. The endpoint allows one request per minute per session,
ten per minute globally, two simultaneous collections and 30 seconds per request.

The file contains up to 100,000 projected rows and 32 MiB of JSON. A projected
row is limited to 64 Ki characters of database text before its data crosses the
database connection; an oversized row returns a refusal, not truncated data.
The final UTF-8 file size is checked separately. Counts, cursor order, malformed
queue/history and an account disappearing during collection are checked. The
file records start/finish times and states that concurrent edits may require
another collection. It is a live account collection, not a database snapshot.

Password hashes, session payloads, operational credentials, future private
columns, receipt subject hashes and other accounts' data are excluded. Unknown
new owner tables require an explicit projection: a schema inventory test fails
until their download scope is reviewed. The operator must assist with larger,
malformed or otherwise exceptional data requests; no partial file is reported
as complete. This path does not export separate logs/backups or assert that
an external processor has completed a data request.

Web saves `tawny-account-data.json` using a data channel and a temporary Blob
URL, revoked after a minute. Android/iOS open the native share sheet only for
copies up to 128 KiB; larger copies direct the user to the web app/operator.
Desktop currently directs the user to web/mobile before fetching private data;
a desktop file-save flow remains work. No export is persisted on the server,
no public download token is issued, and a successful fetch alone is not reported
as successful delivery. The saved/shared file contains private account data.

Tests cover one-record paging across two accounts, every owned section,
credential/session/future-column canaries, missing/switched review bindings,
row/byte/configuration bounds, malformed library state and deleted accounts.
The milestone passes 85 web/173 server tests (one existing live YouTube test
ignored), strict checks on all four host feature sets, and strict actual WASM
and Android checks. Ready/loading cards were inspected at 390×844 and 1440×900
in both modes/themes. Actual browser file delivery, native sharing, iOS builds,
multi-tab cookie/cache races, database-editor WebSocket permissions, offsite
recovery remain unverified or unfinished as stated above.

## Live database write fences

The six owner tables and both private relation tables reject creates/updates
for a missing account. Each private write updates the account's private
`privacy_revision` counter in the same transaction. Deletion removes that
account row, so an older database snapshot cannot commit a private write
after deletion. Owned rows cannot be reassigned to another account or made
unowned, and relation endpoints cannot be changed. Existing unowned demo rows
remain unowned; no account is inferred for them.

These guards are embedded with the desired schema and applied by startup
schema sync. Retire older app instances before relying on the new deployment;
verify the schema through the database editor's actual credentials. Tests cover
normal library operations, all private tables/relations, a transaction holding
an old account snapshot, cross-account isolation and legacy unowned data. This
protects the live database; it does not reconcile older orphan rows or make a
restored backup safe to serve without independent erasure replay.

Guard milestone validation: 178 server tests and 85 web tests pass, with the
existing live YouTube check ignored; strict checks pass for web, server,
mobile and desktop. The Node/Python suites also pass. No UI changed in this
milestone, and no production database was modified.

## Saved authentication sessions

Tawny source-pins the reviewed `g3-auth` adapter at `9f68e70c`. Serving uses
account tracking, and user lookup caching stays disabled. Before accepting
requests, startup indexes known plaintext legacy sessions in 128-row batches
and removes sessions naming a missing account. Migration has a 180-second
limit. Unknown/encrypted, malformed or oversized legacy payloads hold startup
with a generic diagnostic; they are not guessed, discarded or logged.

Account deletion checks that no unindexed sessions remain and removes the
account's sessions in the same transaction as its library/profile. The private
`auth_session_subject` fence stores only the account reference, retirement flag
and timestamp. Delayed tracked responses touch that same fence and cannot
restore retired payloads. A database event rejects the untracked format from
older binaries. Other accounts and anonymous session payloads stay intact.
The fence is retained private operational data, not anonymized data or a public
receipt. Its retention must cover supported backup/restore horizons.

Stop older app instances before the migration and do not roll back to an
untracked session writer. This does not immediately wipe every process's
in-memory session copy, browser cookie or saved file. Deleted accounts cannot
resolve for authentication; cache expiry, device handling and independent
recovery replay remain separate responsibilities. Never serve an old restored
database before the recovery process verifies it. No external provider session
or grant is implied to be revoked by this local operation.

Session milestone validation: 181 server and 85 web tests pass (one existing
live YouTube test ignored), with strict checks for all four host builds and
actual WASM/Android targets. Tests cover repeated legacy migration, exact
account and anonymous-session preservation, orphan cleanup, unknown/large
payload holds, atomic deletion and twelve concurrent deletion/store races.
No production sessions or offsite backups were accessed or changed.
