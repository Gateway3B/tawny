//! Private legal state is always bound to the authenticated account, never to
//! the shared public catalog cache or to a client-supplied subject identifier.
use super::BrandState;
use crate::{
    app::Route,
    auth::{Session, use_session},
    state::AppState,
};
use dioxus::prelude::*;
use g3_brand::{
    Catalog,
    legal::{DocumentSet, LegalAction, LegalRequest, LegalStatus},
};
use g3_ui::{Button, ButtonFill, Card, Modal, Stack, Text, TextTone};
use serde::{Deserialize, Serialize};

#[cfg(feature = "server")]
pub mod enforcement;
#[cfg(feature = "server")]
pub mod server;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountLegalReview {
    pub owner: String,
    pub catalog: Catalog,
    pub document_set: DocumentSet,
    pub status: LegalStatus,
}

#[get("/api/v1/auth/legal-review",
    state: dioxus::fullstack::extract::State<crate::server::AppServerState>,
    owner: crate::auth::Owner
)]
async fn get_legal_review() -> Result<Option<AccountLegalReview>> {
    server::review(&state.db, &owner, false).await
}

#[post("/api/v1/auth/legal-action",
    state: dioxus::fullstack::extract::State<crate::server::AppServerState>,
    owner: crate::auth::Owner
)]
async fn record_legal_action(
    request: LegalRequest,
    expected_owner: Option<String>,
) -> Result<AccountLegalReview> {
    server::record(&state.db, &owner, expected_owner.as_deref(), request).await
}

#[derive(Clone, Copy)]
struct LegalState {
    enabled: Signal<bool>,
    review: Resource<Result<Option<AccountLegalReview>>>,
    pending: Signal<Option<PendingAction>>,
    submit: Callback<LegalRequest>,
}

#[derive(Clone)]
struct PendingAction {
    owner: String,
    generation: u64,
}
impl PendingAction {
    fn completes(&self, owner: &str, generation: u64) -> bool {
        self.owner == owner && self.generation == generation
    }
}

fn owner(session: Session) -> Option<String> {
    session
        .account
        .read()
        .as_ref()
        .map(|account| account.id.clone())
}

#[component]
pub fn LegalProvider(children: Element) -> Element {
    let app = use_context::<AppState>();
    let session = use_session();
    let brand = use_context::<BrandState>();
    let mut enabled = use_signal(|| false);
    // Keep the server and first browser tree identical, and keep this optional
    // check off the server-rendering path when the catalog is unavailable.
    use_effect(move || enabled.set(true));
    let key = use_memo(move || {
        (
            owner(session),
            brand
                .catalog
                .read()
                .as_ref()
                .map(|value| value.catalog.revision.clone()),
            (brand.now)() / 3600,
        )
    });
    let mut review = use_resource(move || {
        let key = key();
        let enabled = enabled();
        async move {
            if !enabled || key.0.is_none() {
                return Ok(None);
            }
            get_legal_review().await
        }
    });
    let mut pending = use_signal(|| None::<PendingAction>);
    let mut generation = use_signal(|| 0_u64);
    let submit = use_callback(move |request: LegalRequest| {
        let Some(target) = owner(session) else {
            return;
        };
        if pending
            .peek()
            .as_ref()
            .is_some_and(|value| value.owner == target)
        {
            return;
        }
        let matches = review
            .peek()
            .as_ref()
            .and_then(|value| value.as_ref().ok())
            .and_then(Option::as_ref)
            .is_some_and(|value| {
                value.owner == target && value.document_set.digest() == request.document_set_digest
            });
        if !matches {
            review.restart();
            app.show_toast(
                "Please review the current documents before continuing.",
                g3_ui::Color::Danger,
            );
            return;
        }
        let Some(next) = generation.peek().checked_add(1) else {
            return;
        };
        generation.set(next);
        pending.set(Some(PendingAction {
            owner: target.clone(),
            generation: next,
        }));
        spawn(async move {
            let accepted_terms = request.action == LegalAction::TermsAccepted;
            let result = record_legal_action(request, Some(target.clone())).await;
            let current = pending
                .peek()
                .as_ref()
                .is_some_and(|value| value.completes(&target, next));
            if !current {
                return;
            }
            pending.set(None);
            // A response for a previous account must never become the new
            // account's acknowledgment or produce a misleading success toast.
            if owner(session).as_deref() != Some(&target) {
                return;
            }
            match result {
                Ok(value) if value.owner == target => {
                    app.show_toast("Your acknowledgment was recorded.", g3_ui::Color::Success);
                    if accepted_terms {
                        // Reads refused by the optional gate must retry after
                        // verified acceptance, including the account's viewer.
                        g3_cache::invalidate_all_cached();
                    }
                    review.restart();
                }
                Ok(_) => {
                    app.show_toast(
                        "Your account changed. Please refresh and try again.",
                        g3_ui::Color::Danger,
                    );
                }
                Err(error) => {
                    app.show_toast(
                        crate::auth::readable(&error.to_string()),
                        g3_ui::Color::Danger,
                    );
                    review.restart();
                }
            }
        });
    });
    use_context_provider(|| LegalState {
        enabled,
        review,
        pending,
        submit,
    });
    rsx! { {children} }
}

