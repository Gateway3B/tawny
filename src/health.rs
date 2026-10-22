//! The container probe. Mounted outside the session layers (see `main`), so a
//! probe never allocates a viewer session.

use axum::{Extension, http::StatusCode};

use crate::server::AppServerState;

/// `200` while SurrealDB answers, `503` when it does not.
pub async fn health(Extension(state): Extension<AppServerState>) -> StatusCode {
    match state.db.health().await {
        Ok(()) => StatusCode::OK,
        Err(_) => {
            crate::telemetry::diagnostic(crate::telemetry::Diagnostic::DatabaseUnavailable);
            StatusCode::SERVICE_UNAVAILABLE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::health;
    use crate::server::AppServerState;

    #[tokio::test]
    async fn reports_container_health() {
        let state = AppServerState::initialize().await.unwrap();
        assert_eq!(
            health(axum::Extension(state)).await,
            axum::http::StatusCode::OK
        );
    }
}
