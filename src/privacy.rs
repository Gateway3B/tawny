//! Explicit private account downloads; never use the ordinary read cache or
//! the shared public catalog to transport these records.
use crate::{auth::use_session, state::AppState};
use dioxus::prelude::*;

#[cfg(feature = "server")]
mod guards;
#[cfg(feature = "server")]
mod server;
#[cfg(feature = "server")]
pub(crate) mod sessions;

#[post("/api/v1/auth/export",
    state: dioxus::fullstack::extract::State<crate::server::AppServerState>,
    owner: crate::auth::Owner
)]
async fn export_account(expected_owner: Option<String>) -> Result<String> {
    owner.require_reviewed(expected_owner.as_deref())?;
    static EXPORTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);
    let _permit = EXPORTS
        .try_acquire()
        .map_err(|_| ServerFnError::new("Account downloads are busy. Please retry shortly."))?;
    tokio::time::timeout(
        std::time::Duration::from_secs(30),
        server::export(
            &state.db,
            &owner,
            expected_owner.as_deref(),
            server::Limits::default(),
        ),
    )
    .await
    .map_err(|_| {
        ServerFnError::new("Your download took too long. Retry or contact your server operator.")
    })?
}

#[component]
pub fn AccountDataDownload() -> Element {
    let session = use_session();
    let app = use_context::<AppState>();
    let mut loading = use_signal(|| false);
    #[cfg(any(target_os = "android", target_os = "ios"))]
    let mut plugins = use_context::<g3_native_plugins::NativePlugins>();
    let download = use_callback(move |_| {
        if *loading.peek() {
            return;
        }
        if !cfg!(any(
            all(feature = "web", target_arch = "wasm32"),
            target_os = "android",
            target_os = "ios"
        )) {
            app.show_toast(
                "Use the web app to save your data, or the mobile app to share a smaller copy.",
                g3_ui::Color::Danger,
            );
            return;
        }
        let Some(owner) = session
            .account
            .peek()
            .as_ref()
            .map(|account| account.id.clone())
        else {
            return;
        };
        loading.set(true);
        spawn(async move {
            let result = export_account(Some(owner.clone())).await;
            if !session.matches(&owner) {
                loading.set(false);
                return;
            }
            match result {
                Ok(data) => {
                    let delivered: std::result::Result<(), &'static str> = async {
                        cfg_if::cfg_if! {
                            if #[cfg(all(feature = "web", target_arch = "wasm32"))] {
                                save_web_download(data).await
                            } else if #[cfg(any(target_os = "android", target_os = "ios"))] {
                                if data.len() > 128 * 1024 {
                                    return Err("This copy is too large to share here. Use the web app to save it, or contact your server operator.");
                                }
                                plugins.clipboard.write().share(data).map_err(|_| "Your device couldn't open sharing. Use the web app or contact your server operator.")
                            } else {
                                let _ = data;
                                Err("Use the web app to save your data, or the mobile app to share a smaller copy.")
                            }
                        }
                    }.await;
                    if let Err(message) = delivered {
                        app.show_toast(message, g3_ui::Color::Danger);
                    }
                }
                Err(problem) => app.show_toast(
                    crate::auth::readable(&problem.to_string()),
                    g3_ui::Color::Danger,
                ),
            }
            loading.set(false);
        });
    });
    rsx! { DownloadCard { loading: loading(), on_download: download } }
}

#[cfg(all(feature = "web", target_arch = "wasm32"))]
async fn save_web_download(data: String) -> std::result::Result<(), &'static str> {
    let mut eval = document::eval(
        r#"
        const data = await dioxus.recv();
        try {
            const url = URL.createObjectURL(new Blob([data], {type:'application/json'}));
            const link = document.createElement('a');
            link.href = url; link.download = 'tawny-account-data.json';
            document.body.appendChild(link); link.click(); link.remove();
            setTimeout(() => URL.revokeObjectURL(url), 60000);
            dioxus.send(true);
        } catch (_) { dioxus.send(false); }
    "#,
    );
    eval.send(data)
        .map_err(|_| "Couldn't open your download. Please retry.")?;
    if eval
        .recv::<bool>()
        .await
        .map_err(|_| "Couldn't open your download. Please retry.")?
    {
        Ok(())
    } else {
        Err(
            "Your browser couldn't open the download. Please retry or contact your server operator.",
        )
    }
}

#[component]
fn DownloadCard(loading: bool, on_download: EventHandler<()>) -> Element {
    use g3_ui::{Button, ButtonFill, Card, Stack, Text, TextTone};
    rsx! {
        Card { title: "Your data",
            Stack {
                Text { "Get a copy of your profile, subscriptions, playlists, queue, history, progress and legal acknowledgments from this server." }
                Text { tone: TextTone::Secondary, "Web downloads save as JSON. Mobile can share smaller copies. Contact your server operator for requests needing review, including backups and logs." }
                Button { fill: ButtonFill::Outline, loading, disabled: loading,
                    onclick: move |_| on_download.call(()), "Get account data"
                }
            }
        }
    }
}

#[cfg(test)]
mod markup_tests {
    use super::*;
    use crate::{app::tawny_theme, models::Appearance};
    use g3_ui::{AppWrapper, ComponentMode, Content, ContentWidth, Header};

    #[component]
    fn Preview(mode: ComponentMode, appearance: Appearance, loading: bool) -> Element {
        rsx! {
            AppWrapper { mode, theme: tawny_theme(appearance),
                Header { title: "Account" }
                Content { width: ContentWidth::Readable,
                    DownloadCard { loading, on_download: move |_| {} }
                }
            }
        }
    }

    #[test]
    fn download_explains_scope_and_disables_repeat_submission() {
        for (mode_name, mode) in [("ios", ComponentMode::Ios), ("md", ComponentMode::Md)] {
            for (appearance_name, appearance) in
                [("light", Appearance::Light), ("dark", Appearance::Dark)]
            {
                for loading in [false, true] {
                    let mut dom = VirtualDom::new_with_props(
                        Preview,
                        PreviewProps {
                            mode,
                            appearance,
                            loading,
                        },
                    );
                    dom.rebuild_in_place();
                    let html = dioxus_ssr::render(&dom);
                    for label in [
                        "Get account data",
                        "legal acknowledgments",
                        "backups and logs",
                        "Mobile can share smaller copies",
                    ] {
                        assert!(html.contains(label));
                    }
                    if loading {
                        assert!(html.contains("aria-disabled=\"true\""));
                    }
                    if let Ok(directory) = std::env::var("G3_BRAND_PREVIEW_DIR") {
                        let path = std::path::Path::new(&directory);
                        std::fs::create_dir_all(path).unwrap();
                        std::fs::write(path.join(format!("download-{mode_name}-{appearance_name}-{loading}.html")),
                            format!("<!doctype html><html><head><meta charset=utf-8><meta name=viewport content=\"width=device-width,initial-scale=1\"><link rel=stylesheet href=/g3-ui.css><link rel=stylesheet href=/app.css></head><body style=\"margin:0\">{html}</body></html>")).unwrap();
                    }
                }
            }
        }
    }
}
