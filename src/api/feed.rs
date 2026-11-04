use super::*;

#[post(
    "/api/v1/feed/refresh",
    state: dioxus::fullstack::extract::State<crate::server::AppServerState>,
    owner: crate::auth::Owner
)]
pub async fn refresh_subscription_feed() -> Result<FeedRefreshResult> {
    Ok(state.refresh_feed(&owner.0).await.map_err(server_error)?)
}

/// Resolve exact metadata for the cards currently on screen.
///
/// RSS has no runtime, so the feed, a playlist, and anything else built from
/// cached rows all show videos whose length is unknown - and a duration filter
/// cannot speak for those. Keeping this separate from the full feed refresh
/// lets a page fill in what it is showing without reconciling any channels.
#[post(
    "/api/v1/videos/durations",
    state: dioxus::fullstack::extract::State<crate::server::AppServerState>,
    owner: crate::auth::Owner
)]
pub async fn hydrate_video_durations(video_ids: Vec<String>) -> Result<Vec<Video>> {
    Ok(state
        .hydrate_video_durations(&owner.0, video_ids)
        .await
        .map_err(server_error)?)
}

// ---------------------------------------------------------------------------
// Accounts
//
// Axum owns the opaque cookie and restores the account before each handler.
// The client never receives a session identifier to persist or replay.
//
// Errors come back as their rendered message. Field-level marking is done on
// the client with the shared validators in `models`, before the request is
// made; by the time the server refuses something, the only useful thing left to
// say is why.
// ---------------------------------------------------------------------------
