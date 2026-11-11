//! Moving the library between this device and a server: the client's side.
//!
//! 1. [`begin`] exports the library from the backend in use, keeps it in a
//!    file on the device, and switches to the other backend.
//! 2. The app reconnects there, as a guest or with a session it already had.
//!    The viewer can sign in to an account they already have first.
//! 3. Settings offers to add the kept library to that account ([`finish`]),
//!    which merges it in (see `server::library_move`).
//!
//! The backend left behind keeps its copy. Nothing is deleted, so a move that
//! stops halfway can be made again.

use serde::{Deserialize, Serialize};

use crate::{
    api::{export_library, import_library},
    auth::readable,
    models::{LibraryExport, LibraryImport},
    server_url,
};

/// A library exported from one backend, waiting to be added on another.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PendingMove {
    /// Where it came from, for the viewer: "this device" or a server address.
    pub source: String,
    pub library: LibraryExport,
}

pub enum Destination {
    Device,
    Server(String),
}

/// Export the library, keep it, and switch to `destination`. The caller
/// reconnects the session.
pub async fn begin(destination: Destination) -> Result<(), String> {
    // Checked before exporting, so a mistyped address costs nothing.
    if let Destination::Server(url) = &destination
        && server_url::normalize_backend_url(url).is_none()
    {
        return Err("Enter a full address, including http:// or https://.".into());
    }
    let source = if server_url::uses_device_backend() {
        "this device".to_string()
    } else {
        server_url::backend_url()
    };
    let library = export_library()
        .await
        .map_err(|error| readable(&error.to_string()))?;
    if !library.is_empty() {
        store::save(&PendingMove { source, library })?;
    }
    match destination {
        Destination::Device => {
            server_url::use_device_backend()?;
        }
        Destination::Server(url) => {
            server_url::set_backend_url(&url)
                .ok_or("Enter a full address, including http:// or https://.")?;
        }
    }
    Ok(())
}

/// The library waiting to be added, if a move left one.
pub fn pending() -> Option<PendingMove> {
    store::load()
}

/// Add the waiting library to the signed-in account, then forget it.
pub async fn finish(pending: PendingMove) -> Result<LibraryImport, String> {
    let added = import_library(pending.library)
        .await
        .map_err(|error| readable(&error.to_string()))?;
    store::clear();
    Ok(added)
}

/// Forget the waiting library. The backend it came from still has it.
pub fn discard() {
    store::clear();
}

/// A file beside the device library. Only native apps move libraries: the
/// web has no device backend to move to or from.
#[cfg(all(not(feature = "server"), any(feature = "desktop", feature = "mobile")))]
mod store {
    use super::PendingMove;

    fn path() -> Option<std::path::PathBuf> {
        Some(crate::device_backend::data_dir()?.join("pending-library.json"))
    }

    pub fn save(pending: &PendingMove) -> Result<(), String> {
        let path = path().ok_or("This device has no folder for app data.")?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
        }
        let json = serde_json::to_vec(pending).map_err(|error| error.to_string())?;
        std::fs::write(path, json).map_err(|error| format!("Could not keep the library: {error}"))
    }

    pub fn load() -> Option<PendingMove> {
        serde_json::from_slice(&std::fs::read(path()?).ok()?).ok()
    }

    pub fn clear() {
        if let Some(path) = path() {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[cfg(not(all(not(feature = "server"), any(feature = "desktop", feature = "mobile"))))]
mod store {
    use super::PendingMove;

    pub fn save(_pending: &PendingMove) -> Result<(), String> {
        Err("Only the Tawny app can move a library.".into())
    }

    pub fn load() -> Option<PendingMove> {
        None
    }

    pub fn clear() {}
}
