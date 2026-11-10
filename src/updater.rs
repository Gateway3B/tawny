//! Where the native apps look for a newer Tawny, and the key a release must be
//! signed with (g3-native-plugins' `updater`).
//!
//! Releases are GitHub releases of this repository, made by
//! `.github/workflows/release.yml`; `releases/latest` always redirects to the
//! newest one that is not a draft. See docs/releases.md.

use g3_native_plugins::{Updater, UpdaterConfig};

/// The public half of the release key (`scripts/release-signing.mjs keygen`).
/// The release workflow refuses to publish when its secret does not match
/// this, since an app would refuse every file it signed.
pub const UPDATER_PUBLIC_KEY: &str = "untrusted comment: minisign public key E66261DCC1A7F581
RWSB9afB3GFi5g+d7vZFHPNSNFvkxLJWIpaSE23r74b8LTJimW/QqSnb
";

pub const RELEASES: &str =
    "https://github.com/Gateway3B/tawny/releases/latest/download/latest.json";

/// The builds a release carries: the Android APK and the Windows installer.
/// Elsewhere there is nothing to update to, so Settings shows no updates.
pub const SUPPORTED: bool = cfg!(any(
    all(feature = "mobile", target_os = "android"),
    all(feature = "desktop", target_os = "windows"),
));

/// Before launch, so Settings finds the updater ready.
pub fn configure() {
    if !SUPPORTED {
        return;
    }
    let config =
        UpdaterConfig::new(UPDATER_PUBLIC_KEY, env!("CARGO_PKG_VERSION")).endpoint(RELEASES);
    if let Err(error) = Updater::new().configure(config) {
        // The key and version are compiled in, so this is a build mistake.
        eprintln!("The app updater is misconfigured: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_compiled_in_key_and_version_are_accepted() {
        // On a target with no release `configure` returns early, so check the
        // inputs directly.
        let config = UpdaterConfig::new(UPDATER_PUBLIC_KEY, env!("CARGO_PKG_VERSION"));
        assert!(Updater::new().configure(config).is_ok());
        assert!(RELEASES.starts_with("https://github.com/Gateway3B/tawny/releases/latest/"));
    }
}
