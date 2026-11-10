# Releases and app updates

Tawny's Android and Windows apps update themselves from this repository's
GitHub releases. Settings → App updates checks
`https://github.com/Gateway3B/tawny/releases/latest/download/latest.json`,
downloads the build for the device, checks it against the release key
compiled into the app, and installs it. The updater itself is
g3-native-plugins' `updater` feature; its README ("App updates") covers what
each platform does.

| Platform | Release file | Installs by |
| --- | --- | --- |
| Android | `Tawny_<version>.apk` | Android's installer. Silent on Android 12+ once the user has let Tawny install apps; otherwise the system asks |
| Windows | `Tawny_<version>_x64-setup.exe` (first install) and `…-setup.zip` (updates) | Tawny closes, the installer runs silently, Tawny reopens |

iOS, macOS and Linux get no release yet, so Settings shows no updates there.

## Cutting a release

1. Raise `version` in `Cargo.toml`. The app offers a release only when it is
   newer than its own version, and Android's `versionCode` comes from it
   (`major * 1000000 + minor * 1000 + patch`).
2. Commit, then tag and push:

   ```bash
   git tag v0.2.0
   git push origin v0.2.0
   ```

3. **Release** (`.github/workflows/release.yml`) checks the tag matches
   `Cargo.toml`, builds the APK and the Windows installer, signs both, and
   publishes the release with its generated notes. The release is a draft
   until `latest.json` is uploaded, so no app sees it half-made.

The release builds carry no `SERVER_URL`: one build for any server, set up on
first launch (see [mobile.md](mobile.md#the-backend-address)).

## Keys and secrets

Two keys, both stored only as repository secrets:

| Secret | What it is | Why |
| --- | --- | --- |
| `TAWNY_UPDATER_KEY` | The release key's secret half, from `node scripts/release-signing.mjs keygen <folder>` | The app installs only files it signed. Its public half is `UPDATER_PUBLIC_KEY` in `src/updater.rs` |
| `ANDROID_KEYSTORE_BASE64`, `ANDROID_KEYSTORE_PASSWORD`, `ANDROID_KEY_ALIAS`, `ANDROID_KEY_PASSWORD` | The APK signing key, as for [Build Mobile Artifacts](deployment.md#native-mobile-artifacts) | Android installs an update only when it is signed with the same key as the installed app |

The workflow stops before building if `TAWNY_UPDATER_KEY` is missing or does
not belong to the public key in `src/updater.rs`, and the Android job stops if
the keystore is missing: an unsigned APK, or one signed with another key,
could not update anyone.

Losing either secret strands every install: an app keeps trusting the key it
was built with. Keep an offline copy of both. To replace the release key,
ship one release signed with the old key whose app carries the new public
key, then switch the secret.

`scripts/release-signing.mjs` writes keys and signatures in minisign's
format, so `minisign -V -P <public key> -m <file>` checks a release file by
hand. `scripts/release-manifest.mjs` writes `latest.json`.

## Installing for the first time

- **Android**: download the APK from the release on the phone and open it.
  Android asks to allow the browser or file manager to install apps. A phone
  that has a Tawny signed with a different key (a debug build, say) must
  uninstall it first.
- **Windows**: run the `-setup.exe`. It installs for the current user, under
  `%LOCALAPPDATA%\Programs\Tawny`.
