# Database workflow

`database/schema` is Tawny's desired schema. The server currently embeds it and
runs SurrealKit during startup. `database/presync.surql` is applied just before
the sync to repair data that would fail an index rebuild, and
`database/backfill.surql` is applied immediately afterward. Keep schema files and the application checkout at the same revision.

## Scoped operator commands

Copy `.env.surrealkit.template` to an ignored file named for the target
environment, then replace the environment marker, Tailscale service URL, and
credentials:

```powershell
Copy-Item .env.surrealkit.template .env.surrealkit.production
```

Run any SurrealKit command through the scoped launcher:

```powershell
.\scripts\db.ps1 production status
.\scripts\db.ps1 production sync --fail-fast
.\scripts\db.ps1 production seed
.\scripts\db.ps1 production rollout status
.\scripts\db.ps1 production rollout baseline
.\scripts\db.ps1 production rollout plan --name add-tags
.\scripts\db.ps1 production rollout lint <rollout-id>
.\scripts\db.ps1 production rollout start <rollout-id>
.\scripts\db.ps1 production rollout complete <rollout-id>
.\scripts\db.ps1 production rollout rollback <rollout-id>
```

The launcher temporarily applies `.env.surrealkit.<environment>`, runs from the
repository root, and restores the calling process's environment afterward. It
does not modify `.env`, so later development commands keep their local settings.

Tawny does not currently have a `database/seed/` directory, so `seed` has
nothing project-specific to apply until seed files are added. The startup
backfill is not a SurrealKit seed or rollout.

Before switching an existing production database to operator-controlled
rollouts, first move release startup schema changes and `backfill.surql` into
the reviewed rollout workflow. Until then, startup remains authoritative.

## Runtime identity on the shared server

Set `SURREALDB_AUTH_LEVEL=database` with explicit namespace, database, username
and password. Tawny authenticates at that exact scope; a failed scoped login
never retries as root. Missing credentials or an unknown auth level fail
startup without printing the supplied values. Existing standalone deployments
keep the `root` default for compatibility; the local template names it
explicitly. This default is not suitable for shared-host app credentials.

Apply the schema with a separate migration identity, provision a per-app
EDITOR using g3-platform's reviewed provisioning flow, and retire older root
connections before enabling shared traffic. Startup still runs SurrealKit and
backfills; operator-controlled release rollouts remain work as stated above.
Tawny is not yet in g3-platform's Compose/service inventory, and this change
does not move its database or provision a production identity.

`python ci/privacy_auth_drill.py` creates a memory-only SurrealDB 3.2.4 fixture,
with outbound SQL networking and scripting disabled, no host data volume,
read-only filesystem, resource limits and no retained container logs. It binds
only a random loopback port and cleans up the exact returned container/network
IDs. It runs the explicitly ignored WebSocket test outside sibling Cargo patch
scopes, using the locked, pre-fetched dependencies. Ordinary tests always use
isolated memory connections and ignore deployment database environment values.
The server CI job also runs this drill after fetching its private dependency.

The wire drill covers startup schema/backfills on an empty database, legacy
subscription duplicate repair, private receipt writes, transactional account
and session removal, delayed-write refusal, denial of root-user creation,
and foreign private-data read/write isolation. It also checks that supplying
valid root credentials in database mode fails without a root retry. Pre-sync
binds its subscription SELECT before iterating; this fixes the remote 3.2.4
empty-iterator failure exposed by the drill.

Validation: 187 server tests and 85 web tests pass, with strict checks for all
four host builds. The full server run excludes the existing live YouTube test
and the explicit wire drill; the wire drill also passes when run separately.
Eight Python CI tests, workflow lint, formatting, spelling and both Compose
layouts pass. This uses only synthetic data in newly created local fixtures;
existing services, host database volumes and production credentials are untouched.


The same disposable drill now checks recovery-first deletion and a real logical
old-snapshot export/import. It restores into its own `restore/tawny` database,
replays an independently supplied synthetic deletion event twice and verifies
actual primary/session removal while preserving another account, shared video
facts, relations and unowned demo rows. See [privacy operations](../docs/privacy-operations.md)
for the release-specific quarantine boundary. This test does not build the final
Linux app image or contact B2; encrypted offsite recovery remains required.
