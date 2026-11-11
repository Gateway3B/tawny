# Releases and app updates

Tawny's Android and Windows apps update themselves from
[g3techhq/releases](https://github.com/g3techhq/releases), the release
repository every g3 app shares. Settings → App updates checks
`https://raw.githubusercontent.com/g3techhq/releases/main/apps/tawny/latest.json`,
downloads the build for the device, checks it against the release key
compiled into the app, and installs it. The updater itself is
g3-native-plugins' `updater` feature (and `updater-install` on Android, for
installing an APK); its README ("App updates") covers what each platform
does. The releases repository's README covers how any app joins.

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
   `Cargo.toml` and that the secrets are in place, builds the APK and the
   Windows installer, and calls the shared publish workflow. That signs both,
   publishes them as the release `tawny-v0.2.0` in g3techhq/releases, and
   only then updates `apps/tawny/latest.json`, so no app sees a release
   before its files are there.

The release builds carry no `SERVER_URL`: one build for any server, set up on
first launch (see [mobile.md](mobile.md#the-backend-address)).

## Keys and secrets

All in this repository's secrets:

| Secret | What it is | Why |
| --- | --- | --- |
| `UPDATER_KEY` | Tawny's release key, the secret half (`~/.g3-release/tawny-updater.key`, key id `9A1942D685589173`) | The app installs only files it signed. Its public half is `UPDATER_PUBLIC_KEY` in `src/updater.rs` |
| `RELEASES_TOKEN` | A fine-grained token with *Contents: read and write* on g3techhq/releases only | Publishing there from this repository |
| `ANDROID_KEYSTORE_BASE64`, `ANDROID_KEYSTORE_PASSWORD`, `ANDROID_KEY_ALIAS`, `ANDROID_KEY_PASSWORD` | The APK signing key, as for [Build Mobile Artifacts](deployment.md#native-mobile-artifacts) | Android installs an update only when it is signed with the same key as the installed app |

The workflow stops before building if a secret is missing or `UPDATER_KEY`
does not belong to the public key in `src/updater.rs`: an app would refuse
every file it signed.

Losing `UPDATER_KEY` strands every install, since an app keeps trusting the
key it was built with. Keep an offline copy. To replace it, ship one release
signed with the old key whose app carries the new public key, then switch the
secret. A new key comes from the releases repository:
`node scripts/release-signing.mjs keygen <folder> tawny`.

Tawny 0.1.2 changed the key (to `9A1942D685589173`, which lives on the
workstation as well as in the secret, so a release can be published from
either). Installs from before 0.1.2 trust the old key and refuse 0.1.2, so
they must install it by hand once; from then on they update themselves.

## Publishing from a workstation

The releases repository's `scripts/publish-local.mjs` does what the publish
workflow does, from a machine with `~/.g3-release/tawny-updater.key` (see the
releases README, "Publishing from a workstation"). Build the same files as
**Release** (`dx bundle --android --release --no-default-features --fullstack
false --package-types apk`, and `dx bundle --desktop ... --package-types nsis
--package-types updater`), name them as it does, then from a clean
`../releases` on `main`:

```bash
node scripts/publish-local.mjs --app tawny --name Tawny --version <x.y.z>   --key ~/.g3-release/tawny-updater.key --public-key ../tawny/src/updater.rs   --file android=Tawny_<x.y.z>.apk   --file windows-x86_64-nsis=Tawny_<x.y.z>_x64-setup.zip   --attach Tawny_<x.y.z>_x64-setup.exe   --source Gateway3B/tawny@<sha> --notes @notes.md --prerelease
```

A dev APK built this way is debug-signed (the workstation's
`~/.android/debug.keystore`), so it installs over `dx serve` and `adb`
builds, but not over a release-keystore build, and the other way round.

## Installing for the first time

From the release's page in g3techhq/releases:

- **Android**: download the APK on the phone and open it. Android asks to
  allow the browser or file manager to install apps. A phone that has a
  Tawny signed with a different key (a debug build, say) must uninstall it
  first.
- **Windows**: run the `-setup.exe`. It installs for the current user, under
  `%LOCALAPPDATA%\Programs\Tawny`. It is not code-signed, so SmartScreen
  warns the first time.
