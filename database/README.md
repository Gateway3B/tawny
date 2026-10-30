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

## Runtime database identity

The local home-lab setup defaults to root authentication. To use a database-scoped
identity, set `SURREALDB_AUTH_LEVEL=database` and explicitly provide the namespace,
database, username and password. Tawny authenticates at that scope and does not
retry a failed scoped login as root. Startup still applies the schema and data
backfills, so the chosen identity must be allowed to perform those operations.

Tests use isolated in-memory databases and ignore deployment database credentials.
Account deletion removes the account's library and sessions in one transaction,
while preserving the shared YouTube catalog and other accounts. Back up and
restore the SurrealDB volume using your own home-lab backup process.
