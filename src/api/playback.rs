use super::*;

#[get(
    "/api/v1/playback/{video_id}?prefer_sabr&fresh",
    state: dioxus::fullstack::extract::State<crate::server::AppServerState>
)]
pub async fn resolve_playback(
    video_id: String,
    prefer_sabr: bool,
    fresh: bool,
) -> Result<PlaybackSession, PlaybackFailure> {
    // A typed error, so the player learns whose problem it is and what fixes
    // it rather than a flattened message.
    state.playback_session(&video_id, prefer_sabr, fresh).await
}

/// What the typed server function error needs. Failures the resolver never
/// saw - the request not arriving, or not decoding - are classified here.
impl From<ServerFnError> for PlaybackFailure {
    fn from(error: ServerFnError) -> Self {
        match &error {
            ServerFnError::Request(_) | ServerFnError::StreamError(_) => PlaybackFailure::new(
                PlaybackFailureOrigin::Setup,
                "The Tawny server could not be reached.",
            )
            .remedy("Check your connection and that the server is running, then try again.")
            .retryable(),
            ServerFnError::ServerError { .. } => PlaybackFailure::new(
                PlaybackFailureOrigin::Unknown,
                "The server could not start playback.",
            )
            .remedy("Check the server log for the cause."),
            _ => PlaybackFailure::new(
                PlaybackFailureOrigin::Bug,
                "The app and the server disagreed about the playback request.",
            )
            .remedy(
                "Reload the app, since it may be older than the server. If it persists, report \
                 it with the detail below.",
            ),
        }
        .detail(error.to_string())
    }
}

impl dioxus::fullstack::AsStatusCode for PlaybackFailure {
    fn as_status_code(&self) -> dioxus::fullstack::StatusCode {
        use dioxus::fullstack::StatusCode;
        match self.origin {
            // The upstream refused or a dependency is down: a gateway failure.
            PlaybackFailureOrigin::Youtube | PlaybackFailureOrigin::Setup => {
                StatusCode::BAD_GATEWAY
            }
            PlaybackFailureOrigin::Bug | PlaybackFailureOrigin::Unknown => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        }
    }
}

/// SponsorBlock segments for one video.
///
/// The categories come from the client because they are the viewer's settings,
/// and asking for the ones they ignore would fetch data only to discard it.
/// Authenticated like the rest of the library surface: this instance is not an
/// open SponsorBlock proxy.
#[get(
    "/api/v1/videos/{video_id}/sponsor?categories",
    state: dioxus::fullstack::extract::State<crate::server::AppServerState>,
    _owner: crate::auth::Owner
)]
pub async fn get_sponsor_segments(
    video_id: String,
    categories: String,
) -> Result<Vec<SponsorSegment>> {
    let categories = categories
        .split(',')
        .filter(|name| !name.is_empty())
        .filter_map(crate::models::SponsorCategory::from_api_name)
        .collect::<Vec<_>>();
    Ok(state
        .sponsor_segments(&video_id, &categories)
        .await
        .map_err(server_error)?)
}

// ---------------------------------------------------------------------------
// The viewer
//
// One account's own state, and the videos each screen shows. Reads are
// screen-sized; nothing here returns the catalog. Writes are "set to": a
// device showing stale state sends what it saw, and a toggle would undo
// another device's change.
//
// Complex arguments go over POST, since a GET query string cannot carry a
// struct; these reads are per-viewer and never cached anywhere but the
// device, so nothing is lost.
// ---------------------------------------------------------------------------
