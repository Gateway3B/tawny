//! Client configuration that has to exist *before* the app does.
//!
//! The backend address lives here because it must be known before
//! `dioxus::launch`: the first render already calls server functions. This
//! does not use [`crate::cache::use_persistent_signal`] - that reads
//! localStorage through `document::eval`, and there is no document yet.
//!
//! Each platform reads its own store directly - `localStorage` on web, the
//! g3-native-plugins key-value store on Android, a small JSON file under the
//! user's config directory on desktop and iOS - because that is the only thing
//! available this early.

#![cfg_attr(feature = "server", allow(dead_code))]

/// Baked in at build time: the answer for a packaged client whose viewer never
/// opens settings. The web does not use it; see [`default_backend_url`].
const COMPILED_DEFAULT: Option<&str> = option_env!("SERVER_URL");

#[cfg(target_os = "android")]
const FALLBACK_DEFAULT: &str = "http://127.0.0.1:8080";

#[cfg(not(target_os = "android"))]
const FALLBACK_DEFAULT: &str = "http://localhost:8080";

const BACKEND_URL_KEY: &str = "tawny.backend-url";

/// Trim a URL into the form the server functions want: an origin with no
/// trailing slash.
///
/// A trailing slash is the single most likely thing for someone to paste, and
/// it turns every request path into a double slash - which some reverse proxies
/// answer and others 404, making it look like the server is wrong rather than
/// the input.
pub fn normalize_backend_url(url: &str) -> Option<String> {
    let trimmed = url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return None;
    }
    // Without a scheme a relative URL is resolved against the page, which on a
    // packaged client means a `dioxus://` or `file://` origin and a request
    // that never leaves the device.
    if !trimmed.starts_with("http://") && !trimmed.starts_with("https://") {
        return None;
    }
    if trimmed.split("://").nth(1).is_none_or(str::is_empty) {
        return None;
    }
    Some(trimmed.to_string())
}

/// The backend this client should talk to, stored value first.
pub fn backend_url() -> String {
    stored(BACKEND_URL_KEY)
        .as_deref()
        .and_then(normalize_backend_url)
        .unwrap_or_else(default_backend_url)
}

/// The backend to offer before one was chosen.
///
/// On the web, the origin that served the page: a Tawny server serves its own
/// client, so that is the server, whatever was baked in at build time. The
/// compiled default used to win here, and a build without `SERVER_URL` then
/// offered `localhost` to someone who had opened `https://tawny.example`.
pub fn default_backend_url() -> String {
    #[cfg(all(not(feature = "server"), target_arch = "wasm32"))]
    if let Some(origin) = page_origin() {
        return origin;
    }
    COMPILED_DEFAULT.unwrap_or(FALLBACK_DEFAULT).to_string()
}

#[cfg(all(not(feature = "server"), target_arch = "wasm32"))]
fn page_origin() -> Option<String> {
    let origin = web_sys::window()?.location().origin().ok()?;
    normalize_backend_url(&origin)
}

/// Whether a backend was ever chosen deliberately.
///
/// A first launch has not chosen one, which is what puts the setup screen in
/// front of the app rather than letting it fail against a default nobody picked.
pub fn has_chosen_backend() -> bool {
    stored(BACKEND_URL_KEY)
        .as_deref()
        .and_then(normalize_backend_url)
        .is_some()
}

/// Save the backend and point the server functions at it.
///
/// The running app still holds the previous backend's session and library,
/// so callers send the viewer through a reload as well.
pub fn set_backend_url(url: &str) -> Option<String> {
    let normalized = normalize_backend_url(url)?;
    store(BACKEND_URL_KEY, &normalized);
    install();
    Some(normalized)
}

/// Point this process's server-function client at the configured backend.
///
/// Called from `main` before `dioxus::launch`, and again whenever the backend
/// changes.
pub fn install() {
    // Leaked deliberately: `set_server_url` takes `&'static str`. One small
    // leak per launch or change of backend.
    let url: &'static str = Box::leak(backend_url().into_boxed_str());
    dioxus::fullstack::set_server_url(url);
}

// ---------------------------------------------------------------------------
// Per-platform stores
// ---------------------------------------------------------------------------

#[cfg(all(not(feature = "server"), target_arch = "wasm32"))]
fn local_storage() -> Option<web_sys::Storage> {
    // Every one of these is `None` in a private window with site data blocked,
    // which is a normal state and not an error: the app then behaves like a
    // first launch.
    web_sys::window()?.local_storage().ok()?
}

#[cfg(all(not(feature = "server"), target_arch = "wasm32"))]
fn stored(key: &str) -> Option<String> {
    local_storage()?.get_item(key).ok()?
}

#[cfg(all(not(feature = "server"), target_arch = "wasm32"))]
fn store(key: &str, value: &str) {
    if let Some(storage) = local_storage() {
        let _ = storage.set_item(key, value);
    }
}

