# Authentication

Sessions, guests and accounts. The code is `src/auth/` (`accounts.rs` on the server, `session.rs` on the client) and the guard in `src/main.rs`.

Sessions come from `g3-auth`, the same stack as Greenside Partee and Media Mancer: cookie sessions stored in SurrealDB, resolved to an `app_user` (`auth::AppUser`) on every request. First launch creates a guest account, seeded with the default playlists and no follows; registration promotes that row in place. Native clients initialize the Dioxus cookie runtime before launch, so the cookie stays outside application state.

The guard (`g3_auth::require_session`) refuses every server function without a signed-in account unless it is marked `#[g3_auth::public]`: only guest creation, sign-in and sign-out are, and a test pins that list. Every page is public, because the client signs a first visit in from whichever page it opened. Health, the playback proxy and the WebSub callback sit outside the session layers: probes and YouTube's hub have no cookie, and a proxy token is itself the capability, fetched by native media elements without the app's cookie.

Sessions name their account by its record key. Sessions written before g3-auth named it `app_user:<key>`; `database/presync.surql` rewrites those on start, so a signed-in device stays signed in.

## The endpoints

| Server function | Public | Does |
| --- | --- | --- |
| `create_guest_account` | yes | Makes a guest, seeded with the default playlists and no follows, and signs it in |
| `register_account` | no | Promotes the signed-in guest to an email account, in place |
| `sign_in_to_account` | yes | Email and password (argon2 hashes, in `auth/accounts.rs`); replaces the session's account |
| `sign_out_of_account` | yes | Ends the session; the client starts a new guest |
| `current_account` | no | Who is signed in |
| `delete_account` | no | Permanently removes the caller's account and owned library; other devices can no longer authenticate |

Credentials are checked by `validate_email` and `validate_password` in
`src/models/account.rs`, shared by the form and the server.

## On the client

Every read answers `401` until a session exists, so the session is set up
above the UI: `use_session_provider` (`src/auth/session.rs`), called from
`AppStateProvider`, asks for the current account and creates a guest when
there is none, then refetches every cached read once. A new guest's very first
requests can therefore log `401` before the cookie lands; they are retried.

The client cache is keyed to the signed-in account and emptied when it
changes (`g3_cache::set_cache_owner`).

Deletion requires confirmation in Settings, removes playlist relations and
account-owned tables in one transaction, and keeps the shared YouTube catalog.
The client discards pending playback progress, stops playback and clears its
cache before reloading. Auth user caching is disabled so another device cannot
keep using a deleted row.

Guest creation and registration allow ten requests an hour per client;
sign-in allows five a minute. The three endpoints also share a sixty-per-minute
ceiling to bound work when clients rotate cookies. Password hashing and
verification run on blocking workers. Set `CLIENT_IP_HEADER` only when the
app is reachable exclusively through a trusted proxy that overwrites that
header with the real client address. Without it, per-client limits use the
session cookie; the shared ceiling still applies.
