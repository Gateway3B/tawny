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
    CatalogClient::new(&origin()).map(|_| ())
}

async fn catalog(force: bool) -> Result<Catalog> {
    static CACHE: OnceLock<tokio::sync::Mutex<Option<CachedCatalog>>> = OnceLock::new();
    let now = super::super::timestamp();
    let mut cached = CACHE.get_or_init(Default::default).lock().await;
    if !force
        && let Some(value) = cached.as_ref()
        && now >= value.fetched_at
        && now.saturating_sub(value.fetched_at) < 60
    {
        return Ok(value.catalog.clone());
    }
    let client = CatalogClient::new(&origin()).map_err(error)?;
    let value = client
        .refresh(cached.as_ref(), now)
        .await
        .map_err(|_| error("Shared legal documents are unavailable. Please try again."))?;
    let result = value.catalog.clone();
    *cached = Some(value);
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

async fn review_for(
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

async fn commit(
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
