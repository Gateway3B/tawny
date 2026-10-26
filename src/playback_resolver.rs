//! Where the player gets a video's playback session: the phone itself when it
//! can (Android, with the setting on), otherwise the server.

use crate::{
    api::resolve_playback,
    models::{PlaybackFailure, PlaybackSession},
};

/// A playback session for `video_id`. With `from_device`, an Android phone
/// resolves it itself first (`local_streams`), so video comes straight from
/// YouTube; any failure there asks the server as before.
pub async fn resolve_session(
    video_id: String,
    prefer_sabr: bool,
    fresh: bool,
    from_device: bool,
) -> Result<PlaybackSession, PlaybackFailure> {
    #[cfg(target_os = "android")]
    if from_device {
        match crate::local_streams::resolve(&video_id, fresh).await {
            Ok(session) => return Ok(session),
            Err(error) => dioxus::logger::tracing::warn!(
                "Resolving {video_id} on this device failed, asking the server: {error}"
            ),
        }
    }
    #[cfg(not(target_os = "android"))]
    let _ = from_device;
    resolve_playback(video_id, prefer_sabr, fresh).await
}

/// Whether this build can resolve playback on the device at all, which is
/// what decides whether Settings offers it.
pub const DEVICE_RESOLVE_AVAILABLE: bool = cfg!(target_os = "android");
