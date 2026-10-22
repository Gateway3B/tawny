//! Who the viewer is, and which backend they are talking to.
//!
//! The flow a first launch takes:
//!
//! 1. No backend has been chosen, so [`AccountGate`] shows the setup screen
//!    instead of the app. Nothing else can work until this is answered - there
//!    is no library to show and no server to ask.
//! 2. With a backend, the gate mints a guest account and receives its session
//!    cookie. No sign-in wall: the app is usable immediately, and the account exists so
//!    that everything saved from this moment has an owner.
//! 3. Later, from settings, that guest can be promoted to a real account
//!    (keeping its library) or abandoned by signing in to another one.

use dioxus::prelude::*;
use g3_ui::{
    AlertButton, AlertOptions, Button, ButtonExpand, ButtonFill, Card, Color, Content,
    ContentWidth, HeadingLevel, Input, InputType, Space, Stack, Text, TextTone, TextVariant,
    use_alert,
};

use crate::{
    api::{delete_account, register_account, sign_in_to_account, sign_out_of_account},
    auth::{SessionStatus, readable, use_session},
    models::{Credentials, validate_email, validate_password},
    server_url,
    state::AppState,
};

/// Reload the client so a new backend URL takes effect.
///
/// `set_server_url` keeps only the first value it is given, so there is no way
/// to repoint a running process - see `config`. On the web a reload is a fresh
/// process; a packaged client has to be restarted by hand, which is what the
/// setup screen says.
fn reload_client() {
    // Signing in/out unmounts the account's keyed form. Keep the reload in
    // the root scope so that removing that form cannot cancel it.
    let document = document::document();
    dioxus::core::spawn_forever(async move {
        let mut eval = document.eval("window.location.reload(); dioxus.send(true);".into());
        let _ = eval.recv::<bool>().await;
    });
}

/// Chooses between the app and the "connect to a server" screen.
///
/// **The first render has to match what the server rendered.** Reading
/// `has_chosen_backend()` in the signal's initialiser did not: it consults
/// localStorage on the client and is always false on the server, so the moment
/// a backend had been saved the client hydrated a Router over a server-rendered
/// setup screen. Dioxus's interpreter then walked a node table that did not
/// describe the DOM in front of it and died on
/// `Cannot set properties of undefined (setting 'textContent')`.
///
/// So both sides start by rendering the app, and the real answer arrives in an
/// effect, which only ever runs on the client. A configured viewer - almost
/// everyone, almost always - sees no flash; a first launch sees the app for a
/// frame before the setup screen replaces it, which is the cheaper of the two
/// mistakes to make.
#[component]
pub fn AccountGate(#[props(default)] allow_offline: bool, children: Element) -> Element {
    let session = use_session();
    let mut backend_chosen = use_signal(|| true);

    use_effect(move || backend_chosen.set(server_url::has_chosen_backend()));

    if !allow_offline && !backend_chosen() {
        return rsx! {
            BackendSetupScreen {
                on_saved: move |_| {
                    backend_chosen.set(true);
                    reload_client();
                },
            }
        };
    }

    match (session.status)() {
        SessionStatus::Failed(message) if !allow_offline => rsx! {
            BackendSetupScreen {
                problem: message,
                on_saved: move |_| reload_client(),
            }
        },
        // The app renders while the guest is being minted rather than behind a
        // spinner. Everything on screen at that moment comes from the local
        // cache, and blocking it would make a cold start feel like a network
        // failure.
        _ => rsx! { {children} },
    }
}

#[cfg(test)]
mod gate_tests {
    use super::*;

    #[component]
    fn Fixture(allow_offline: bool) -> Element {
        use_context_provider(|| crate::auth::Session {
            account: Signal::new(None),
            status: Signal::new(SessionStatus::Failed("Synthetic offline server".into())),
        });
        rsx! {
            g3_ui::AppWrapper {
                AccountGate { allow_offline, p { "Public help fixture" } }
            }
        }
    }

    #[test]
    fn offline_help_can_render_while_library_pages_offer_server_setup() {
        for allow_offline in [false, true] {
            let mut dom = VirtualDom::new_with_props(Fixture, FixtureProps { allow_offline });
            dom.rebuild_in_place();
            let html = dioxus_ssr::render(&dom);
            assert_eq!(html.contains("Public help fixture"), allow_offline);
            assert_eq!(html.contains("Connect to a Tawny server"), !allow_offline);
            if !allow_offline {
                assert!(html.contains("About g3tech and other apps"));
            }
        }
    }
}

/// The address field's value, filled in on the client.
///
/// The stored address lives in client storage the server cannot read, so a
/// server render of `backend_url()` wrote its own fallback, `localhost`, into
/// the field. Hydration keeps markup it believes matches, so the field went on
/// showing `localhost` after every reload, even though the address had saved
/// and was in use. Both sides render it empty, and the client fills it in.
fn use_backend_url_field() -> Signal<String> {
    let mut url = use_signal(String::new);
    use_effect(move || url.set(server_url::backend_url()));
    url
}

