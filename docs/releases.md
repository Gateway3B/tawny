# Releases and app updates

Tawny's Android and Windows apps update themselves from this repository's
[releases](https://github.com/Gateway3B/tawny/releases). Settings → App
updates checks
`https://github.com/Gateway3B/tawny/releases/latest/download/latest.json`
(GitHub redirects it to the `latest.json` asset of the latest release),
downloads the build for the device, checks it against the release key
compiled into the app, and installs it. The repository is public, so
nothing needs a token. The updater itself is g3-native-plugins' `updater`
feature (and `updater-install` on Android, for installing an APK); its
README ("App updates") covers what each platform does.

| Platform | Release file | Installs by |
| --- | --- | --- |
| Android | `Tawny_<version>.apk` | Android's installer. Silent on Android 12+ once the user has let Tawny install apps; otherwise the system asks |
| Windows | `Tawny_<version>_x64-setup.exe` (first install) and `…-setup.zip` (updates) | Tawny closes, the installer runs silently, Tawny reopens |

iOS, macOS and Linux get no release yet, so Settings shows no updates there.

Releases up to 0.1.2 were published elsewhere and have been removed; every
install has since updated to a build that reads this repository.

## Cutting a release

Releases are published from a workstation, with `~/.g3-release/tawny-updater.key`,
`gh` logged in with push access here, and the g3techhq/releases checkout
beside this one (`../releases`, for its `scripts/publish-local.mjs`). No
workflow runs on the tag.

1. Raise `version` in `Cargo.toml` (and the `tawny` entry's version in
   `Cargo.lock`: commit only that line, not a lock rewritten by local
   `[patch]` overrides). The app offers a release only when it is newer than
   its own version, and Android's `versionCode` comes from it
   (`major * 1000000 + minor * 1000 + patch`). Commit and push `main`.
2. Build the two files from that commit:

   ```bash
   dx bundle --android --release --no-default-features --fullstack false      --package-types apk --target aarch64-linux-android
   dx bundle --desktop --release --no-default-features --fullstack false      --package-types nsis --package-types updater
   ```

   and put them in one folder as `Tawny_<x.y.z>.apk`,
   `Tawny_<x.y.z>_x64-setup.exe` and `Tawny_<x.y.z>_x64-setup.zip` (the
   `updater` package type wraps the installer in that zip).
3. Publish:

   ```bash
   just publish <x.y.z> <folder> <notes.md>          # add --dry-run to check first
   ```

   which runs:

   ```bash
   node ../releases/scripts/publish-local.mjs --repo Gateway3B/tawny --manifest asset      --app tawny --name Tawny --version <x.y.z>      --key ~/.g3-release/tawny-updater.key --public-key src/updater.rs      --file android=<folder>/Tawny_<x.y.z>.apk      --file windows-x86_64-nsis=<folder>/Tawny_<x.y.z>_x64-setup.zip      --attach <folder>/Tawny_<x.y.z>_x64-setup.exe      --source Gateway3B/tawny@<HEAD> --notes @<notes.md>
   ```

   It signs both files, creates the draft release `v<x.y.z>` on that commit
   with the files, their signatures, the setup `.exe` and `latest.json`,
   checks they are all there, and only then publishes it as the latest
   release, which is when apps see it. It refuses a version that is not
   newer than the one the URL above serves, a tag that exists, and a key that
   is not the one in `src/updater.rs`.

The release builds carry no `SERVER_URL`: one build for any server, set up on
first launch (see [mobile.md](mobile.md#the-backend-address)).

A dev APK built this way is debug-signed (the workstation's
`~/.android/debug.keystore`), so it installs over `dx serve` and `adb`
builds, but not over a release-keystore build, and the other way round.

## The release key

Tawny's release key is `~/.g3-release/tawny-updater.key` (key id
`9A1942D685589173`). The app installs only files it signed; its public half is
`UPDATER_PUBLIC_KEY` in `src/updater.rs`. Losing it strands every install,
since an app keeps trusting the key it was built with: keep an offline copy.
To replace it, ship one release signed with the old key whose app carries the
new public key. A new key comes from the releases repository:
`node scripts/release-signing.mjs keygen <folder> tawny`.

Tawny 0.1.2 changed the key (to `9A1942D685589173`). Installs from before
0.1.2 trust the old key and refuse 0.1.2, so they must install it by hand
once; from then on they update themselves.

## Installing for the first time

From the latest release's page, https://github.com/Gateway3B/tawny/releases/latest:

- **Android**: download the APK on the phone and open it. Android asks to
  allow the browser or file manager to install apps. A phone that has a
  Tawny signed with a different key (a debug build, say) must uninstall it
  first.
- **Windows**: run the `-setup.exe`. It installs for the current user, under
  `%LOCALAPPDATA%\Programs\Tawny`. It is not code-signed, so SmartScreen
  warns the first time.
