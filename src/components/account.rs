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
    ContentWidth, Header, HeadingLevel, Input, InputType, Space, Stack, Text, TextTone, use_alert,
};

use crate::{
    api::{delete_account, register_account, sign_in_to_account, sign_out_of_account},
    auth::{SessionStatus, readable, use_session},
    data_change::{DataChange, invalidate},
    library_move,
    models::{Credentials, LibraryImport, validate_email, validate_password},
    server_url,
    state::AppState,
};

/// Reload the client after a change of backend or account.
///
/// The running app holds the previous session and library in memory. On the
/// web a reload is a fresh process; a packaged client keeps its state across a
/// WebView reload and has to be restarted by hand, which is what the setup
/// screen says.
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
pub fn AccountGate(children: Element) -> Element {
    let mut session = use_session();
    let app_state = use_context::<AppState>();
    let mut backend_chosen = use_signal(|| true);

    use_effect(move || backend_chosen.set(server_url::has_chosen_backend()));
    let on_saved = use_callback(move |()| {
        backend_chosen.set(true);
        app_state.forget_account();
        session.reconnect();
    });

    if !backend_chosen()
        || session.is_reconnecting()
        || matches!((session.status)(), SessionStatus::Failed(_))
    {
        return rsx! {
            BackendSetupScreen {
                connecting: session.is_reconnecting(),
                on_saved,
            }
        };
    }

    rsx! { {children} }
}

/// The server address field's value, filled in on the client.
///
/// The stored address lives in client storage the server cannot read, so a
/// server render of `backend_url()` wrote its own fallback, `localhost`, into
/// the field. Hydration keeps markup it believes matches, so the field went on
/// showing `localhost` after every reload, even though the address had saved
/// and was in use. Both sides render it empty, and the client fills it in.
///
/// With the library on this device, it is the server saved before, if any:
/// the backend in use is the device's own loopback address, which is no use
/// to type anywhere.
fn use_server_address_field() -> Signal<String> {
    let mut url = use_signal(String::new);
    use_effect(move || {
        url.set(if server_url::uses_device_backend() {
            server_url::server_backend_url().unwrap_or_default()
        } else {
            server_url::backend_url()
        })
    });
    url
}

/// Whether this build can keep the library on the device, known on the
/// client only: the server renders the web's answer, which is no.
fn use_device_backend_available() -> Signal<bool> {
    let mut available = use_signal(|| false);
    use_effect(move || available.set(server_url::DEVICE_BACKEND_AVAILABLE));
    available
}

