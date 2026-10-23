//! Optional server-side effective-terms checks. Essential account controls
//! remain accessible even when documents or private verification are unavailable.
use super::AccountLegalReview;
use crate::auth::{AuthSession, Owner};
use axum::{
    extract::Request,
    http::{StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::sync::Arc;
use surrealdb::{Surreal, engine::any::Any};

fn required_mode(value: Option<&str>) -> Result<bool, &'static str> {
    match value {
        None | Some("disabled") => Ok(false),
        Some("required") => Ok(true),
        _ => Err("Invalid legal enforcement mode"),
    }
}

pub fn validate_configuration() -> Result<(), &'static str> {
    required_mode(std::env::var("G3_LEGAL_ENFORCEMENT").ok().as_deref()).map(|_| ())
}

fn protected(path: &str) -> bool {
    path.starts_with("/api/")
        && !g3_auth::is_public_endpoint(path)
        && !matches!(
            path,
            "/api/v1/auth/account"
                | "/api/v1/auth/sign-out"
                | "/api/v1/auth/delete"
                | "/api/v1/auth/export"
                | "/api/v1/auth/legal-review"
                | "/api/v1/auth/legal-action"
        )
}

fn failure(status: StatusCode, message: &str) -> Response {
    (
        status,
        [
            (header::CONTENT_TYPE, "application/json"),
            (header::CACHE_CONTROL, "private, no-store"),
        ],
        serde_json::json!({"message":message,"code":status.as_u16()}).to_string(),
    )
        .into_response()
}

fn needs_effective_terms(review: &AccountLegalReview, now: i64) -> Result<bool, String> {
    let effective = review
        .document_set
        .effective_at(&review.catalog.documents)?;
    Ok(effective <= now
        && review.document_set.requires_terms_acceptance
        && review.status.needs_action(&review.document_set))
}

pub async fn enforce_terms(request: Request, next: Next) -> Response {
    let mode = required_mode(std::env::var("G3_LEGAL_ENFORCEMENT").ok().as_deref());
    enforce_configured(mode, request, next, |db, owner| async move {
        super::server::review(&db, &owner, false).await
    })
    .await
}

