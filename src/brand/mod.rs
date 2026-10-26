//! Public catalog state belongs to the installation, never to a shared identity.
//! Only validated public content and optional notice dismissals are persisted.
use crate::app::Route;
mod store;
use dioxus::prelude::*;
use g3_brand::{
    AppIdentity, Catalog, Channel, Platform,
    client::{CachedCatalog, CatalogClient},
    privacy::Dismissals,
    updates::{ReleasePolicy, UpdateDecision},
};
use g3_ui::{Button, ButtonFill, Modal};

const CACHE_KEY: &str = "g3tech:tawny:catalog:v1";
const DISMISSALS_KEY: &str = "g3tech:tawny:updates:v1";
pub(crate) const ORIGIN: &str = match option_env!("G3_CATALOG_ORIGIN") {
    Some(value) => value,
    None => "https://catalog.g3tech.net",
};

#[derive(Clone, Copy)]
pub struct BrandState {
    pub catalog: Signal<Option<CachedCatalog>>,
    pub identity: Signal<AppIdentity>,
    pub busy: Signal<bool>,
    pub offline: Signal<bool>,
    pub ready: Signal<bool>,
    pub now: Signal<i64>,
    dismissals: Signal<Dismissals>,
    last_attempt: Signal<Option<i64>>,
}

pub(crate) fn timestamp() -> i64 {
    web_time::SystemTime::now()
        .duration_since(web_time::UNIX_EPOCH)
        .map(|value| value.as_secs().min(i64::MAX as u64) as i64)
        .unwrap_or(0)
}

fn channel(value: Option<&str>) -> Channel {
    match value {
        Some("production") => Channel::Production,
        Some("beta") => Channel::Beta,
        // A typo must not opt a development build into production prompts.
        Some(_) => Channel::Development,
        None if cfg!(debug_assertions) => Channel::Development,
        None => Channel::Production,
    }
}

fn identity() -> AppIdentity {
    AppIdentity {
        id: "tawny".into(),
        name: "Tawny".into(),
        platform: if cfg!(target_os = "ios") {
            Platform::Ios
        } else if cfg!(target_os = "android") {
            Platform::Android
        } else {
            Platform::Web
        },
        channel: channel(option_env!("G3_RELEASE_CHANNEL")),
        version: if cfg!(any(target_os = "ios", target_os = "android")) {
            "Unavailable".into()
        } else {
            env!("CARGO_PKG_VERSION").into()
        },
        // No native build means evaluate() returns Unknown, never a forced update.
        build: None,
    }
}

pub fn empty_catalog() -> Catalog {
    Catalog {
        schema_version: 1,
        revision: "offline".into(),
        apps: vec![],
        releases: vec![],
        documents: vec![],
        legal_sets: vec![],
        announcements: vec![],
        release_notes: vec![],
    }
}

fn cached_value(raw: &str) -> Option<CachedCatalog> {
    if raw.len() > g3_brand::model::MAX_CATALOG_BYTES + 4096 {
        return None;
    }
    let value: CachedCatalog = serde_json::from_str(raw).ok()?;
    value.validate().ok()?;
    Some(value)
}

/// Fetch failures retain the last validated snapshot. The clock still advances
/// offline, so an expired required-update policy cannot trap an installation.
pub async fn refresh(mut state: BrandState, force: bool) {
    let now = timestamp();
    state.now.set(now);
    if *state.busy.peek() {
        return;
    }
    let cached = state.catalog.peek().clone();
    if !force {
        let recent_attempt = state
            .last_attempt
            .peek()
            .is_some_and(|at| now >= at && now.saturating_sub(at) < 60);
        if recent_attempt || cached.as_ref().is_some_and(|v| !v.refresh_due(now)) {
            return;
        }
    }
    state.busy.set(true);
    state.last_attempt.set(Some(now));
    let result = match CatalogClient::new(ORIGIN) {
        Ok(client) => client.refresh(cached.as_ref(), now).await,
        Err(error) => Err(error),
    };
    match result {
        Ok(value) => {
            let saved = serde_json::to_string(&value).ok();
            state.catalog.set(Some(value));
            state.offline.set(false);
            if let Some(saved) = saved {
                store::save(CACHE_KEY, &saved).await;
            }
        }
        Err(_) => state.offline.set(true),
    }
    state.busy.set(false);
}

