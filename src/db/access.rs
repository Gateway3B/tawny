//! Explicit authentication scope, without retrying scoped failures as root.
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
    #[ignore = "requires ci/privacy_auth_drill.py's owned disposable database"]
    async fn database_editor_can_erase_without_access_to_other_databases_over_websocket() {
        use axum_session::DatabasePool;
        use std::sync::Arc;
        let endpoint = std::env::var("TAWNY_PRIVACY_TEST_WS_URL").unwrap();
        let url = reqwest::Url::parse(&endpoint).unwrap();
        assert_eq!(url.scheme(), "ws");
        assert_eq!(url.host_str(), Some("127.0.0.1"));
        assert!(url.port().is_some());
        assert!(url.username().is_empty() && url.password().is_none());
        assert!(url.query().is_none() && url.fragment().is_none() && url.path() == "/");
        let root = Access {
            endpoint: endpoint.clone(),
            namespace: "tawny_privacy_fixture".into(),
            database: "app".into(),
            username: "fixture-root".into(),
            password: "synthetic-privacy-fixture-password".into(),
            scope: Scope::Root,
        };
        let provision = surrealdb::engine::any::connect(root.endpoint.clone())
            .await
            .unwrap();
        provision
            .signin(surrealdb::opt::auth::Root {
                username: root.username.clone(),
                password: root.password.clone(),
            })
            .await
            .unwrap();
        // Check namespace creation before selecting it: selection can create it.
        provision
            .query("DEFINE NAMESPACE tawny_privacy_fixture;")
            .await
            .unwrap()
            .check()
            .unwrap();
        provision.use_ns("tawny_privacy_fixture").await.unwrap();
        provision
            .query("DEFINE DATABASE app;")
            .await
            .unwrap()
            .check()
            .unwrap();
        provision.use_db("app").await.unwrap();
        provision.query("DEFINE USER fixture_writer ON DATABASE PASSWORD 'synthetic-database-writer-password' ROLES EDITOR;").await.unwrap().check().unwrap();
        let setup = root.open().await.unwrap();
        crate::db::sync_schema(&setup).await.unwrap();
        setup.use_db("other").await.unwrap();
        setup
            .query("CREATE private_fixture:kept SET secret = 'other-database-canary';")
            .await
            .unwrap()
            .check()
            .unwrap();
        setup.use_db("app").await.unwrap();
        let scoped = Access {
            endpoint,
            namespace: root.namespace.clone(),
            database: root.database.clone(),
            username: "fixture_writer".into(),
            password: "synthetic-database-writer-password".into(),
            scope: Scope::Database,
        };
        let db = Arc::new(scoped.open().await.unwrap());
        // Startup sync is currently authoritative. Verify it really works under
        // the runtime role rather than assuming the memory engine's privileges.
        crate::db::sync_schema(&db).await.unwrap();
        crate::privacy::sessions::prepare(&db).await.unwrap();
        let accounts = crate::auth::Accounts::new(&db);
        let owner = crate::auth::Owner(accounts.create_guest().await.unwrap().id);
        let other = crate::auth::Owner(accounts.create_guest().await.unwrap().id);
        // Exercise the non-empty legacy repair as well as the empty startup.
        db.query("REMOVE INDEX user_subscription_owner_channel_idx ON user_subscription; CREATE user_subscription:wire_old SET owner = type::record($owner), channel_id = 'fixture', updated_at = <datetime>'2026-01-01T00:00:00Z'; CREATE user_subscription:wire_new SET owner = type::record($owner), channel_id = 'fixture', updated_at = <datetime>'2026-01-02T00:00:00Z';")
            .bind(("owner", owner.0.clone())).await.unwrap().check().unwrap();
        db.query(include_str!("../../database/presync.surql"))
            .await
            .unwrap()
            .check()
            .unwrap();
        let surviving: Vec<surrealdb_types::RecordId> = db
            .query("SELECT VALUE id FROM user_subscription WHERE channel_id = 'fixture'")
            .await
            .unwrap()
            .check()
            .unwrap()
            .take(0)
            .unwrap();
        assert_eq!(
            surviving,
            [surrealdb_types::RecordId::new(
                "user_subscription",
                "wire_new"
            )]
        );
        db.query(include_str!(
            "../../database/schema/user_subscription.surql"
        ))
        .await
        .unwrap()
        .check()
        .unwrap();
        crate::db::sync_schema(&db).await.unwrap();
        for account in [&owner, &other] {
            db.query("CREATE playlist SET owner = type::record($owner), playlist_id = 'private', name = 'Private'; CREATE library_state SET owner = type::record($owner); CREATE brand_legal_receipt SET app = 'tawny', owner = type::record($owner), subject = 'synthetic', document_set_digest = 'synthetic', action = 'acknowledged', purpose = '', created_at = time::now();")
                .bind(("owner", account.0.clone())).await.unwrap().check().unwrap();
        }
        let pool = crate::auth::session_pool(Arc::clone(&db));
        let payload = serde_json::json!({"data":{"user_auth_session_id":serde_json::to_string(owner.0.strip_prefix("app_user:").unwrap()).unwrap()}}).to_string();
        pool.store(
            "wire-session",
            &payload,
            time::OffsetDateTime::now_utc().unix_timestamp() + 3600,
            "sessions",
        )
        .await
        .unwrap();
        assert!(
            pool.load("wire-session", "sessions")
                .await
                .unwrap()
                .is_some()
        );
        accounts.delete(&owner, Some(&owner.0)).await.unwrap();
        assert!(accounts.find(&owner.0).await.is_err());
        assert!(accounts.find(&other.0).await.is_ok());
        assert!(
            pool.load("wire-session", "sessions")
                .await
                .unwrap()
                .is_none()
        );
        pool.store(
            "delayed-wire",
            &payload,
            time::OffsetDateTime::now_utc().unix_timestamp() + 3600,
            "sessions",
        )
        .await
        .unwrap();
        assert!(
            pool.load("delayed-wire", "sessions")
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            db.query("CREATE library_state SET owner = type::record($owner)")
                .bind(("owner", owner.0.clone()))
                .await
                .unwrap()
                .check()
                .is_err()
        );
        let remaining: Vec<surrealdb_types::RecordId> = db
            .query("SELECT VALUE id FROM brand_legal_receipt WHERE owner = type::record($owner)")
            .bind(("owner", owner.0.clone()))
            .await
            .unwrap()
            .check()
            .unwrap()
            .take(0)
            .unwrap();
        assert!(remaining.is_empty());
        assert!(
            db.query("DEFINE USER escalation_probe ON ROOT PASSWORD 'synthetic' ROLES OWNER;")
                .await
                .unwrap()
                .check()
                .is_err()
        );
        crate::privacy::recovery::exercise_editor_queue(Arc::clone(&db)).await;
        // Permission-denied reads can return an empty result rather than a
        // transport error. Check the actual data and attempted write effects.
        if db.use_db("other").await.is_ok() {
            if let Ok(mut response) = db
                .query("SELECT VALUE secret FROM private_fixture")
                .await
                .and_then(|response| response.check())
            {
                assert!(response.take::<Vec<String>>(0).unwrap().is_empty());
            }
            let _ = db.query("UPDATE private_fixture:kept SET secret = 'cross-database-write-canary'; CREATE private_fixture:injected SET secret = 'cross-database-create-canary';")
                .await.and_then(|response| response.check());
        }
        setup.use_db("other").await.unwrap();
        let secret: Vec<String> = setup
            .query("SELECT VALUE secret FROM private_fixture")
            .await
            .unwrap()
            .check()
            .unwrap()
            .take(0)
            .unwrap();
        assert_eq!(secret, ["other-database-canary"]);
        let root_in_database_mode = Access {
            scope: Scope::Database,
            ..root
        };
        assert_eq!(
            root_in_database_mode
                .open()
                .await
                .err()
                .unwrap()
                .to_string(),
            "Database-scoped authentication failed."
        );
        let mut invalid = scoped;
        invalid.password = "wrong-password-canary".into();
        let error = invalid.open().await.err().unwrap().to_string();
        assert_eq!(error, "Database-scoped authentication failed.");
        assert!(!error.contains("wrong-password-canary"));
    }
}
