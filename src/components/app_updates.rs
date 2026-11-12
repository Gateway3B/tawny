//! Updating the installed app from GitHub releases: check, download, install.
//!
//! Only the native builds have the updater. The web and server builds get an
//! empty card, so a server render and the page it hydrates still match, and
//! a native build with no release (iOS, macOS, Linux) renders nothing either.

#[cfg(not(any(feature = "desktop", feature = "mobile")))]
use dioxus::prelude::*;

#[cfg(not(any(feature = "desktop", feature = "mobile")))]
#[component]
pub fn AppUpdatesCard() -> Element {
    rsx! {}
}

#[cfg(any(feature = "desktop", feature = "mobile"))]
pub use native::AppUpdatesCard;

#[cfg(any(feature = "desktop", feature = "mobile"))]
mod native {
    use crate::{state::AppState, updater};
    use dioxus::prelude::*;
    use dioxus_icons::lucide::{Download, RefreshCw};
    use g3_native_plugins::{Updater, UpdaterState};
    use g3_ui::{
        Button, ButtonFill, ButtonSize, Card, Color, Item, List, ListVariant, Space, Stack, Text,
        TextVariant,
    };

    #[component]
    pub fn AppUpdatesCard() -> Element {
        let app_state = use_context::<AppState>();
        let mut state = use_signal(|| Updater::new().state());

        // The updater reports a state rather than events, so read it while the
        // card is open. A timer on the page paces it: there is no Rust timer
        // shared by every build.
        use_future(move || async move {
            if !updater::SUPPORTED {
                return;
            }
            // A check the first time Settings opens, so an update is on show
            // without looking for it.
            if Updater::new().state() == UpdaterState::Idle {
                let _ = Updater::new().start_check();
            }
            let mut tick = document::eval(
                "clearInterval(window.__tawnyUpdaterTick); window.__tawnyUpdaterTick = setInterval(() => dioxus.send(true), 500);",
            );
            while tick.recv::<bool>().await.is_ok() {
                let next = Updater::new().state();
                if *state.peek() != next {
                    state.set(next);
                }
            }
        });

        use_drop(|| {
            let _ = document::eval("clearInterval(window.__tawnyUpdaterTick);");
        });

        if !updater::SUPPORTED {
            return rsx! {};
        }

        let current = env!("CARGO_PKG_VERSION");
        let (description, action) = describe(&state.read());

        rsx! {
            Card { title: "App updates",
                Stack { gap: Space::Md,
                    List { variant: ListVariant::Filled,
                        Item {
                            wrap: true,
                            start: rsx! { RefreshCw { size: 18 } },
                            label: "Tawny {current}",
                            description: "{description}",
                        }
                    }
                    if let Some(action) = action {
                        Stack { horizontal: true, gap: Space::Sm,
                            Button {
                                fill: if action == Action::Check { ButtonFill::Outline } else { ButtonFill::Solid },
                                color: if action == Action::Check { Color::Neutral } else { Color::Accent },
                                size: ButtonSize::Sm,
                                start: rsx! { Download { size: 16 } },
                                onclick: move |_| {
                                    let mut updater = Updater::new();
                                    let started = match action {
                                        Action::Check => updater.start_check(),
                                        Action::Download => updater.start_download(),
                                        // A store page opens without a download first.
                                        Action::Install | Action::OpenStore => updater.install(),
                                    };
                                    match started {
                                        Ok(()) => state.set(updater.state()),
                                        Err(error) => app_state.show_toast(error, Color::Danger),
                                    }
                                },
                                "{action.label()}"
                            }
                        }
                    }
                    Text { variant: TextVariant::Caption,
                        "Releases come from github.com/Gateway3B/tawny and are checked against Tawny's signing key before they install."
                    }
                }
            }
        }
    }

    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    enum Action {
        Check,
        Download,
        Install,
        OpenStore,
    }