/// Android keeps them in the platform key-value store.
///
/// Not the JSON file below: `dirs` finds the config directory through `$HOME`,
/// which an Android app process does not have, so that file was never written
/// and every launch looked like a first launch. The store is usable this early
/// because tao sets up the Android context before it calls `main`.
#[cfg(all(not(feature = "server"), target_os = "android"))]
fn stored(key: &str) -> Option<String> {
    match g3_native_plugins::KeyValueStore::new().get(key) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("tawny: could not read {key}: {error}");
            None
        }
    }
}

#[cfg(all(not(feature = "server"), target_os = "android"))]
fn store(key: &str, value: &str) {
    if let Err(error) = g3_native_plugins::KeyValueStore::new().set(key, value) {
        eprintln!("tawny: could not save {key}: {error}");
    }
}

/// Desktop and iOS keep the same two values in a small JSON file.
///
/// Not the WebView's own localStorage: that is reachable only once the app is
/// running, which is too late for the URL these values configure.
#[cfg(all(
    not(feature = "server"),
    not(target_arch = "wasm32"),
    not(target_os = "android"),
    any(feature = "desktop", feature = "mobile")
))]
fn config_path() -> Option<std::path::PathBuf> {
    let directory = dirs::config_dir()?.join("tawny");
    std::fs::create_dir_all(&directory).ok()?;
    Some(directory.join("client.json"))
}

#[cfg(all(
    not(feature = "server"),
    not(target_arch = "wasm32"),
    not(target_os = "android"),
    any(feature = "desktop", feature = "mobile")
))]
fn read_config() -> serde_json::Map<String, serde_json::Value> {
    config_path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

#[cfg(all(
    not(feature = "server"),
    not(target_arch = "wasm32"),
    not(target_os = "android"),
    any(feature = "desktop", feature = "mobile")
))]
fn stored(key: &str) -> Option<String> {
    read_config()
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
}

#[cfg(all(
    not(feature = "server"),
    not(target_arch = "wasm32"),
    not(target_os = "android"),
    any(feature = "desktop", feature = "mobile")
))]
fn write_config(config: &serde_json::Map<String, serde_json::Value>) {
    let Some(path) = config_path() else {
        return;
    };
    if let Ok(serialized) = serde_json::to_string_pretty(config) {
        let _ = std::fs::write(path, serialized);
    }
}

#[cfg(all(
    not(feature = "server"),
    not(target_arch = "wasm32"),
    not(target_os = "android"),
    any(feature = "desktop", feature = "mobile")
))]
fn store(key: &str, value: &str) {
    let mut config = read_config();
    config.insert(
        key.to_string(),
        serde_json::Value::String(value.to_string()),
    );
    write_config(&config);
}

// No client store: the server binary, which links this module for its shared
// constants, and any host build of the web client - `cargo test` compiles for
// the host, where there is neither a `window` nor a packaged config directory.
// Both behave as a permanent first launch, which is what a build with nowhere
// to persist honestly is.
#[cfg(any(
    feature = "server",
    all(
        not(target_arch = "wasm32"),
        not(feature = "desktop"),
        not(feature = "mobile")
    )
))]
fn stored(_key: &str) -> Option<String> {
    None
}

#[cfg(any(
    feature = "server",
    all(
        not(target_arch = "wasm32"),
        not(feature = "desktop"),
        not(feature = "mobile")
    )
))]
fn store(_key: &str, _value: &str) {}

#[cfg(test)]
mod tests {
    use super::normalize_backend_url;

    #[test]
    fn a_trailing_slash_is_removed_before_it_becomes_a_double_slash() {
        assert_eq!(
            normalize_backend_url("https://tawny.example/"),
            Some("https://tawny.example".into())
        );
        assert_eq!(
            normalize_backend_url("  http://192.168.1.10:8080///  "),
            Some("http://192.168.1.10:8080".into())
        );
    }

    #[test]
    fn a_url_without_a_scheme_is_refused_rather_than_resolved_against_the_page() {
        // On a packaged client the page origin is not http at all, so a
        // schemeless value produces a request that never leaves the device.
        assert_eq!(normalize_backend_url("tawny.example"), None);
        assert_eq!(normalize_backend_url("192.168.1.10:8080"), None);
        assert_eq!(normalize_backend_url("//tawny.example"), None);
        assert_eq!(normalize_backend_url("ftp://tawny.example"), None);
    }

    #[test]
    fn empty_input_is_not_a_backend() {
        assert_eq!(normalize_backend_url(""), None);
        assert_eq!(normalize_backend_url("   "), None);
        assert_eq!(normalize_backend_url("/"), None);
        assert_eq!(normalize_backend_url("https://"), None);
    }

    #[test]
    fn both_supported_schemes_survive() {
        // Plain http is not a mistake here: a self-hosted instance on a LAN is
        // the common case, and refusing it would refuse the main deployment.
        assert_eq!(
            normalize_backend_url("http://localhost:8080"),
            Some("http://localhost:8080".into())
        );
        assert_eq!(
            normalize_backend_url("https://tawny.example"),
            Some("https://tawny.example".into())
        );
    }
}
