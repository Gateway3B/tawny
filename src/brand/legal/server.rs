//! Private receipts live in this Tawny server's database. The public catalog
//! never receives the account, its status, or an acknowledgment.
use super::AccountLegalReview;
use crate::auth::Owner;
use dioxus::prelude::*;
use g3_brand::{
    Catalog,
    client::{CachedCatalog, CatalogClient},
    legal::{DocumentSet, LegalAction, LegalRequest, LegalStatus},
};
use std::sync::OnceLock;
use surrealdb::{Surreal, engine::any::Any};
use surrealdb_types::SurrealValue;

const APP: &str = "tawny";

fn error(message: impl std::fmt::Display) -> dioxus::CapturedError {
    ServerFnError::new(message.to_string()).into()
}

fn origin() -> String {
    std::env::var("G3_CATALOG_ORIGIN").unwrap_or_else(|_| super::super::ORIGIN.into())
}

pub fn validate_configuration() -> std::result::Result<(), String> {
    super::enforcement::validate_configuration().map_err(str::to_owned)?;
    CatalogClient::new(&origin()).map(|_| ())
}

async fn catalog(force: bool) -> Result<Catalog> {
    static CACHE: OnceLock<tokio::sync::Mutex<PublicCache>> = OnceLock::new();
    let mut cached = CACHE.get_or_init(Default::default).lock().await;
    let client = CatalogClient::new(&origin()).map_err(error)?;
    load_catalog(&mut cached, &client, force, super::super::timestamp).await
}

#[derive(Default)]
struct PublicCache {
    snapshot: Option<CachedCatalog>,
    failed_at: Option<i64>,
}

async fn load_catalog(
    cached: &mut PublicCache,
    client: &CatalogClient,
    force: bool,
    clock: impl Fn() -> i64,
) -> Result<Catalog> {
    let now = clock();
    let unavailable = || error("Shared legal documents are unavailable. Please try again.");
    if cached
        .failed_at
        .is_some_and(|at| now >= at && now.saturating_sub(at) < 10)
    {
        return Err(unavailable());
    }
    if !force
        && let Some(value) = cached.snapshot.as_ref()
        && now >= value.fetched_at
        && now.saturating_sub(value.fetched_at) < 60
    {
        return Ok(value.catalog.clone());
    }
    let value = match client.refresh(cached.snapshot.as_ref(), now).await {
        Ok(value) => value,
        Err(_) => {
            // Stamp the end of the bounded fetch so a timeout cannot consume
            // the whole cooldown and immediately trigger another slow request.
            cached.failed_at = Some(clock());
            return Err(unavailable());
        }
    };
    let result = value.catalog.clone();
    cached.snapshot = Some(value);
    cached.failed_at = None;
    Ok(result)
}

fn subject(owner: &Owner) -> String {
    g3_brand::model::digest(owner.0.as_bytes())
}

#[derive(serde::Deserialize, SurrealValue)]
struct StatusRow {
    document_set_digest: String,
    acknowledged: bool,
    terms_accepted: bool,
}

async fn status(db: &Surreal<Any>, owner: &Owner, set: &DocumentSet) -> Result<LegalStatus> {
    let row = db
        .query(g3_brand::backend::LEGAL_STATUS_QUERY)
        .bind(("app", APP))
        .bind(("subject", subject(owner)))
        .bind(("digest", set.digest()))
        .await
        .map_err(|_| error("Couldn't check your legal status."))?
        .check()
        .map_err(|_| error("Couldn't check your legal status."))?
        .take::<Option<StatusRow>>(0)
        .map_err(|_| error("Couldn't check your legal status."))?
        .ok_or_else(|| error("Couldn't check your legal status."))?;
    Ok(LegalStatus {
        document_set_digest: row.document_set_digest,
        acknowledged: row.acknowledged,
        terms_accepted: row.terms_accepted,
    })
}