#[component]
pub fn LegalReviewPanel() -> Element {
    let session = use_session();
    let mut legal = use_context::<LegalState>();
    let on_action = use_callback(move |request| legal.submit.call(request));
    use_effect(move || legal.review.restart());
    let brand = use_context::<BrandState>();
    let current_owner = owner(session);
    let review = legal.review.read();
    let mine = review
        .as_ref()
        .and_then(|value| value.as_ref().ok())
        .and_then(Option::as_ref)
        .filter(|value| Some(&value.owner) == current_owner.as_ref());
    let stale_owner = review
        .as_ref()
        .and_then(|value| value.as_ref().ok())
        .and_then(Option::as_ref)
        .is_some_and(|value| Some(&value.owner) != current_owner.as_ref());
    let pending = legal
        .pending
        .read()
        .as_ref()
        .is_some_and(|value| Some(&value.owner) == current_owner.as_ref());
    rsx! {
        Card { title: "Terms and privacy",
            if current_owner.is_some() {
                if !(legal.enabled)() {
                    div { role: "status", Text { "Checking your legal status…" } }
                } else if let Some(value) = mine {
                    Stack {
                        if value.status.needs_action(&value.document_set) {
                            Text { "Please review the current documents and what changed." }
                            if pending {
                                // The action is hidden while it is in flight; the
                                // pending flag also rejects repeated callbacks.
                                g3_brand::ui::LegalDocuments { catalog: value.catalog.clone(), document_set: value.document_set.clone() }
                                div { role: "status", Text { "Recording your acknowledgment…" } }
                            } else {
                                g3_brand::ui::LegalDocuments { catalog: value.catalog.clone(), document_set: value.document_set.clone(), on_action }
                            }
                        } else {
                            Text { tone: TextTone::Secondary, "Your acknowledgment of these documents is recorded for this account." }
                            g3_brand::ui::LegalDocuments { catalog: value.catalog.clone(), document_set: value.document_set.clone() }
                        }
                    }
                } else if review.as_ref().is_some_and(Result::is_err) {
                    Text { tone: TextTone::Secondary, "Your legal status couldn't be checked. Published documents can still be viewed below when available." }
                    Button { fill: ButtonFill::Clear, onclick: move |_| legal.review.restart(), "Retry legal status" }
                } else if review.is_none() || stale_owner {
                    div { role: "status", Text { "Checking your legal status…" } }
                } else {
                    Text { tone: TextTone::Secondary, "No shared document set has been published for this app. Published documents can still be viewed below when available." }
                }
            } else {
                Text { tone: TextTone::Secondary, "Connect to your Tawny server to check this account's acknowledgment." }
            }
            // A cached public document is readable help, never an account receipt.
            if mine.is_none() {
                PublicDocuments { catalog: brand.catalog }
            }
        }
    }
}

#[component]
fn PublicDocuments(catalog: ReadSignal<Option<g3_brand::client::CachedCatalog>>) -> Element {
    let cached = catalog.read();
    let catalog = cached.as_ref().map(|value| &value.catalog);
    let set = catalog
        .as_ref()
        .and_then(|value| DocumentSet::resolve(value, "tawny", "en").ok().flatten())
        .cloned();
    rsx! {
        if let (Some(catalog), Some(document_set)) = (catalog, set) {
            Text { tone: TextTone::Secondary, "Public copy for reference. Connect to check the current documents and this account's acknowledgment." }
            g3_brand::ui::LegalDocuments { catalog: catalog.clone(), document_set }
        } else {
            Text { tone: TextTone::Secondary, "Published terms and privacy information are unavailable right now. Check again later or contact the operator of your Tawny server." }
        }
    }
}

#[component]
pub fn LegalChangeNotice() -> Element {
    let route = use_route::<Route>();
    rsx! { LegalChangeNoticeForRoute { route } }
}