#[component]
pub fn BrandProvider(children: Element) -> Element {
    let mut state = BrandState {
        catalog: use_signal(|| None),
        identity: use_signal(identity),
        busy: use_signal(|| false),
        offline: use_signal(|| false),
        ready: use_signal(|| false),
        now: use_signal(|| 0),
        dismissals: use_signal(Dismissals::default),
        last_attempt: use_signal(|| None),
    };
    use_context_provider(|| state);
    let mut session = use_signal(|| None::<document::Eval>);
    use_future(move || async move {
        // No network request or device storage access during server rendering.
        if !cfg!(feature = "server") {
            if let Ok(Some(info)) = g3_native_plugins::installed_app_info() {
                state.identity.with_mut(|value| {
                    value.version = info.version;
                    value.build = Some(info.build);
                });
            }
            if let Some(raw) = store::load(CACHE_KEY).await {
                state.catalog.set(cached_value(&raw));
            }
            if let Some(raw) = store::load(DISMISSALS_KEY).await
                && raw.len() <= 16384
                && let Ok(value) = serde_json::from_str(&raw)
            {
                state.dismissals.set(value);
            }
            state.ready.set(true);
            refresh(state, false).await;
        }
    });
    use_effect(move || {
        if !cfg!(feature = "server") {
            let mut eval =
                document::eval(&format!("{}\nreturn null;", include_str!("foreground.js")));
            session.set(Some(eval));
            spawn(async move {
                while eval.recv::<bool>().await.is_ok() {
                    if *state.ready.peek() {
                        refresh(state, false).await;
                    }
                }
            });
        }
    });
    use_drop(move || {
        if let Some(eval) = *session.peek() {
            let _ = eval.send("dispose");
        }
    });
    rsx! { {children} }
}

pub fn use_external_link() -> Option<EventHandler<String>> {
    #[cfg(any(target_os = "android", target_os = "ios"))]
    {
        let mut plugins = use_context::<g3_native_plugins::NativePlugins>();
        let app_state = use_context::<crate::state::AppState>();
        Some(use_callback(move |url: String| {
            if g3_brand::PublicUrl::new(url.clone()).is_ok()
                && plugins.external_url.write().open(&url).is_err()
            {
                app_state.show_toast(
                    "Couldn't open the link. Please try again.",
                    g3_ui::Color::Danger,
                );
            }
        }))
    }
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    None
}

fn prompt_key(policy: &ReleasePolicy) -> String {
    // A revised policy must not inherit the snooze for an older release.
    g3_brand::model::digest(&serde_json::to_vec(policy).unwrap_or_default())
}

fn active_policy(
    catalog: &Catalog,
    app: &AppIdentity,
    dismissals: &Dismissals,
    now: i64,
) -> Option<ReleasePolicy> {
    catalog
        .releases
        .iter()
        .find(|policy| match policy.evaluate(app, now) {
            UpdateDecision::Required => true,
            UpdateDecision::Optional => !dismissals.hidden(&app.id, &prompt_key(policy), now),
            _ => false,
        })
        .cloned()
}

pub fn use_update_policy() -> Memo<Option<ReleasePolicy>> {
    let state = use_context::<BrandState>();
    use_memo(move || {
        state.catalog.read().as_ref().and_then(|value| {
            active_policy(
                &value.catalog,
                &state.identity.read(),
                &state.dismissals.read(),
                (state.now)(),
            )
        })
    })
}

pub fn snooze(mut state: BrandState, policy: &ReleasePolicy) {
    let now = timestamp();
    state.dismissals.with_mut(|value| {
        value.entries.retain(|_, until| *until > now);
        value.snooze("tawny", &prompt_key(policy), now + 86400);
    });
    let saved = serde_json::to_string(&*state.dismissals.peek()).ok();
    if let Some(saved) = saved {
        spawn(async move {
            store::save(DISMISSALS_KEY, &saved).await;
        });
    }
}

/// Store requirements are announced globally, while help and account controls
/// remain accessible. This is not a server-side authorization or version gate.
#[component]
pub fn StoreUpdatePrompt() -> Element {
    let route = use_route::<Route>();
    rsx! { StoreUpdatePromptForRoute { route } }
}

#[component]
fn StoreUpdatePromptForRoute(route: ReadSignal<Route>) -> Element {
    let state = use_context::<BrandState>();
    let policy = use_update_policy();
    let mut open = use_signal(|| false);
    let on_open_link = use_external_link();
    use_effect(move || {
        open.set(policy.read().is_some() && !prompt_exempt(&route()));
    });
    let notice = policy.read();
    rsx! {
        Modal { open, aria_label: "App update", dismissible: false,
            if let Some(policy) = notice.as_ref() {
                g3_brand::ui::UpdateNotice {
                    policy: policy.clone(), app: (state.identity)(), now: (state.now)(), on_open_link,
                    on_later: {
                        let policy = policy.clone();
                        move |_| snooze(state, &policy)
                    },
                    Button { fill: ButtonFill::Clear,
                        onclick: move |_| { spawn(g3_route_transitions::animated_navigate(Route::AboutG3tech {})); },
                        "About and help"
                    }
                    Button { fill: ButtonFill::Clear,
                        onclick: move |_| { spawn(g3_route_transitions::animated_navigate(Route::SettingsPage {})); },
                        "Account and privacy controls"
                    }
                }
            }
        }
    }
}

