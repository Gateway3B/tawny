use super::*;

/// Mint an anonymous account. Called once, on a first launch.
///
/// Deliberately unauthenticated - this is where a viewer gets their first
/// credential, so requiring one would be circular.
#[g3_auth::public]
#[post(
    "/api/v1/auth/guest",
    state: dioxus::fullstack::extract::State<crate::server::AppServerState>,
    crate::auth::SessionContext { auth_session, .. }: crate::auth::SessionContext
)]
pub async fn create_guest_account() -> Result<Account> {
    let account = state
        .accounts()
        .create_guest()
        .await
        .map_err(|error| ServerFnError::new(error.to_string()))?;
    state
        .seed_new_account(&account.id)
        .await
        .map_err(server_error)?;
    crate::auth::sign_in_session(&auth_session, &account);
    Ok(account)
}

/// Promote the calling guest into a real account, keeping its library.
#[post(
    "/api/v1/auth/register",
    state: dioxus::fullstack::extract::State<crate::server::AppServerState>,
    owner: crate::auth::Owner
)]
pub async fn register_account(credentials: Credentials) -> Result<Account> {
    Ok(state
        .accounts()
        .register(&owner.0, &credentials.email, &credentials.password)
        .await
        .map_err(|error| ServerFnError::new(error.to_string()))?)
}

/// Sign in to an existing account, abandoning whatever session was held.
///
/// Public: a device whose session expired signs back in from here.
#[g3_auth::public]
#[post(
    "/api/v1/auth/sign-in",
    state: dioxus::fullstack::extract::State<crate::server::AppServerState>,
    crate::auth::SessionContext { auth_session, .. }: crate::auth::SessionContext
)]
pub async fn sign_in_to_account(credentials: Credentials) -> Result<Account> {
    let account = state
        .accounts()
        .sign_in(&credentials.email, &credentials.password)
        .await
        .map_err(|error| ServerFnError::new(error.to_string()))?;
    auth_session.logout_user();
    crate::auth::sign_in_session(&auth_session, &account);
    Ok(account)
}

/// Drop this device's session. Other devices keep theirs.
///
/// Public: signing out of a session that already expired has to succeed, not
/// strand the device on an error.
#[g3_auth::public]
#[post(
    "/api/v1/auth/sign-out",
    crate::auth::SessionContext { auth_session, .. }: crate::auth::SessionContext
)]
pub async fn sign_out_of_account() -> Result<()> {
    auth_session.logout_user();
    Ok(())
}

/// Who the caller is, used on launch to restore the cookie-backed session.
#[get(
    "/api/v1/auth/account",
    state: dioxus::fullstack::extract::State<crate::server::AppServerState>,
    owner: crate::auth::Owner
)]
pub async fn current_account() -> Result<Account> {
    Ok(state
        .accounts()
        .find(&owner.0)
        .await
        .map_err(|error| ServerFnError::new(error.to_string()))?)
}

/// Permanently remove the caller's account and library. Other devices lose
/// access because their sessions can no longer resolve an account row.
#[post(
    "/api/v1/auth/delete",
    state: dioxus::fullstack::extract::State<crate::server::AppServerState>,
    owner: crate::auth::Owner,
    crate::auth::SessionContext { auth_session, .. }: crate::auth::SessionContext
)]
pub async fn delete_account() -> Result<()> {
    state
        .accounts()
        .delete(&owner.0)
        .await
        .map_err(server_error)?;
    auth_session.logout_user();
    Ok(())
}