#[component]
fn BackendSetupScreen(problem: Option<String>, on_saved: EventHandler<()>) -> Element {
    let url = use_backend_url_field();
    let mut error = use_signal(|| problem.clone().unwrap_or_default());

    rsx! {
        Content { width: ContentWidth::Readable,
            Card {
                title: "Connect to a Tawny server",
                heading_level: HeadingLevel::H1,
                Stack {
                    Text { tone: TextTone::Secondary,
                        "Tawny stores your subscriptions and history on a server you or someone you trust runs. Enter its address to begin."
                    }
                    Input {
                        label: "Server address",
                        value: url,
                        input_type: InputType::Url,
                        placeholder: "https://tawny.example",
                        error: Some(error()).filter(|message| !message.is_empty()),
                        oninput: move |_| error.set(String::new()),
                    }
                    Button {
                        expand: ButtonExpand::Block,
                        onclick: move |_| {
                            match server_url::set_backend_url(&url()) {
                                Some(_) => on_saved.call(()),
                                None => error.set(
                                    "Enter a full address, including http:// or https://.".into(),
                                ),
                            }
                        },
                        "Connect"
                    }
                    Button { fill: ButtonFill::Clear,
                        onclick: move |_| { spawn(g3_route_transitions::animated_navigate(crate::app::Route::AboutG3tech {})); },
                        "About g3tech and other apps"
                    }
                    Text { variant: TextVariant::Caption,
                        "Changing this restarts the app. On a packaged build, close and reopen it."
                    }
                }
            }
        }
    }
}

/// The account section of the settings page.
#[component]
pub fn AccountSettings() -> Element {
    let session = use_session();
    let account = session.account.read();
    rsx! {
        if let Some(account) = account.as_ref() {
            // Draft credentials and in-flight confirmations belong to one
            // account. A switch remounts the form with empty fields.
            AccountForm { key: "{account.id}", account: account.clone() }
        } else {
            Card { title: "Account",
                Text { tone: TextTone::Secondary, "Connecting to your server…" }
            }
        }
    }
}

