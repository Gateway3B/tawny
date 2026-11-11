//! Explicit authentication scope, without retrying scoped failures as root.
//!
//! A server always connects to the SurrealDB its Compose stack runs. Only the
//! backend inside the app (`device_backend`) keeps its own database file.
use std::path::Path;

use anyhow::{Result, anyhow};
use surrealdb::{Surreal, engine::any::Any};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Scope {
    Root,
    Database,
}

// Never Debug: these values include the database credential.
struct Access {
    endpoint: String,
    namespace: String,
    database: String,
    username: String,
    password: String,
    scope: Scope,
}

impl Access {
    fn read(read: impl Fn(&str) -> Option<String>) -> Result<Self> {
        let required = |key| {
            read(key)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| anyhow!("Required database configuration is missing."))
        };
        let scope = match read("SURREALDB_AUTH_LEVEL").as_deref().unwrap_or("root") {
            "root" => Scope::Root,
            "database" => Scope::Database,
            _ => return Err(anyhow!("SURREALDB_AUTH_LEVEL must be root or database.")),
        };
        // Existing standalone root environments keep their defaults. A shared
        // database identity must name its scope explicitly before connecting.
        let (namespace, database) = if scope == Scope::Database {
            (
                required("SURREALDB_NAMESPACE")?,
                required("SURREALDB_NAME")?,
            )
        } else {
            (
                read("SURREALDB_NAMESPACE").unwrap_or_else(|| "tawny".into()),
                read("SURREALDB_NAME").unwrap_or_else(|| "main".into()),
            )
        };
        if namespace.is_empty() || database.is_empty() {
            return Err(anyhow!("Database namespace and name must not be empty."));
        }
        Ok(Self {
            endpoint: required("SURREALDB_HOST")?,
            namespace,
            database,
            username: required("SURREALDB_USER")?,
            password: required("SURREALDB_PASSWORD")?,
            scope,
        })
    }

    async fn open(&self) -> Result<Surreal<Any>> {
        let db = surrealdb::engine::any::connect(self.endpoint.clone())
            .await
            .map_err(|_| anyhow!("Database connection failed."))?;
        match self.scope {
            Scope::Database => {
                db.signin(surrealdb::opt::auth::Database {
                    namespace: self.namespace.clone(),
                    database: self.database.clone(),
                    username: self.username.clone(),
                    password: self.password.clone(),
                })
                .await
                .map_err(|_| anyhow!("Database-scoped authentication failed."))?;
            }
            Scope::Root => {
                db.signin(surrealdb::opt::auth::Root {
                    username: self.username.clone(),
                    password: self.password.clone(),
                })
                .await
                .map_err(|_| anyhow!("Root database authentication failed."))?;
            }
        }
        db.use_ns(self.namespace.clone())
            .use_db(self.database.clone())
            .await
            .map_err(|_| anyhow!("Database selection failed."))?;
        Ok(db)
    }
}

pub async fn connect() -> Result<Surreal<Any>> {
    if cfg!(test) {
        // Ordinary tests must never connect to a developer's configured DB.
        let db = surrealdb::engine::any::connect("mem://").await?;
        db.use_ns("test").use_db("test").await?;
        return Ok(db);
    }
    Access::read(|key| std::env::var(key).ok())?.open().await
}

/// The library file a backend inside the app keeps under `data_dir`, in
/// SurrealDB's embedded engine. It has no users to sign in as.
pub async fn connect_embedded(data_dir: &Path) -> Result<Surreal<Any>> {
    let endpoint = format!("surrealkv://{}", data_dir.join("database").display());
    let db = surrealdb::engine::any::connect(endpoint)
        .await
        .map_err(|_| anyhow!("Could not open the library file."))?;
    db.use_ns("tawny").use_db("main").await?;
    Ok(db)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values(scope: Option<&str>, omit: Option<&str>) -> impl Fn(&str) -> Option<String> {
        move |key| {
            if omit == Some(key) {
                return None;
            }
            match key {
                "SURREALDB_AUTH_LEVEL" => scope.map(str::to_string),
                "SURREALDB_HOST" => Some("ws://127.0.0.1:8001".into()),
                "SURREALDB_NAMESPACE" => Some("fixture".into()),
                "SURREALDB_NAME" => Some("app".into()),
                "SURREALDB_USER" => Some("app_runtime".into()),
                "SURREALDB_PASSWORD" => Some("credential-canary".into()),
                _ => None,
            }
        }
    }

    #[test]
    fn scoped_access_requires_complete_explicit_configuration() {
        assert!(Access::read(values(Some("database"), None)).unwrap().scope == Scope::Database);
        for key in [
            "SURREALDB_HOST",
            "SURREALDB_NAMESPACE",
            "SURREALDB_NAME",
            "SURREALDB_USER",
            "SURREALDB_PASSWORD",
        ] {
            let error = Access::read(values(Some("database"), Some(key)))
                .err()
                .unwrap()
                .to_string();
            assert!(!error.contains("credential-canary"));
        }
        for scope in ["", "Database", "namespaced", "credential-canary"] {
            let error = Access::read(values(Some(scope), None))
                .err()
                .unwrap()
                .to_string();
            assert_eq!(error, "SURREALDB_AUTH_LEVEL must be root or database.");
        }
    }

    #[test]
    fn standalone_root_defaults_are_preserved_without_weakening_database_mode() {
        assert!(Access::read(values(None, None)).unwrap().scope == Scope::Root);
        assert!(Access::read(values(Some("root"), Some("SURREALDB_NAME"))).is_ok());
        assert!(Access::read(values(Some("database"), Some("SURREALDB_NAME"))).is_err());
    }

    #[tokio::test]
    async fn a_device_library_file_opens_and_keeps_its_data() {
        let dir = std::env::temp_dir().join(format!("tawny-embedded-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        {
            let db = connect_embedded(&dir).await.unwrap();
            db.query("CREATE marker:one SET at = 1").await.unwrap();
        }
        // A dropped handle closes the file in the background; until then the
        // file is still locked.
        let mut reopened = None;
        for _ in 0..100 {
            if let Ok(db) = connect_embedded(&dir).await {
                reopened = Some(db);
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        let db = reopened.expect("the file is released once its handle is dropped");
        let mut answer = db.query("SELECT VALUE at FROM marker:one").await.unwrap();
        let at: Option<i64> = answer.take(0).unwrap();
        assert_eq!(at, Some(1));
        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