pub async fn review(
    db: &Surreal<Any>,
    owner: &Owner,
    force: bool,
) -> Result<Option<AccountLegalReview>> {
    review_for(db, owner, catalog(force).await?).await
}

pub(super) async fn review_for(
    db: &Surreal<Any>,
    owner: &Owner,
    catalog: Catalog,
) -> Result<Option<AccountLegalReview>> {
    catalog.validate().map_err(error)?;
    let Some(set) = DocumentSet::resolve(&catalog, APP, "en")
        .map_err(error)?
        .cloned()
    else {
        return Ok(None);
    };
    crate::auth::Accounts::new(db)
        .find(&owner.0)
        .await
        .map_err(|_| error("Your account changed. Please sign in again."))?;
    let status = status(db, owner, &set).await?;
    Ok(Some(AccountLegalReview {
        owner: owner.0.clone(),
        catalog,
        document_set: set,
        status,
    }))
}

pub async fn record(
    db: &Surreal<Any>,
    owner: &Owner,
    expected_owner: Option<&str>,
    request: LegalRequest,
) -> Result<AccountLegalReview> {
    // Check before fetching documents or touching a receipt table.
    owner.require_reviewed(expected_owner).map_err(error)?;
    let current = review(db, owner, true)
        .await?
        .ok_or_else(|| error("No shared documents have been published for Tawny."))?;
    commit(db, owner, request, current).await
}

// Touch the same account row that deletion removes. A concurrent deletion
// conflicts with this transaction, so a late receipt cannot survive erasure.
const COMMIT: &str = r#"
BEGIN TRANSACTION;
LET $active = (UPDATE type::record($owner) SET updated_at = time::now() RETURN VALUE id);
IF array::len($active) != 1 { THROW 'Account changed during legal review'; };
CREATE brand_legal_receipt CONTENT {
    app: $app, owner: type::record($owner), subject: $subject,
    document_set_digest: $digest, action: $action, purpose: '', created_at: time::now()
};
COMMIT TRANSACTION;
"#;