#[component]
fn AccountForm(account: crate::models::Account) -> Element {
    let app_state = use_context::<AppState>();
    let alerts = use_alert();
    let mut session = use_session();
    let email = use_signal(String::new);
    let mut password = use_signal(String::new);
    let mut error = use_signal(String::new);
    let mut busy = use_signal(|| false);
    let mut signing_in = use_signal(|| false);
    let registration_owner = account.id.clone();
    let deletion_owner = account.id.clone();
    let sign_out_owner = account.id.clone();
    let is_guest = account.is_guest;
    let upgrade = is_guest && !signing_in();

    rsx! {
        Card {
            title: "Account",
            subtitle: account.is_recoverable().then(|| format!("Signed in as {}", account.label())),
            Stack {
                if !account.is_recoverable() {
                    Text { color: Color::Warning,
                        "You are signed in as a guest. This library lives only on this device's session - add an email and password to be able to sign back in, here or anywhere else."
                    }
                }
                if is_guest || signing_in() {
                    Input {
                        label: "Email",
                        value: email,
                        input_type: InputType::Email,
                        autocomplete: "username",
                        oninput: move |_| error.set(String::new()),
                    }
                    Input {
                        label: "Password",
                        value: password,
                        input_type: InputType::Password,
                        // Tells a password manager to offer a new suggestion
                        // when this is a sign-up and the stored one when it is not.
                        autocomplete: if upgrade { "new-password" } else { "current-password" },
                        error: Some(error()).filter(|message| !message.is_empty()),
                        oninput: move |_| error.set(String::new()),
                    }
                    Button {
                        expand: ButtonExpand::Block,
                        loading: busy(),
                        onclick: move |_| {
                            if *busy.peek() || !session.matches(&registration_owner) { return; }
                            let reviewed_owner = registration_owner.clone();
                            // Validated here first so the field-level rules in
                            // `models` mark the input rather than arriving as
                            // server prose. The server checks them again; this
                            // is for the message, not for trust.
                            if upgrade {
                                if let Err(problem) = validate_email(&email()) {
                                    error.set(problem.message().into());
                                    return;
                                }
                                if let Err(problem) = validate_password(&password()) {
                                    error.set(problem.message().into());
                                    return;
                                }
                            }
                            let credentials = Credentials { email: email(), password: password() };
                            busy.set(true);
                            spawn(async move {
                                let result = if upgrade {
                                    register_account(credentials, Some(reviewed_owner.clone())).await
                                } else {
                                    sign_in_to_account(credentials).await
                                };
                                if !session.matches(&reviewed_owner) { return; }
                                match result {
                                    Ok(account) => {
                                        let switched = !upgrade;
                                        if switched {
                                            g3_cache::set_cache_owner(None).await;
                                            if !session.matches(&reviewed_owner) { return; }
                                            app_state.forget_account();
                                        }
                                        session.adopt(account);
                                        password.set(String::new());
                                        signing_in.set(false);
                                        app_state.show_toast(
                                            if switched { "Signed in" } else { "Account created" },
                                            Color::Success,
                                        );
                                        // A different account has a different
                                        // library, and this process is holding
                                        // the previous one in memory.
                                        if switched {
                                            reload_client();
                                        }
                                    }
                                    Err(server_error) => error.set(readable(&server_error.to_string())),
                                }
                                busy.set(false);
                            });
                        },
                        if upgrade { "Create account" } else { "Sign in" }
                    }
                    if is_guest {
                        Button {
                            fill: ButtonFill::Clear,
                            expand: ButtonExpand::Block,
                            disabled: busy(),
                            onclick: move |_| {
                                signing_in.toggle();
                                error.set(String::new());
                            },
                            if signing_in() { "Back to creating an account" } else { "I already have an account" }
                        }
                        if signing_in() {
                            Text { color: Color::Warning,
                                "Signing in to another account leaves this guest library behind. It is not merged."
                            }
                        }
                    }
                } else {
                    Button {
                        fill: ButtonFill::Outline,
                        color: Color::Neutral,
                        expand: ButtonExpand::Block,
                        disabled: busy(),
                        onclick: move |_| {
                            if *busy.peek() || !session.matches(&sign_out_owner) { return; }
                            let reviewed_owner = sign_out_owner.clone();
                            busy.set(true);
                            spawn(async move {
                                let result = sign_out_of_account().await;
                                if !session.matches(&reviewed_owner) { return; }
                                if let Err(problem) = result {
                                    app_state.show_toast(readable(&problem.to_string()), Color::Danger);
                                    busy.set(false);
                                    return;
                                }
                                g3_cache::set_cache_owner(None).await;
                                if !session.matches(&reviewed_owner) { return; }
                                app_state.forget_account();
                                session.forget();
                                reload_client();
                            });
                        },
                        "Sign out"
                    }
                }
                Button {
                    fill: ButtonFill::Outline,
                    color: Color::Danger,
                    expand: ButtonExpand::Block,
                    disabled: busy(),
                    onclick: move |_| {
                        if *busy.peek() || !session.matches(&deletion_owner) { return; }
                        let reviewed_owner = deletion_owner.clone();
                        busy.set(true);
                        spawn(async move {
                        let buttons = vec![AlertButton::cancel("Cancel"), AlertButton::destructive("Delete account")];
                        if !alerts.show(AlertOptions {
                            title: "Delete account?".into(),
                            message: Some("This deletes your account, subscriptions, playlists, queue and history from this server. Other devices lose server access to this account. The server operator manages backup retention.".into()),
                            buttons: buttons.clone(),
                            input: None,
                        }).await.confirmed(&buttons) { busy.set(false); return; }
                        if !session.matches(&reviewed_owner) { return; }
                        let result = delete_account(Some(reviewed_owner.clone())).await;
                        if !session.matches(&reviewed_owner) { return; }
                        match result {
                            Ok(()) => {
                                // Finish cleanup before forgetting the session
                                // unmounts this form and cancels its task.
                                g3_cache::set_cache_owner(None).await;
                                if !session.matches(&reviewed_owner) { return; }
                                app_state.forget_account();
                                session.forget();
                                reload_client();
                            }
                            Err(problem) => app_state.show_toast(readable(&problem.to_string()), Color::Danger),
                        }
                        busy.set(false);
                        });
                    },
                    "Delete account"
                }
            }
        }
    }
}

/// The server section of the settings page.
#[component]
pub fn BackendSettings() -> Element {
    let url = use_backend_url_field();
    let mut error = use_signal(String::new);
    let mut saved = use_signal(|| false);

    rsx! {
        Card { title: "Server",
            Stack { gap: Space::Md,
                Input {
                    label: "Server address",
                    value: url,
                    input_type: InputType::Url,
                    error: Some(error()).filter(|message| !message.is_empty()),
                    helper: saved().then(|| {
                        "Saved. Restarting to connect to it - on a packaged build, close and reopen the app."
                            .to_string()
                    }),
                    oninput: move |_| {
                        error.set(String::new());
                        saved.set(false);
                    },
                }
                Button {
                    fill: ButtonFill::Outline,
                    color: Color::Neutral,
                    expand: ButtonExpand::Block,
                    onclick: move |_| {
                        match server_url::set_backend_url(&url()) {
                            Some(_) => {
                                saved.set(true);
                                reload_client();
                            }
                            None => error.set(
                                "Enter a full address, including http:// or https://.".into(),
                            ),
                        }
                    },
                    "Save and reconnect"
                }
            }
        }
    }
}