#[component]
fn LegalChangeNoticeForRoute(route: ReadSignal<Route>) -> Element {
    let session = use_session();
    let legal = use_context::<LegalState>();
    let update = super::use_update_policy();
    let key = use_memo(move || {
        let current_owner = owner(session)?;
        let review = legal.review.read();
        let value = review.as_ref()?.as_ref().ok()?.as_ref()?;
        (value.owner == current_owner && value.status.needs_action(&value.document_set))
            .then(|| format!("{}:{}", value.owner, value.document_set.digest()))
    });
    let mut dismissed = use_signal(|| None::<String>);
    let mut open = use_signal(|| false);
    use_effect(move || {
        let safe = matches!(route(), Route::AboutG3tech {} | Route::SettingsPage {});
        open.set(
            !safe
                && update.read().is_none()
                && key.read().is_some()
                && *key.read() != *dismissed.read(),
        );
    });
    rsx! {
        Modal { open, title: "Terms and privacy have changed",
            on_dismiss: move |_| dismissed.set(key()),
            Stack {
                Text { "Review the current documents and the summary of changes in About Tawny." }
                Button { onclick: move |_| { spawn(g3_route_transitions::animated_navigate(Route::AboutG3tech {})); }, "Review changes" }
                Button { fill: ButtonFill::Clear, onclick: move |_| { dismissed.set(key()); open.set(false); }, "Later" }
            }
        }
    }
}

#[cfg(test)]
mod account_action_tests {
    use super::PendingAction;
    #[test]
    fn older_completions_cannot_clear_another_accounts_or_newer_action() {
        let pending = PendingAction {
            owner: "account-a".into(),
            generation: 3,
        };
        assert!(!pending.completes("account-b", 3));
        assert!(!pending.completes("account-a", 1));
        assert!(pending.completes("account-a", 3));
    }
}

#[cfg(test)]
mod public_document_tests {
    use super::*;
    use crate::{app::tawny_theme, models::Appearance};
    use g3_ui::{AppWrapper, ComponentMode, Content, ContentWidth, Header};

    #[component]
    fn Preview(mode: ComponentMode, appearance: Appearance, missing: bool) -> Element {
        rsx! {
            AppWrapper { mode, theme: tawny_theme(appearance),
                Header { title: "Terms and privacy" }
                Content { width: ContentWidth::Readable,
                    Card { title: "Terms and privacy",
                        PublicDocuments { catalog: if missing { None } else { Some(g3_brand::client::CachedCatalog {
                            fetched_at: 100, etag: None, catalog: fixture_catalog(),
                        }) } }
                    }
                }
            }
        }
    }

    #[test]
    fn public_documents_inherit_the_theme_and_cannot_submit_private_actions() {
        for (mode_name, mode) in [("ios", ComponentMode::Ios), ("md", ComponentMode::Md)] {
            for (appearance_name, appearance) in
                [("light", Appearance::Light), ("dark", Appearance::Dark)]
            {
                for missing in [false, true] {
                    let mut dom = VirtualDom::new_with_props(
                        Preview,
                        PreviewProps {
                            mode,
                            appearance,
                            missing,
                        },
                    );
                    dom.rebuild_in_place();
                    let html = dioxus_ssr::render(&dom);
                    assert!(!html.contains("Accept terms"));
                    assert!(!html.contains("Acknowledge notice"));
                    assert_eq!(html.contains("Fixture only; never publish."), !missing);
                    assert_eq!(
                        html.contains("Published terms and privacy information are unavailable"),
                        missing
                    );
                    if !missing {
                        assert!(html.contains("Fixture changes"));
                        assert!(html.contains("Public copy for reference"));
                    }
                    if let Ok(directory) = std::env::var("G3_BRAND_PREVIEW_DIR") {
                        let path = std::path::Path::new(&directory);
                        std::fs::create_dir_all(path).unwrap();
                        std::fs::write(path.join(format!("legal-{mode_name}-{appearance_name}-{missing}.html")),
                            format!("<!doctype html><html><head><meta charset=utf-8><meta name=viewport content=\"width=device-width,initial-scale=1\"><link rel=stylesheet href=/g3-ui.css><link rel=stylesheet href=/app.css></head><body style=\"margin:0\">{html}</body></html>")).unwrap();
                    }
                }
            }
        }
    }
}

#[cfg(test)]
fn fixture_catalog() -> Catalog {
    use g3_brand::{
        AppLinks, AppListing,
        legal::{DocumentKind, LegalRevision},
    };
    let mut catalog = super::empty_catalog();
    let mut document = LegalRevision {
        document: "terms".into(),
        revision: "fixture-1".into(),
        locale: "en".into(),
        kind: DocumentKind::Terms,
        title: "Synthetic terms".into(),
        body: "Fixture only; never publish.".into(),
        summary: "Fixture changes".into(),
        published_at: 100,
        effective_at: 200,
        sha256: String::new(),
    };
    document.sha256 = document.content_digest();
    catalog.apps.push(AppListing {
        id: "tawny".into(),
        name: "Tawny".into(),
        description: String::new(),
        icon: None,
        order: 0,
        links: AppLinks::default(),
    });
    catalog.legal_sets.push(DocumentSet {
        app: "tawny".into(),
        revision: "fixture-1".into(),
        locale: "en".into(),
        documents: vec![document.reference()],
        requires_terms_acceptance: true,
    });
    catalog.documents.push(document);
    catalog.validate().unwrap();
    catalog
}