pub(super) async fn commit(
    db: &Surreal<Any>,
    owner: &Owner,
    request: LegalRequest,
    mut review: AccountLegalReview,
) -> Result<AccountLegalReview> {
    if review.owner != owner.0 || review.document_set.app != APP {
        return Err(error("Your account changed. Review the documents again."));
    }
    // Purposes need a reviewed processing inventory. Terms and notice actions
    // never authorize unrelated processing.
    if !matches!(
        request.action,
        LegalAction::Acknowledged | LegalAction::TermsAccepted
    ) || request.purpose.is_some()
    {
        return Err(error("This action doesn't grant processing consent."));
    }
    review.catalog.validate().map_err(error)?;
    if DocumentSet::resolve(&review.catalog, APP, "en").map_err(error)?
        != Some(&review.document_set)
    {
        return Err(error("Review the current documents before continuing."));
    }
    request.validate(&review.document_set).map_err(error)?;
    db.query(COMMIT)
        .bind(("owner", owner.0.clone()))
        .bind(("app", APP))
        .bind(("subject", subject(owner)))
        .bind(("digest", review.document_set.digest()))
        .bind(("action", request.action.as_str()))
        .await
        .map_err(|_| error("Couldn't record your acknowledgment. Please review again."))?
        .check()
        .map_err(|_| error("Couldn't record your acknowledgment. Please review again."))?;
    review.status = status(db, owner, &review.document_set).await?;
    let saved = match request.action {
        LegalAction::Acknowledged => review.status.acknowledged,
        LegalAction::TermsAccepted => review.status.terms_accepted,
        _ => false,
    };
    if !saved {
        return Err(error(
            "Your acknowledgment couldn't be verified. Please retry.",
        ));
    }
    Ok(review)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn catalog_expiry_outages_and_recovery_never_authorize_a_stale_snapshot() {
        use axum::{Router, http::StatusCode, response::IntoResponse, routing::get};
        use std::sync::{
            Arc,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        };
        let available = Arc::new(AtomicBool::new(true));
        let requests = Arc::new(AtomicUsize::new(0));
        let fixture = super::super::fixture_catalog();
        let response_catalog = fixture.clone();
        let healthy = Arc::clone(&available);
        let count = Arc::clone(&requests);
        let app = Router::new().route(
            "/api/v1/catalog",
            get(move || {
                let fixture = response_catalog.clone();
                let healthy = Arc::clone(&healthy);
                let count = Arc::clone(&count);
                async move {
                    count.fetch_add(1, Ordering::SeqCst);
                    if healthy.load(Ordering::SeqCst) {
                        axum::Json(fixture).into_response()
                    } else {
                        StatusCode::SERVICE_UNAVAILABLE.into_response()
                    }
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client =
            CatalogClient::new(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let mut cache = PublicCache::default();
        assert_eq!(
            load_catalog(&mut cache, &client, false, || 100)
                .await
                .unwrap(),
            fixture
        );
        assert_eq!(
            load_catalog(&mut cache, &client, false, || 159)
                .await
                .unwrap(),
            fixture
        );
        assert_eq!(requests.load(Ordering::SeqCst), 1);
        available.store(false, Ordering::SeqCst);
        assert!(
            load_catalog(&mut cache, &client, false, || 160)
                .await
                .is_err()
        );
        for force in [false, true] {
            assert!(
                load_catalog(&mut cache, &client, force, || 169)
                    .await
                    .is_err()
            );
        }
        assert_eq!(requests.load(Ordering::SeqCst), 2);
        assert!(
            load_catalog(&mut cache, &client, false, || 170)
                .await
                .is_err()
        );
        assert_eq!(requests.load(Ordering::SeqCst), 3);
        available.store(true, Ordering::SeqCst);
        assert!(
            load_catalog(&mut cache, &client, false, || 179)
                .await
                .is_err()
        );
        assert_eq!(
            load_catalog(&mut cache, &client, false, || 180)
                .await
                .unwrap(),
            fixture
        );
        assert_eq!(requests.load(Ordering::SeqCst), 4);
        // A backwards clock must revalidate instead of extending cache age.
        load_catalog(&mut cache, &client, false, || 179)
            .await
            .unwrap();
        assert_eq!(requests.load(Ordering::SeqCst), 5);
        load_catalog(&mut cache, &client, true, || 180)
            .await
            .unwrap();
        assert_eq!(requests.load(Ordering::SeqCst), 6);
        server.abort();
    }

    fn request(review: &AccountLegalReview, action: LegalAction) -> LegalRequest {
        LegalRequest {
            document_set_digest: review.document_set.digest(),
            action,
            purpose: None,
        }
    }

    #[tokio::test]
    async fn a_changed_or_missing_review_binding_is_refused_before_network_or_database_work() {
        let db = Surreal::<Any>::init();
        for expected in [None, Some("app_user:other")] {
            let outcome = tokio::time::timeout(
                std::time::Duration::from_secs(1),
                record(
                    &db,
                    &Owner("app_user:current".into()),
                    expected,
                    LegalRequest {
                        document_set_digest: "synthetic".into(),
                        action: LegalAction::Acknowledged,
                        purpose: None,
                    },
                ),
            )
            .await
            .unwrap();
            assert!(outcome.is_err());
        }
    }

    #[tokio::test]
    async fn only_current_account_and_set_receive_the_verified_acknowledgment() {
        let state = crate::server::AppServerState::initialize().await.unwrap();
        let first = Owner(state.accounts().create_guest().await.unwrap().id);
        let second = Owner(state.accounts().create_guest().await.unwrap().id);
        let review = review_for(&state.db, &first, super::super::fixture_catalog())
            .await
            .unwrap()
            .unwrap();
        let acknowledged = commit(
            &state.db,
            &first,
            request(&review, LegalAction::Acknowledged),
            review.clone(),
        )
        .await
        .unwrap();
        assert!(acknowledged.status.acknowledged);
        assert!(!acknowledged.status.terms_accepted);
        assert!(acknowledged.status.needs_action(&review.document_set));
        assert!(
            commit(
                &state.db,
                &second,
                request(&review, LegalAction::TermsAccepted),
                review.clone()
            )
            .await
            .is_err()
        );
        let other = review_for(&state.db, &second, super::super::fixture_catalog())
            .await
            .unwrap()
            .unwrap();
        assert!(!other.status.acknowledged);
        let mut changed = review.clone();
        changed.document_set.revision = "fixture-2".into();
        assert!(
            commit(
                &state.db,
                &first,
                request(&review, LegalAction::TermsAccepted),
                changed
            )
            .await
            .is_err()
        );
        let mut consent = request(&review, LegalAction::ConsentGranted);
        consent.purpose = Some("unreviewed-purpose".into());
        assert!(
            commit(&state.db, &first, consent, review.clone())
                .await
                .is_err()
        );
        let accepted = commit(
            &state.db,
            &first,
            request(&review, LegalAction::TermsAccepted),
            review,
        )
        .await
        .unwrap();
        assert!(!accepted.status.needs_action(&accepted.document_set));
    }

    #[tokio::test]
    async fn deletion_erases_only_its_receipts_and_late_acceptance_cannot_recreate_them() {
        let state = crate::server::AppServerState::initialize().await.unwrap();
        let first = Owner(state.accounts().create_guest().await.unwrap().id);
        let second = Owner(state.accounts().create_guest().await.unwrap().id);
        let review = review_for(&state.db, &first, super::super::fixture_catalog())
            .await
            .unwrap()
            .unwrap();
        let other = review_for(&state.db, &second, super::super::fixture_catalog())
            .await
            .unwrap()
            .unwrap();
        commit(
            &state.db,
            &first,
            request(&review, LegalAction::TermsAccepted),
            review.clone(),
        )
        .await
        .unwrap();
        commit(
            &state.db,
            &second,
            request(&other, LegalAction::TermsAccepted),
            other.clone(),
        )
        .await
        .unwrap();
        state
            .accounts()
            .delete(&first, Some(&first.0))
            .await
            .unwrap();
        assert!(
            commit(
                &state.db,
                &first,
                request(&review, LegalAction::TermsAccepted),
                review.clone()
            )
            .await
            .is_err()
        );
        assert!(
            !status(&state.db, &first, &review.document_set)
                .await
                .unwrap()
                .acknowledged
        );
        assert!(
            status(&state.db, &second, &other.document_set)
                .await
                .unwrap()
                .terms_accepted
        );
        assert!(state.accounts().find(&first.0).await.is_err());
    }

    #[tokio::test]
    async fn concurrent_deletion_and_receipt_write_do_not_leave_orphaned_evidence() {
        let state = crate::server::AppServerState::initialize().await.unwrap();
        for _ in 0..12 {
            let owner = Owner(state.accounts().create_guest().await.unwrap().id);
            let review = review_for(&state.db, &owner, super::super::fixture_catalog())
                .await
                .unwrap()
                .unwrap();
            let accounts = state.accounts();
            let (_, removed) = tokio::join!(
                commit(
                    &state.db,
                    &owner,
                    request(&review, LegalAction::TermsAccepted),
                    review.clone()
                ),
                accounts.delete(&owner, Some(&owner.0)),
            );
            if removed.is_err() {
                accounts.delete(&owner, Some(&owner.0)).await.unwrap();
            }
            assert!(accounts.find(&owner.0).await.is_err());
            assert!(
                !status(&state.db, &owner, &review.document_set)
                    .await
                    .unwrap()
                    .acknowledged
            );
        }
    }
}
