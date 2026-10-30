//! Accounts, sessions, and who the client is acting as.

// Accounts and the cookie-backed Axum session: server only.
#[cfg(feature = "server")]
mod accounts;
#[cfg(feature = "server")]
mod sessions;
// The client's view of its session, established before the UI asks for data.
mod session;

#[cfg(feature = "server")]
pub use accounts::*;
pub use session::*;
#[cfg(feature = "server")]
pub(crate) use sessions::prepare as prepare_sessions;
