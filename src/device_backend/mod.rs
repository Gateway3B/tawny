//! Tawny without a server: the backend running inside the app.
//!
//! A viewer who chooses "On this device" gets the same server, running in the
//! app and listening on 127.0.0.1, with its database in the app's own data
//! folder. The client talks to it exactly as it talks to a remote server, so
//! every screen, cache and mutation works unchanged.
//!
//! It is a separate build of this crate, with the `server` feature, loaded at
//! run time (`tawny_backend.dll`, `libtawny_backend.so`). It cannot be compiled
//! into the app itself. Under `server`, a server function call runs the
//! function in place inside a request's context rather than sending a request,
//! and Cargo would switch on g3-cache's and g3-auth's server halves for the app
//! as well, which turn off the client cache. Two builds keep each side what it
//! is. See docs/device-backend.md.
//!
//! - [`serve`] is the library's side: one C function that starts the backend.
//! - [`host`] is the app's side: finding the library, loading it, starting it.

#[cfg(feature = "server")]
mod serve;

#[cfg(all(not(feature = "server"), any(feature = "desktop", feature = "mobile")))]
mod host;

#[cfg(all(not(feature = "server"), any(feature = "desktop", feature = "mobile")))]
pub use host::{data_dir, start};

/// Bumped whenever `tawny_backend_start`'s signature or meaning changes, so an
/// app never calls into a library built for another version of it.
#[cfg_attr(
    not(any(feature = "server", feature = "desktop", feature = "mobile")),
    allow(dead_code, reason = "Only native apps and the backend library use it.")
)]
const ABI_VERSION: u32 = 1;