    impl Action {
        fn label(self) -> &'static str {
            match self {
                Action::Check => "Check for updates",
                Action::Download => "Download",
                Action::OpenStore => "Open the store",
                // Desktop closes and reopens; Android may show the system's own
                // confirmation.
                Action::Install => {
                    if cfg!(target_os = "android") {
                        "Install"
                    } else {
                        "Restart and install"
                    }
                }
            }
        }
    }

    /// What to say about the updater's state, and the button that moves it on.
    fn describe(state: &UpdaterState) -> (String, Option<Action>) {
        match state {
            UpdaterState::Idle => (
                "Check GitHub for a newer release".to_string(),
                Some(Action::Check),
            ),
            UpdaterState::Checking => ("Checking for updates\u{2026}".to_string(), None),
            UpdaterState::UpToDate => (
                "This is the latest version".to_string(),
                Some(Action::Check),
            ),
            UpdaterState::Available(update) if update.store => (
                format!("Version {} is in the store", update.version),
                Some(Action::OpenStore),
            ),
            UpdaterState::Available(update) => (
                format!("Version {} is available", update.version),
                Some(Action::Download),
            ),
            UpdaterState::Downloading { update, progress } => {
                let done = megabytes(progress.bytes_done);
                let text = match progress.bytes_total {
                    Some(total) if total > 0 => format!(
                        "Downloading {}: {}%",
                        update.version,
                        progress.bytes_done.saturating_mul(100) / total
                    ),
                    _ => format!("Downloading {}: {done}", update.version),
                };
                (text, None)
            }
            UpdaterState::Ready(update) => (
                format!("Version {} is downloaded and verified", update.version),
                Some(Action::Install),
            ),
            UpdaterState::NeedsPermission(_) => (
                "Allow Tawny to install apps in the screen that opened, then come back and install"
                    .to_string(),
                Some(Action::Install),
            ),
            UpdaterState::Installing(update) => {
                (format!("Installing {}\u{2026}", update.version), None)
            }
            UpdaterState::Failed(error) => {
                (format!("Could not update: {error}"), Some(Action::Check))
            }
        }
    }

    fn megabytes(bytes: u64) -> String {
        format!("{:.1} MB", bytes as f64 / 1_000_000.0)
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use g3_native_plugins::{DownloadProgress, Update};

        fn update() -> Update {
            Update {
                version: "0.2.0".to_string(),
                current_version: "0.1.0".to_string(),
                notes: None,
                pub_date: None,
                store: false,
            }
        }

        #[test]
        fn each_state_offers_the_step_that_moves_it_on() {
            assert_eq!(describe(&UpdaterState::Idle).1, Some(Action::Check));
            assert_eq!(describe(&UpdaterState::Checking).1, None);
            let (text, action) = describe(&UpdaterState::Available(update()));
            assert_eq!(text, "Version 0.2.0 is available");
            assert_eq!(action, Some(Action::Download));
            let store = Update {
                store: true,
                ..update()
            };
            assert_eq!(
                describe(&UpdaterState::Available(store)).1,
                Some(Action::OpenStore)
            );
            let downloading = UpdaterState::Downloading {
                update: update(),
                progress: DownloadProgress {
                    bytes_done: 25,
                    bytes_total: Some(100),
                },
            };
            assert_eq!(
                describe(&downloading),
                ("Downloading 0.2.0: 25%".to_string(), None)
            );
            let unknown = UpdaterState::Downloading {
                update: update(),
                progress: DownloadProgress {
                    bytes_done: 2_500_000,
                    bytes_total: None,
                },
            };
            assert_eq!(describe(&unknown).0, "Downloading 0.2.0: 2.5 MB");
            assert_eq!(
                describe(&UpdaterState::Ready(update())).1,
                Some(Action::Install)
            );
            assert_eq!(
                describe(&UpdaterState::NeedsPermission(update())).1,
                Some(Action::Install)
            );
            assert_eq!(
                describe(&UpdaterState::Failed("offline".to_string())).1,
                Some(Action::Check)
            );
        }
    }
}
