//! Public installation data only. Account information and credentials never
//! enter this store; losing it costs a refresh, not user data.
fn allowed(key: &str) -> bool {
    matches!(key, super::CACHE_KEY | super::DISMISSALS_KEY)
}

pub async fn load(key: &str) -> Option<String> {
    if !allowed(key) {
        return None;
    }
    #[cfg(any(target_os = "android", target_os = "ios"))]
    {
        g3_native_plugins::KeyValueStore::new()
            .get(key)
            .ok()
            .flatten()
    }
    #[cfg(all(
        not(any(target_os = "android", target_os = "ios")),
        any(target_arch = "wasm32", feature = "desktop")
    ))]
    {
        // The desktop webview has localStorage too. Values cross the eval
        // channel as data, never as interpolated JavaScript.
        let mut eval = dioxus::document::eval(
            "const key = await dioxus.recv(); try { dioxus.send(localStorage.getItem(key)); } catch (_) { dioxus.send(null); }",
        );
        eval.send(key).ok()?;
        eval.recv::<Option<String>>().await.ok().flatten()
    }
    #[cfg(all(
        not(any(target_os = "android", target_os = "ios")),
        not(any(target_arch = "wasm32", feature = "desktop"))
    ))]
    None
}

pub async fn save(key: &str, value: &str) {
    if !allowed(key) {
        return;
    }
    #[cfg(any(target_os = "android", target_os = "ios"))]
    {
        let _ = g3_native_plugins::KeyValueStore::new().set(key, value);
    }
    #[cfg(all(
        not(any(target_os = "android", target_os = "ios")),
        any(target_arch = "wasm32", feature = "desktop")
    ))]
    {
        let mut eval = dioxus::document::eval(
            "const [key, value] = await dioxus.recv(); try { localStorage.setItem(key, value); } catch (_) {} dioxus.send(true);",
        );
        if eval.send((key, value)).is_ok() {
            let _ = eval.recv::<bool>().await;
        }
    }
    #[cfg(all(
        not(any(target_os = "android", target_os = "ios")),
        not(any(target_arch = "wasm32", feature = "desktop"))
    ))]
    let _ = value;
}

#[cfg(test)]
mod tests {
    #[test]
    fn storage_accepts_only_the_two_public_tawny_keys() {
        assert!(super::allowed(super::super::CACHE_KEY));
        assert!(super::allowed(super::super::DISMISSALS_KEY));
        for key in [
            "g3tech:tawny:account",
            "g3tech:other:catalog:v1",
            "token",
            "",
        ] {
            assert!(!super::allowed(key));
        }
    }
}
