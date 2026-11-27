//! Database connection-adjacent helpers shared by Tawny's server startup.
//!
//! Schema files live under `database/schema` and are embedded in the server
//! binary. SurrealKit records their hashes in the database, so startup only
//! applies files that changed and can remove schema objects no longer managed
//! by the checked-in schema.

use anyhow::{Context, Result};
use surrealdb::{Surreal, engine::any::Any};

surrealkit::embed_schema!("database/schema");

pub async fn sync_schema(db: &Surreal<Any>) -> Result<()> {
    let version = db
        .version()
        .await
        .context("read SurrealDB server version")?;
    anyhow::ensure!(
        version.major >= 4,
        "Tawny needs SurrealDB 4 or later. Upgrade the datastore before starting Tawny."
    );
    db.query(include_str!("../../database/presync.surql"))
        .await
        .context("apply Tawny pre-sync repairs")?
        .check()
        .context("validate Tawny pre-sync repairs")?;
    embedded_schema::sync(db)
        .await
        .context("sync Tawny schema with SurrealKit")?;
    db.query(include_str!("../../database/backfill.surql"))
        .await
        .context("apply Tawny data backfills")?
        .check()
        .context("validate Tawny data backfills")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn schema_sync_can_repeat_when_the_private_tables_are_empty() {
        let db = surrealdb::engine::any::connect("mem://").await.unwrap();
        db.use_ns("test").use_db("test").await.unwrap();
        super::sync_schema(&db).await.unwrap();
        super::sync_schema(&db).await.unwrap();
    }

    /// SurrealDB 4 refuses a DELETE of NONE, which the duplicate cleanup used
    /// to issue for every follow, so a database with any follow at all failed
    /// to start.
    #[tokio::test]
    async fn schema_sync_keeps_the_newest_of_duplicate_follows() {
        let db = surrealdb::engine::any::connect("mem://").await.unwrap();
        db.use_ns("test").use_db("test").await.unwrap();
        super::sync_schema(&db).await.unwrap();
        // Rows the unique index never saw, as some development databases hold.
        db.query(
            "REMOVE INDEX user_subscription_owner_channel_idx ON user_subscription;
             CREATE user_subscription:old SET owner = app_user:a, channel_id = 'c',
                 updated_at = d'2026-01-01T00:00:00Z';
             CREATE user_subscription:new SET owner = app_user:a, channel_id = 'c',
                 updated_at = d'2026-02-01T00:00:00Z';
             CREATE user_subscription:solo SET owner = app_user:a, channel_id = 'd';",
        )
        .await
        .unwrap()
        .check()
        .unwrap();

        super::sync_schema(&db).await.unwrap();

        let mut kept = db
            .query("SELECT VALUE record::id(id) FROM user_subscription ORDER BY id")
            .await
            .unwrap();
        let kept: Vec<String> = kept.take(0).unwrap();
        assert_eq!(kept, ["new", "solo"]);
    }
}
