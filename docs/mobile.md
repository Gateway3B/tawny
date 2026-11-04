# Android, iOS and desktop

The native apps are the same Rust code built with the `mobile` or `desktop`
feature: Dioxus renders into a system webview, and the app talks to the server
over HTTP. The player's media element runs in that webview, with the same
Shaka transport as the web.

```bash
just dev-android          # emulator or a connected phone
just dev-ios              # simulator (macOS only)
just dev-desktop
```

## The backend address

A native build has no page origin, so the server it talks to is compiled in
(`SERVER_URL`) and can be changed in Settings. `src/server_url.rs` reads the
choice before `dioxus::launch`, because the first render already calls server
functions. Saving a new one repoints the server functions at once and reloads
the app, which still holds the previous backend's session and library; a
packaged build keeps that state across a reload and has to be reopened. The
player's transport fetches from
the webview directly, so it is told the chosen backend too
(`playback_server_url` in `player.rs`).

On the web the default is the page's own origin, since a Tawny server serves
its own client. The address fields on the setup screen and in Settings are
filled in on the client: the saved address is in client storage, and a server
render of it would write the server's fallback (`localhost`) into the field,
which hydration then kept.

## Android

For the Android emulator, run the normal Dioxus command:

```bash
dx serve --android
```

The Android client and development media proxy default to
`http://127.0.0.1:8080`, which matches Dioxus's generated network-security
policy and the emulator's `adb reverse` mapping. Playback URLs, captions, and
session refreshes are all rebased to that API origin inside the Android
WebView. The media proxy also supplies the CORS, private-network, resource, and
range headers needed by Shaka.

Do not substitute `localhost`: Android's policy entries match literal
hostnames, and only `127.0.0.1` is permitted for cleartext development
traffic. Set `SERVER_URL` and `TAWNY_PUBLIC_URL` explicitly when the client and
server use a different reachable HTTPS address, such as a physical device
connecting to a home-lab server.

## Sessions

Native builds initialize the cookie runtime before launch
(`g3_auth::init()`), so the session cookie stays outside application state. See
[authentication.md](authentication.md).

## Builds

**Build Mobile** (`.github/workflows/build-mobile.yml`) produces the Android
and iOS artifacts; see [deployment.md](deployment.md#native-mobile-artifacts).

Release builds use the `android-release` and `ios-release` profiles in
`Cargo.toml` (opt-level 3, fat LTO). Each run sets the Android `versionCode`
and the Apple bundle version to the workflow's run number
(`DX_ANDROID_VERSION_CODE`, `DX_BUNDLE_VERSION`), since every store upload
needs a higher build number than the last.

Before a first store submission:

- **Icons.** iOS uses `[ios] icon`, the opaque 1024x1024
  `assets/icon/icon-1024.png` (the App Store rejects transparency); Android
  generates its launcher icons from `[bundle] icon`, the SVG master. Render
  both again from the master after changing it.
- **Privacy manifest.** dx copies `ios/PrivacyInfo.xcprivacy` into the app.
  Declare any new "required reason" API there before uploading (ITMS-91053).
- **SDK levels.** `target_sdk` is 36, which Play has required since
  31 August 2026; check Play's current rule before uploading.
- **On a device:** rotate and toggle dark mode (Android recreates the
  activity), leave the app in the background for several minutes and resume,
  and select text in a release build.
