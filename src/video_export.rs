//! Android: saving an exported download into the phone's Downloads folder.
//!
//! The page builds the file (`assets/tawny_export.js`) but cannot write one,
//! so Rust opens it in Downloads through the Kotlin module
//! (`VideoExportPlugin`), hands the descriptor to the local relay as an
//! upload, and the page `PUT`s the bytes there in chunks. Finishing closes the
//! file and makes it visible to other apps' file pickers; abandoning deletes
//! it, so a failed export leaves nothing half-written behind.

use std::{os::fd::FromRawFd, sync::OnceLock};

use serde::Deserialize;

use crate::{android_plugin::Plugin, local_relay};

const PLUGIN_CLASS: &str = "net.gateway3.tawny.localstreams.VideoExportPlugin";

static PLUGIN: OnceLock<Result<Plugin, String>> = OnceLock::new();

/// A file open in Downloads, taking the page's chunks at [`Self::upload_url`].
pub struct PendingExport {
    uri: String,
    upload_url: String,
}

impl PendingExport {
    pub fn upload_url(&self) -> &str {
        &self.upload_url
    }
}

#[derive(Deserialize)]
struct Answer {
    uri: Option<String>,
    fd: Option<i32>,
    path: Option<String>,
    error: Option<String>,
}

/// Opens `Download/Tawny/<file_name>` for the page to write.
pub async fn begin(file_name: String, mime_type: &'static str) -> Result<PendingExport, String> {
    let answer = off_the_ui_thread(move || call("beginFromRust", &[&file_name, mime_type])).await?;
    let (Some(uri), Some(fd)) = (answer.uri, answer.fd) else {
        return Err("Android did not open a file for the export".into());
    };
    // SAFETY: the plugin detached this descriptor from its ParcelFileDescriptor,
    // so nothing else owns or will close it.
    let file = unsafe { std::fs::File::from_raw_fd(fd) };
    let upload_url = local_relay::relay()?.register_upload(file);
    Ok(PendingExport { uri, upload_url })
}

/// Closes the file and publishes it (`keep`), answering where it is, such as
/// "Download/Tawny/Clair de Lune.mp4"; or deletes it.
pub async fn finish(export: PendingExport, keep: bool) -> Result<String, String> {
    off_the_ui_thread(move || {
        let written =
            local_relay::relay().and_then(|relay| relay.finish_upload(&export.upload_url));
        let keep = keep && matches!(written, Ok(bytes) if bytes > 0);
        let answer = call(
            "finishFromRust",
            &[&export.uri, if keep { "true" } else { "false" }],
        )?;
        written?;
        Ok(answer)
    })
    .await?
    .path
    .ok_or_else(|| "Android did not say where the export went".to_string())
}

fn call(method: &str, arguments: &[&str]) -> Result<Answer, String> {
    let text = PLUGIN
        .get_or_init(|| Plugin::construct(PLUGIN_CLASS, "exporter"))
        .as_ref()
        .map_err(Clone::clone)?
        .call(method, arguments)?;
    let answer = serde_json::from_str::<Answer>(&text)
        .map_err(|error| format!("The exporter answered something unreadable: {error}"))?;
    match answer.error {
        Some(error) => Err(error),
        None => Ok(answer),
    }
}

/// Runs `work` on a thread of its own. Finishing flushes a file of hundreds
/// of megabytes, and a native build's UI thread is the WebView's.
async fn off_the_ui_thread<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("tawny-export".into())
        .spawn(move || {
            let _ = sender.send(work());
        })
        .map_err(|error| format!("Could not start the export: {error}"))?;
    receiver
        .await
        .map_err(|_| "The export stopped".to_string())?
}