async fn enforce_configured<F, Fut>(
    mode: Result<bool, &'static str>,
    request: Request,
    next: Next,
    load: F,
) -> Response
where
    F: FnOnce(Arc<Surreal<Any>>, Owner) -> Fut,
    Fut: std::future::Future<Output = dioxus::prelude::Result<Option<AccountLegalReview>>>,
{
    if mode == Ok(false) || !protected(request.uri().path()) {
        return next.run(request).await;
    }
    if mode.is_err() {
        return failure(
            StatusCode::SERVICE_UNAVAILABLE,
            "Legal configuration is unavailable.",
        );
    }
    let Some(session) = request.extensions().get::<AuthSession>() else {
        return failure(
            StatusCode::SERVICE_UNAVAILABLE,
            "Session verification is unavailable.",
        );
    };
    let Some(user) = session.current_user.as_ref().filter(|user| !user.anonymous) else {
        // Let the host's normal guard decide signed-out requests. A guest
        // account has a real session and must review terms like any other user.
        return next.run(request).await;
    };
    let owner = Owner(format!("app_user:{}", user.id));
    let Some(db) = request.extensions().get::<Arc<Surreal<Any>>>().cloned() else {
        return failure(
            StatusCode::SERVICE_UNAVAILABLE,
            "Legal verification is unavailable.",
        );
    };
    match load(db, Owner(owner.0.clone())).await {
        Ok(Some(review)) if review.owner == owner.0 => {
            match needs_effective_terms(&review, super::super::timestamp()) {
                Ok(false) => next.run(request).await,
                Ok(true) => failure(
                    StatusCode::FORBIDDEN,
                    "Please review the current terms in About g3tech before continuing.",
                ),
                Err(_) => failure(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "The current terms couldn't be verified.",
                ),
            }
        }
        _ => failure(
            StatusCode::SERVICE_UNAVAILABLE,
            "The current terms couldn't be verified. Please try again.",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Extension, Router,
        body::Body,
        extract::Path,
        middleware::{from_fn, from_fn_with_state},
        routing::get,
    };
    use dioxus::prelude::ServerFnError;
    use g3_brand::legal::{LegalAction, LegalRequest, LegalStatus};
    use tower::ServiceExt;

    fn review() -> AccountLegalReview {
        let catalog = super::super::fixture_catalog();
        let document_set = catalog.legal_sets[0].clone();
        AccountLegalReview {
            owner: "app_user:fixture".into(),
            status: LegalStatus {
                document_set_digest: document_set.digest(),
                acknowledged: false,
                terms_accepted: false,
            },
            document_set,
            catalog,
        }
    }

    #[test]
    fn effective_terms_require_current_acceptance_without_inventing_consent() {
        let mut review = review();
        assert!(!needs_effective_terms(&review, 199).unwrap());
        assert!(needs_effective_terms(&review, 200).unwrap());
        review.status.acknowledged = true;
        assert!(needs_effective_terms(&review, 200).unwrap());
        review.status.terms_accepted = true;
        assert!(!needs_effective_terms(&review, 200).unwrap());
        review.status.document_set_digest = "old-set".into();
        assert!(needs_effective_terms(&review, 200).unwrap());
        review.document_set.requires_terms_acceptance = false;
        assert!(!needs_effective_terms(&review, 200).unwrap());
        review.catalog.documents.clear();
        assert!(needs_effective_terms(&review, 200).is_err());
    }

    #[test]
    fn essential_controls_remain_accessible_and_new_api_paths_are_protected() {
        for path in [
            "/about-g3tech",
            "/account",
            "/api/v1/auth/guest",
            "/api/v1/auth/sign-in",
            "/api/v1/auth/sign-out",
            "/api/v1/auth/account",
            "/api/v1/auth/delete",
            "/api/v1/auth/export",
            "/api/v1/auth/legal-review",
            "/api/v1/auth/legal-action",
        ] {
            assert!(!protected(path), "{path}");
        }
        for path in [
            "/api/v1/auth/register",
            "/api/v1/auth/future",
            "/api/v1/library",
            "/api/v1/future",
        ] {
            assert!(protected(path), "{path}");
        }
        assert_eq!(required_mode(None), Ok(false));
        assert_eq!(required_mode(Some("disabled")), Ok(false));
        assert_eq!(required_mode(Some("required")), Ok(true));
        assert!(required_mode(Some("typo")).is_err());
    }

    async fn send(app: &Router, path: &str, cookie: Option<&str>) -> Response {
        let mut request = Request::builder().uri(path);
        if let Some(cookie) = cookie {
            request = request.header(header::COOKIE, cookie);
        }
        app.clone()
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn invalid_or_missing_verification_fails_closed_but_never_blocks_exports() {
        for mode in [Ok(false), Ok(true), Err("invalid")] {
            let app = Router::new()
                .route("/api/v1/private", get(|| async { "protected action" }))
                .route("/api/v1/auth/export", get(|| async { "essential control" }))
                .layer(from_fn(move |request, next| {
                    enforce_configured(mode, request, next, |_, _| async {
                        panic!("Bypassed requests must not load legal data")
                    })
                }));
            let response = send(&app, "/api/v1/private", None).await;
            assert_eq!(
                response.status(),
                if mode == Ok(false) {
                    StatusCode::OK
                } else {
                    StatusCode::SERVICE_UNAVAILABLE
                }
            );
            if mode != Ok(false) {
                assert_eq!(
                    response.headers()[header::CACHE_CONTROL],
                    "private, no-store"
                );
            }
            assert_eq!(
                send(&app, "/api/v1/auth/export", None).await.status(),
                StatusCode::OK
            );
        }
    }

    #[tokio::test]
    async fn actual_sessions_and_receipts_isolate_accounts_revisions_and_essential_access() {
        use axum_session::{SessionConfig, SessionLayer, SessionStore};
        use axum_session_auth::AuthConfig;
        use g3_auth::{AuthGuard, AuthSessionLayer, require_session};
        let state = crate::server::AppServerState::initialize().await.unwrap();
        let first = Owner(state.accounts().create_guest().await.unwrap().id);
        let other = Owner(state.accounts().create_guest().await.unwrap().id);
        let db = Arc::clone(&state.db);
        let original = super::super::fixture_catalog();
        let published = Arc::new(tokio::sync::RwLock::new(Some(original.clone())));
        let source = Arc::clone(&published);
        let store = SessionStore::new(
            Some(crate::auth::session_pool(Arc::clone(&db))),
            SessionConfig::default()
                .with_cookie_path("/")
                .with_secure(false),
        )
        .await
        .unwrap();
        async fn login(Path(key): Path<String>, session: AuthSession) -> &'static str {
            session.login_user(key);
            session.remember_user(true);
            "synthetic login"
        }
        let app = Router::new()
            .route("/api/v1/private", get(|| async { "protected action" }))
            .route("/api/v1/auth/export", get(|| async { "essential control" }))
            .route("/api/v1/auth/delete", get(|| async { "essential control" }))
            .route(
                "/api/v1/auth/legal-review",
                get(|| async { "essential control" }),
            )
            .route(
                "/api/v1/auth/legal-action",
                get(|| async { "essential control" }),
            )
            .route(
                "/api/v1/auth/account",
                get(|| async { "essential control" }),
            )
            .route(
                "/api/v1/auth/sign-out",
                get(|| async { "essential control" }),
            )
            .layer(from_fn(move |request, next| {
                let source = Arc::clone(&source);
                enforce_configured(Ok(true), request, next, move |db, owner| async move {
                    let catalog = source
                        .read()
                        .await
                        .clone()
                        .ok_or_else(|| ServerFnError::new("Synthetic outage"))?;
                    super::super::server::review_for(&db, &owner, catalog).await
                })
            }))
            .layer(Extension(Arc::clone(&db)))
            .layer(from_fn_with_state(
                AuthGuard::for_routes(crate::app::Route::Feed {}),
                require_session::<crate::auth::AppUser, Any>,
            ))
            .merge(Router::new().route("/fixture-login/{key}", get(login)))
            .layer(
                AuthSessionLayer::<crate::auth::AppUser, Any>::new(Some(Arc::clone(&db)))
                    .with_config(AuthConfig::<String>::default().set_cache(false)),
            )
            .layer(SessionLayer::new(store));
        let login = send(
            &app,
            &format!(
                "/fixture-login/{}",
                first.0.strip_prefix("app_user:").unwrap()
            ),
            None,
        )
        .await;
        let cookie = login
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .map(|value| value.to_str().unwrap().split(';').next().unwrap())
            .collect::<Vec<_>>()
            .join("; ");
        assert!(!cookie.is_empty());
        assert_eq!(
            send(&app, "/api/v1/private", None).await.status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            send(&app, "/api/v1/private", Some(&cookie)).await.status(),
            StatusCode::FORBIDDEN
        );
        let accept = |catalog: &g3_brand::Catalog, action| LegalRequest {
            document_set_digest: catalog.legal_sets[0].digest(),
            action,
            purpose: None,
        };
        for (owner, action) in [
            (&other, LegalAction::TermsAccepted),
            (&first, LegalAction::Acknowledged),
        ] {
            let review = super::super::server::review_for(&db, owner, original.clone())
                .await
                .unwrap()
                .unwrap();
            super::super::server::commit(&db, owner, accept(&original, action), review)
                .await
                .unwrap();
        }
        assert_eq!(
            send(&app, "/api/v1/private", Some(&cookie)).await.status(),
            StatusCode::FORBIDDEN
        );
        let review = super::super::server::review_for(&db, &first, original.clone())
            .await
            .unwrap()
            .unwrap();
        super::super::server::commit(
            &db,
            &first,
            accept(&original, LegalAction::TermsAccepted),
            review,
        )
        .await
        .unwrap();
        assert_eq!(
            send(&app, "/api/v1/private", Some(&cookie)).await.status(),
            StatusCode::OK
        );
        let mut replacement = original.clone();
        replacement.legal_sets[0].revision = "new-set".into();
        *published.write().await = Some(replacement);
        let response = send(&app, "/api/v1/private", Some(&cookie)).await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            response.headers()[header::CACHE_CONTROL],
            "private, no-store"
        );
        *published.write().await = None;
        let response = send(&app, "/api/v1/private", Some(&cookie)).await;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            response.headers()[header::CACHE_CONTROL],
            "private, no-store"
        );
        for path in [
            "export",
            "delete",
            "legal-review",
            "legal-action",
            "account",
            "sign-out",
        ] {
            assert_eq!(
                send(&app, &format!("/api/v1/auth/{path}"), Some(&cookie))
                    .await
                    .status(),
                StatusCode::OK
            );
        }
        state
            .accounts()
            .delete(&first, Some(&first.0))
            .await
            .unwrap();
        assert_eq!(
            send(&app, "/api/v1/private", Some(&cookie)).await.status(),
            StatusCode::UNAUTHORIZED
        );
    }
}
