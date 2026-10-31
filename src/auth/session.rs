//! Who the client is acting as, established before anything asks the server
//! for data.
//!
//! This lives above the UI rather than inside it because of ordering: the
//! library is per account now, so every read answers 401 until a session
//! exists. `AppStateProvider` bootstraps the session here; a first launch
//! refetches every cached read once its guest account exists.

use dioxus::prelude::*;

use crate::{
    api::{create_guest_account, current_account},
    models::Account,
};

/// Where the session is in its bootstrap.
///
/// `Failed` is a first-class state rather than a toast: a client pointed at an
/// unreachable backend cannot proceed, and the only useful next action is to
/// change the address - so the message has to reach the screen that owns that
/// input.
#[derive(Clone, Debug, PartialEq)]
pub enum SessionStatus {
    Connecting,
    Ready,
    Failed(String),
}

#[derive(Clone, Copy)]
pub struct Session {
    pub account: Signal<Option<Account>>,
    pub status: Signal<SessionStatus>,
}

impl Session {
    /// Discard an asynchronous result if the reviewed account is no longer
    /// shown. This does not authorize the request; the server checks too.
    pub fn matches(self, expected_owner: &str) -> bool {
        self.account
            .peek()
            .as_ref()
            .is_some_and(|account| account.id == expected_owner)
    }

    /// The server wrote the cookie; retain only the account display state.
    pub fn adopt(&mut self, account: Account) {
        self.account.set(Some(account));
        self.status.set(SessionStatus::Ready);
    }

    pub fn forget(&mut self) {
        self.account.set(None);
    }
}

/// Create the session context and bootstrap it.
///
/// Called once, from `AppStateProvider`, so that every consumer below it -
/// including the library sync in that same component - can rely on the account
/// existing before it asks for anything.
pub fn use_session_provider() -> Session {
    let mut session = use_context_provider(|| Session {
        account: Signal::new(None),
        status: Signal::new(SessionStatus::Connecting),
    });

    let mut started = use_signal(|| false);

    use_effect(move || {
        if started() {
            return;
        }
        started.set(true);
        spawn(async move {
            if let Ok(account) = current_account().await {
                session.adopt(account);
                return;
            }
            match create_guest_account().await {
                Ok(account) => {
                    session.adopt(account);
                    // Whatever the screens asked for before this account existed
                    // was refused; ask again now that it does.
                    g3_cache::invalidate_all_cached();
                }
                Err(error) => session
                    .status
                    .set(SessionStatus::Failed(readable(&error.to_string()))),
            }
        });
    });

    session
}

pub fn use_session() -> Session {
    use_context::<Session>()
}

/// Server errors arrive as transport prose. This keeps the one case a viewer
/// can actually act on - not reaching the server at all - legible.
pub fn readable(error: &str) -> String {
    let lowered = error.to_lowercase();
    if lowered.contains("error sending request")
        || lowered.contains("failed to fetch")
        || lowered.contains("connect")
        || lowered.contains("network")
    {
        "Could not reach that server. Check the address and that it is running.".into()
    } else {
        error.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_result_from_an_old_account_no_longer_matches_after_switch_or_sign_out() {
        let mut dom = VirtualDom::new(|| rsx! {});
        dom.rebuild_in_place();
        dom.in_scope(ScopeId::ROOT, || {
            let mut session = Session {
                account: Signal::new(None),
                status: Signal::new(SessionStatus::Connecting),
            };
            let account = |id: &str| Account {
                id: id.into(),
                display_name: "Fixture".into(),
                email: None,
                is_guest: true,
            };
            assert!(!session.matches("app_user:first"));
            session.adopt(account("app_user:first"));
            assert!(session.matches("app_user:first"));
            session.adopt(account("app_user:second"));
            assert!(!session.matches("app_user:first"));
            assert!(session.matches("app_user:second"));
            session.forget();
            assert!(!session.matches("app_user:second"));
        });
    }
}