fn prompt_exempt(route: &Route) -> bool {
    matches!(route, Route::AboutG3tech {} | Route::SettingsPage {})
}

#[component]
pub fn AboutG3tech() -> Element {
    let state = use_context::<BrandState>();
    let on_open_link = use_external_link();
    use_effect(move || {
        if (state.ready)() {
            spawn(refresh(state, false));
        }
    });
    let on_refresh = use_callback(move |_| {
        spawn(refresh(state, true));
    });
    let cached = state.catalog.read();
    rsx! {
        div { class: "auxiliary-cover {g3_route_transitions::ROUTE_TRANSITION_OVERLAY_REGION_CLASS}",
            crate::components::PageHeader { title: "About g3tech".to_string(), back_to: Route::SettingsPage {} }
            AboutContent {
                app: (state.identity)(),
                catalog: cached.as_ref().map(|value| value.catalog.clone()).unwrap_or_else(empty_catalog),
                now: (state.now)(), busy: (state.busy)(), offline: (state.offline)(), saved: cached.is_some(),
                on_open_link, on_refresh,
            }
        }
    }
}

#[component]
fn AboutContent(
    app: AppIdentity,
    catalog: Catalog,
    now: i64,
    busy: bool,
    offline: bool,
    saved: bool,
    on_open_link: Option<EventHandler<String>>,
    on_refresh: EventHandler<()>,
) -> Element {
    use g3_ui::{Card, Content, ContentWidth, Stack, Text, TextTone};
    rsx! {
        Content { width: ContentWidth::Readable,
            Stack {
                Card { title: "App information",
                    Stack {
                        if offline {
                            Text { tone: TextTone::Secondary,
                                if saved { "Showing saved app information. We couldn't check for changes." }
                                else { "App information is unavailable right now. Help remains available below." }
                            }
                        }
                        Button { loading: busy, disabled: busy, onclick: move |_| on_refresh.call(()), "Check for updates" }
                    }
                }
                g3_brand::ui::BrandPage { app, catalog, now: (now > 0).then_some(now), on_open_link }
                Card { title: "Help and your data",
                    Stack {
                        Text { "Tawny stores your subscriptions and history on the server you choose. Account and server controls are in Settings." }
                        Button { fill: ButtonFill::Clear,
                            onclick: move |_| { spawn(g3_route_transitions::animated_navigate(Route::SettingsPage {})); },
                            "Account and server settings"
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use g3_brand::{PublicUrl, updates::NativeRelease};

    #[test]
    fn update_notices_leave_help_and_account_settings_accessible() {
        assert!(prompt_exempt(&Route::AboutG3tech {}));
        assert!(prompt_exempt(&Route::SettingsPage {}));
        assert!(!prompt_exempt(&Route::Feed {}));
        assert!(!prompt_exempt(&Route::VideoDetail {
            id: "fixture".into()
        }));
    }

    fn fixture() -> (Catalog, AppIdentity) {
        let mut app = identity();
        app.platform = Platform::Android;
        app.channel = Channel::Production;
        app.build = Some("10".into());
        let mut catalog = empty_catalog();
        catalog.releases.push(ReleasePolicy {
            app: app.id.clone(),
            platform: app.platform,
            channel: app.channel,
            enabled: true,
            broadly_available: true,
            starts_at: 1,
            expires_at: 1000,
            latest: NativeRelease {
                version: "1.1".into(),
                build: "20".into(),
            },
            minimum: None,
            store_url: PublicUrl::new(
                "https://play.google.com/store/apps/details?id=net.g3tech.fixture",
            )
            .unwrap(),
            message: "Please update.".into(),
        });
        (catalog, app)
    }

    #[test]
    fn snooze_is_release_specific_and_cannot_hide_a_required_update() {
        let (mut catalog, app) = fixture();
        let mut dismissed = Dismissals::default();
        dismissed.snooze(&app.id, &prompt_key(&catalog.releases[0]), 900);
        assert!(active_policy(&catalog, &app, &dismissed, 100).is_none());
        catalog.releases[0].minimum = Some(NativeRelease {
            version: "1.0".into(),
            build: "15".into(),
        });
        dismissed.snooze(&app.id, &prompt_key(&catalog.releases[0]), 900);
        assert!(active_policy(&catalog, &app, &dismissed, 100).is_some());
        assert!(active_policy(&catalog, &app, &dismissed, 1000).is_none());
    }

    #[test]
    fn missing_metadata_and_wrong_channel_do_not_prompt() {
        let (catalog, mut app) = fixture();
        app.build = None;
        assert!(active_policy(&catalog, &app, &Dismissals::default(), 100).is_none());
        app.build = Some("10".into());
        app.channel = Channel::Beta;
        assert!(active_policy(&catalog, &app, &Dismissals::default(), 100).is_none());
        assert_eq!(channel(Some("unknown-channel")), Channel::Development);
    }

    #[test]
    fn invalid_cached_snapshots_are_not_used() {
        let value = CachedCatalog {
            fetched_at: 10,
            catalog: empty_catalog(),
            etag: None,
        };
        assert!(cached_value(&serde_json::to_string(&value).unwrap()).is_some());
        assert!(cached_value("{\"catalog\":{\"schema_version\":2}}").is_none());
        assert!(cached_value(&"x".repeat(g3_brand::model::MAX_CATALOG_BYTES + 4097)).is_none());
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use crate::{app::tawny_theme, models::Appearance};
    use g3_auth::PublicRoutes;
    use g3_brand::{AppLinks, AppListing, PublicUrl};
    use g3_ui::{AppWrapper, ComponentMode};

    fn catalog() -> Catalog {
        let mut value = empty_catalog();
        for (id, name) in [("tawny", "Tawny"), ("sample-app", "Sample App")] {
            value.apps.push(AppListing {
                id: id.into(),
                name: name.into(),
                description: "A synthetic app listing.".into(),
                icon: None,
                order: 0,
                links: AppLinks {
                    website: Some(PublicUrl::new("https://example.invalid/app").unwrap()),
                    ..Default::default()
                },
            });
        }
        value
    }

    #[test]
    fn about_and_settings_remain_public_and_about_is_a_sheet() {
        use g3_route_transitions::{NavigationTransition, RouteTransitions};
        assert!(Route::is_public_path("/about-g3tech"));
        assert!(Route::is_public_path("/settings"));
        assert_eq!(
            Route::Feed {}.transition_to(&Route::AboutG3tech {}),
            NavigationTransition::PresentSheet
        );
        assert_eq!(
            Route::AboutG3tech {}.transition_back(),
            NavigationTransition::DismissSheet
        );
    }

    #[component]
    fn Preview(mode: ComponentMode, scheme: Appearance, state: String) -> Element {
        let offline = state != "busy";
        let saved = state != "empty";
        rsx! {
            AppWrapper { mode, theme: tawny_theme(scheme),
                g3_ui::Header { title: "About g3tech" }
                AboutContent { app: identity(), catalog: if saved { catalog() } else { empty_catalog() },
                    now: 100, busy: state == "busy", offline, saved,
                    on_open_link: None, on_refresh: move |_| {},
                }
            }
        }
    }

    #[test]
    fn about_inherits_host_appearance_and_keeps_help_available_offline() {
        for (mode_name, mode) in [("ios", ComponentMode::Ios), ("md", ComponentMode::Md)] {
            for (scheme_name, scheme) in [("light", Appearance::Light), ("dark", Appearance::Dark)]
            {
                for state in ["saved", "empty", "busy"] {
                    let mut dom = VirtualDom::new_with_props(
                        Preview,
                        PreviewProps {
                            mode,
                            scheme,
                            state: state.into(),
                        },
                    );
                    dom.rebuild_in_place();
                    let html = dioxus_ssr::render(&dom);
                    for label in [
                        "Made by g3tech",
                        "More from g3tech",
                        "Help and your data",
                        "Account and server settings",
                    ] {
                        assert!(html.contains(label), "Missing {label}");
                    }
                    assert_eq!(html.contains("Sample App"), state != "empty");
                    assert!(
                        !html.contains("Website — Tawny"),
                        "Do not cross-promote the current app"
                    );
                    if state == "busy" {
                        assert!(html.contains("aria-disabled=\"true\""));
                    }
                    if state == "empty" {
                        assert!(html.contains("App information is unavailable"));
                    }
                    if state == "saved" {
                        assert!(html.contains("Showing saved app information"));
                    }
                    if let Ok(directory) = std::env::var("G3_BRAND_PREVIEW_DIR") {
                        std::fs::create_dir_all(&directory).unwrap();
                        std::fs::write(std::path::Path::new(&directory).join(format!("brand-{mode_name}-{scheme_name}-{state}.html")),
                            format!("<!doctype html><html><head><meta charset=utf-8><meta name=viewport content=\"width=device-width,initial-scale=1\"><link rel=stylesheet href=/g3-ui.css><link rel=stylesheet href=/app.css></head><body style=\"margin:0\">{html}</body></html>")).unwrap();
                    }
                }
            }
        }
    }
}
