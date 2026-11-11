//! Who the client is acting as, established before anything asks the server
//! for data.
//!
//! This lives above the UI rather than inside it because of ordering: the
//! library is per account now, so every read answers 401 until a session
//! exists. `AppStateProvider` bootstraps the session here; a first launch
//! refetches every cached read once its guest account exists.

use dioxus::{core::Task, prelude::*};

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
    connection: Signal<u64>,
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

    /// A native WebView reload keeps this Rust state alive. Reconnect through
    /// the provider instead, so the old request is cancelled before retrying.
    pub fn reconnect(&mut self) {
        self.forget();
        self.status.set(SessionStatus::Connecting);
        self.connection += 1;
    }

    pub fn is_reconnecting(self) -> bool {
        (self.connection)() > 0 && matches!((self.status)(), SessionStatus::Connecting)
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
        connection: Signal::new(0),
    });

    let mut pending = use_signal(|| None::<Task>);

    use_effect(move || {
        let connection = (session.connection)();
        if let Some(task) = *pending.peek() {
            task.cancel();
        }
        let task = spawn(async move {
            if connection > 0 {
                g3_cache::set_cache_owner(None).await;
            }
            match connect().await {
                Ok(account) => {
                    session.adopt(account);
                    g3_cache::invalidate_all_cached();
                }
                Err(error) => session.status.set(SessionStatus::Failed(error)),
            }
        });
        pending.set(Some(task));
    });

    session
}

async fn connect() -> Result<Account, String> {
    use futures_util::future::{Either, select};

    let request = Box::pin(async {
        if let Ok(account) = current_account().await {
            return Ok(account);
        }
        create_guest_account().await.map_err(|_| {
            "Could not connect to Tawny. Check the server address and try again.".to_string()
        })
    });
    // Bound the whole attempt, including a server that accepts a connection
    // but never replies. Dropping the losing future also cancels its request.
    let deadline = Box::pin(async {
        let mut timer = document::eval(
            "await new Promise(resolve => setTimeout(resolve, 20000)); dioxus.send(true);",
        );
        let _ = timer.recv::<bool>().await;
        Err("Connection timed out. Check your internet connection and try again.".to_string())
    });
    match select(request, deadline).await {
        Either::Left((result, _)) | Either::Right((result, _)) => result,
    }
}

pub fn use_session() -> Session {
    use_context::<Session>()
}

/// Keep account guidance, while hiding transport and server internals.
pub fn readable(error: &str) -> String {
    for message in [
        "Enter an email address.",
        "That does not look like an email address.",
        "An account already uses that email.",
        "Use at least 8 characters.",
        "Use at most 512 characters.",
        "That email and password do not match an account.",
        "This account already has an email address.",
    ] {
        if error.contains(message) {
            return message.into();
        }
    }
    let lowered = error.to_lowercase();
    if lowered.contains("error sending request")
        || lowered.contains("failed to fetch")
        || lowered.contains("connect")
        || lowered.contains("network")
    {
        "Could not reach that server. Check the address and that it is running.".into()
    } else {
        "Something went wrong. Please try again.".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_errors_show_guidance_without_transport_details() {
        assert_eq!(
            readable(
                "error receiving status code: That email and password do not match an account. (401)"
            ),
            "That email and password do not match an account."
        );
        assert_eq!(
            readable("error decoding response body: invalid JSON at line 1"),
            "Something went wrong. Please try again."
        );
    }

    #[test]
    fn a_result_from_an_old_account_no_longer_matches_after_switch_or_sign_out() {
        let mut dom = VirtualDom::new(|| rsx! {});
        dom.rebuild_in_place();
        dom.in_scope(ScopeId::ROOT, || {
            let mut session = Session {
                account: Signal::new(None),
                status: Signal::new(SessionStatus::Connecting),
                connection: Signal::new(0),
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
            session.reconnect();
            assert!(session.is_reconnecting());
            assert!(matches!((session.status)(), SessionStatus::Connecting));
            session.adopt(account("app_user:third"));
            assert!(!session.is_reconnecting());
        });
    }
}