#[component]
fn BackendSetupScreen(connecting: bool, on_saved: EventHandler<()>) -> Element {
    let url = use_server_address_field();
    let session = use_session();
    let device_available = use_device_backend_available();
    let mut error = use_signal(String::new);
    let mut device_error = use_signal(String::new);
    let mut chose_device = use_signal(|| false);
    use_effect(move || {
        let on_device = server_url::uses_device_backend();
        chose_device.set(on_device);
        if on_device && let Err(message) = server_url::device_backend_url() {
            device_error.set(message);
        }
    });
    use_effect(move || {
        if let SessionStatus::Failed(message) = (session.status)() {
            // A first launch still tries the build's default address before
            // anything is chosen. That failure is not the viewer's to fix,
            // so it waits until they pick a backend.
            if !server_url::has_chosen_backend() && !server_url::uses_device_backend() {
                return;
            }
            if *chose_device.peek() {
                device_error.set(message);
            } else {
                error.set(message);
            }
        }
    });

    rsx! {
        // Shown before the app shell exists, so this screen brings its own bar:
        // g3-ui's Header is what keeps content clear of the status bar and the
        // Dynamic Island on a phone.
        Header { title: "Tawny" }
        Content { width: ContentWidth::Readable,
            Stack {
                if device_available() {
                    Card {
                        title: "Use Tawny on this device",
                        heading_level: HeadingLevel::H1,
                        Stack {
                            Text { tone: TextTone::Secondary,
                                "Your subscriptions, playlists and history stay on this device, and it talks to YouTube itself. Nothing to set up, and you can move everything to a server later."
                            }
                            if !device_error().is_empty() {
                                Text { color: Color::Danger, "{device_error}" }
                            }
                            Button {
                                expand: ButtonExpand::Block,
                                loading: connecting && chose_device(),
                                onclick: move |_| {
                                    if connecting { return; }
                                    device_error.set(String::new());
                                    chose_device.set(true);
                                    match server_url::use_device_backend() {
                                        Ok(_) => on_saved.call(()),
                                        Err(message) => device_error.set(message),
                                    }
                                },
                                if connecting && chose_device() { "Starting…" } else { "Start on this device" }
                            }
                        }
                    }
                }
                Card {
                    title: "Connect to a Tawny server",
                    heading_level: if device_available() { HeadingLevel::H2 } else { HeadingLevel::H1 },
                    Stack {
                        Text { tone: TextTone::Secondary,
                            if device_available() {
                                "A server you or someone you trust runs keeps one library for all your devices. Enter its address."
                            } else {
                                "Tawny stores your subscriptions and history on a server you or someone you trust runs. Enter its address to begin."
                            }
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
                            fill: if device_available() { ButtonFill::Outline } else { ButtonFill::Solid },
                            loading: connecting && !chose_device(),
                            onclick: move |_| {
                                if connecting { return; }
                                error.set(String::new());
                                chose_device.set(false);
                                match server_url::set_backend_url(&url()) {
                                    Some(_) => on_saved.call(()),
                                    None => error.set(
                                        "Enter a full address, including http:// or https://.".into(),
                                    ),
                                }
                            },
                            if connecting && !chose_device() { "Connecting…" } else { "Connect" }
                        }
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
                        "You're using a guest account. Add an email and password to keep access to your library and use it on other devices."
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
                            message: Some("This deletes your account, subscriptions, playlists, queue, history and watch progress from this server. Other devices will be signed out.".into()),
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

/// Where the library lives, on the settings page: this device or a server,
/// and moving it from one to the other (see `library_move`).
#[component]
pub fn BackendSettings() -> Element {
    let url = use_server_address_field();
    let device_available = use_device_backend_available();
    let mut error = use_signal(String::new);
    let mut session = use_session();
    let app_state = use_context::<AppState>();
    let mut on_device = use_signal(|| false);
    let mut moving = use_signal(|| false);
    use_effect(move || on_device.set(server_url::uses_device_backend()));

    // The reconnect that ends a move unmounts this section, so nothing is
    // written after it.
    let mut move_to = move |destination: library_move::Destination| {
        if moving() {
            return;
        }
        error.set(String::new());
        moving.set(true);
        spawn(async move {
            match library_move::begin(destination).await {
                Ok(()) => {
                    app_state.forget_account();
                    session.reconnect();
                }
                Err(message) => {
                    error.set(message);
                    moving.set(false);
                }
            }
        });
    };

    rsx! {
        PendingLibrary {}
        if on_device() {
            Card { title: "Library", subtitle: "On this device",
                Stack { gap: Space::Md,
                    Text { tone: TextTone::Secondary,
                        "Your subscriptions, playlists and history are kept on this device. Move them to a Tawny server to use them on your other devices too."
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
                        loading: moving(),
                        onclick: move |_| move_to(library_move::Destination::Server(url())),
                        "Move to this server"
                    }
                }
            }
        } else {
            Card { title: "Server",
                Stack { gap: Space::Md,
                    Input {
                        label: "Server address",
                        value: url,
                        input_type: InputType::Url,
                        error: Some(error()).filter(|message| !message.is_empty()),
                        oninput: move |_| {
                            error.set(String::new());
                        },
                    }

                    Button {
                        fill: ButtonFill::Outline,
                        color: Color::Neutral,
                        expand: ButtonExpand::Block,
                        onclick: move |_| {
                            match server_url::set_backend_url(&url()) {
                                Some(_) => {
                                    app_state.forget_account();
                                    session.reconnect();
                                }
                                None => error.set(
                                    "Enter a full address, including http:// or https://.".into(),
                                ),
                            }
                        },
                        "Save and reconnect"
                    }

                    if device_available() {
                        Text { tone: TextTone::Secondary,
                            "Or keep your library on this device instead, without a server."
                        }
                        Button {
                            fill: ButtonFill::Outline,
                            color: Color::Neutral,
                            expand: ButtonExpand::Block,
                            loading: moving(),
                            onclick: move |_| move_to(library_move::Destination::Device),
                            "Move my library to this device"
                        }
                    }
                }
            }
        }
    }
}

/// A library a move brought here, waiting to be added to this account.
///
/// Not added on its own: the viewer may want it in an account they already
/// have on this server, and signs in to that first.
#[component]
fn PendingLibrary() -> Element {
    let app_state = use_context::<AppState>();
    let mut pending = use_signal(|| None::<library_move::PendingMove>);
    let mut adding = use_signal(|| false);
    use_effect(move || pending.set(library_move::pending()));

    let Some(waiting) = pending() else {
        return rsx! {};
    };
    let from = waiting.source.clone();

    rsx! {
        Card { title: "Add your library",
            Stack { gap: Space::Md,
                Text {
                    "Your library from {from} is ready to add to the account you are using here. To add it to an account you already have, sign in to that account first."
                }
                Button {
                    expand: ButtonExpand::Block,
                    loading: adding(),
                    onclick: move |_| {
                        let Some(waiting) = pending() else { return };
                        if adding() { return; }
                        adding.set(true);
                        spawn(async move {
                            match library_move::finish(waiting).await {
                                Ok(added) => {
                                    pending.set(None);
                                    for change in [
                                        DataChange::Subscriptions,
                                        DataChange::Groups,
                                        DataChange::Playlists,
                                        DataChange::Queue,
                                        DataChange::History,
                                        DataChange::Watched,
                                    ] {
                                        invalidate(change);
                                    }
                                    app_state.show_toast(import_summary(added), Color::Success);
                                }
                                Err(message) => app_state.show_toast(message, Color::Danger),
                            }
                            adding.set(false);
                        });
                    },
                    "Add library"
                }
                Button {
                    fill: ButtonFill::Clear,
                    color: Color::Neutral,
                    expand: ButtonExpand::Block,
                    onclick: move |_| {
                        library_move::discard();
                        pending.set(None);
                    },
                    "Don't add it"
                }
            }
        }
    }
}

/// What an import added, for a toast.
fn import_summary(added: LibraryImport) -> String {
    let count = |n: usize, one: &str, many: &str| match n {
        0 => None,
        1 => Some(format!("1 {one}")),
        n => Some(format!("{n} {many}")),
    };
    let parts = [
        count(added.channels, "channel", "channels"),
        count(added.playlists, "playlist", "playlists"),
        count(added.playlist_videos, "playlist video", "playlist videos"),
        count(added.history, "video in history", "videos in history"),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    match parts.as_slice() {
        [] => "Your library was already here.".into(),
        [only] => format!("Added {only}."),
        [first @ .., last] => format!("Added {} and {last}.", first.join(", ")),
    }
}
