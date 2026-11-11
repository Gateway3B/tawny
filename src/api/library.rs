use super::*;

/// This account's follows, playlists, groups, queue and history.
#[get(
    "/api/v1/viewer",
    state: dioxus::fullstack::extract::State<crate::server::AppServerState>,
    owner: crate::auth::Owner
)]
pub async fn get_viewer() -> Result<Viewer> {
    Ok(state.viewer(&owner.0).await.map_err(server_error)?)
}

/// Counts of what this account has changed, from any device, since the
/// server started. Asked for often, so it touches nothing but memory: a
/// device compares it with what it last saw and refetches only what moved.
#[get(
    "/api/v1/changes",
    state: dioxus::fullstack::extract::State<crate::server::AppServerState>,
    owner: crate::auth::Owner
)]
pub async fn get_library_changes() -> Result<LibraryChanges> {
    Ok(state.library_changes(&owner.0))
}

/// One page of the subscription feed.
#[post(
    "/api/v1/feed",
    state: dioxus::fullstack::extract::State<crate::server::AppServerState>,
    owner: crate::auth::Owner
)]
pub async fn get_feed_page(query: FeedQuery, page: usize) -> Result<FeedPage> {
    Ok(state
        .feed_page(&owner.0, &query, page)
        .await
        .map_err(server_error)?)
}

/// Videos by id, in the order asked: the queue, history.
#[post(
    "/api/v1/videos/by-id",
    state: dioxus::fullstack::extract::State<crate::server::AppServerState>,
    owner: crate::auth::Owner
)]
pub async fn get_videos(ids: Vec<String>) -> Result<Vec<Video>> {
    Ok(state
        .videos_by_id(&owner.0, &ids)
        .await
        .map_err(server_error)?)
}

/// One page of what YouTube recommended beside the videos this account
/// watched lately: the search page with nothing typed.
#[get(
    "/api/v1/recommendations?page",
    state: dioxus::fullstack::extract::State<crate::server::AppServerState>,
    owner: crate::auth::Owner
)]
pub async fn get_recommendations(page: usize) -> Result<FeedPage> {
    Ok(state
        .recommendations(&owner.0, page)
        .await
        .map_err(server_error)?)
}

/// A playlist and its videos, or `None` when this account has no such
/// playlist.
#[get(
    "/api/v1/playlists/{playlist_id}",
    state: dioxus::fullstack::extract::State<crate::server::AppServerState>,
    owner: crate::auth::Owner
)]
pub async fn get_playlist(playlist_id: String) -> Result<Option<PlaylistContents>> {
    Ok(state
        .playlist_contents(&owner.0, &playlist_id)
        .await
        .map_err(server_error)?)
}

/// Every playlist with its first few videos.
#[get(
    "/api/v1/playlists",
    state: dioxus::fullstack::extract::State<crate::server::AppServerState>,
    owner: crate::auth::Owner
)]
pub async fn get_playlist_previews() -> Result<Vec<PlaylistPreview>> {
    Ok(state
        .playlist_previews(&owner.0, 3)
        .await
        .map_err(server_error)?)
}

#[post(
    "/api/v1/subscriptions",
    state: dioxus::fullstack::extract::State<crate::server::AppServerState>,
    owner: crate::auth::Owner
)]
pub async fn set_subscriptions(changes: Vec<SubscriptionChange>) -> Result<()> {
    state
        .set_subscriptions(&owner.0, changes)
        .await
        .map_err(server_error)?;
    state.note_change(&owner.0, LibraryChange::Subscriptions);
    Ok(())
}

/// Create a playlist, or rename one.
#[post(
    "/api/v1/playlists/save",
    state: dioxus::fullstack::extract::State<crate::server::AppServerState>,
    owner: crate::auth::Owner
)]
pub async fn save_playlist(playlist_id: String, name: String) -> Result<Playlist> {
    let done = state
        .save_playlist(&owner.0, &playlist_id, &name)
        .await
        .map_err(server_error)?;
    state.note_change(&owner.0, LibraryChange::Playlists);
    Ok(done)
}

/// Put the playlists in the order given.
#[post(
    "/api/v1/playlists/reorder",
    state: dioxus::fullstack::extract::State<crate::server::AppServerState>,
    owner: crate::auth::Owner
)]
pub async fn reorder_playlists(playlist_ids: Vec<String>) -> Result<()> {
    state
        .reorder_playlists(&owner.0, &playlist_ids)
        .await
        .map_err(server_error)?;
    state.note_change(&owner.0, LibraryChange::Playlists);
    Ok(())
}

#[post(
    "/api/v1/playlists/membership",
    state: dioxus::fullstack::extract::State<crate::server::AppServerState>,
    owner: crate::auth::Owner
)]
pub async fn set_in_playlist(
    playlist_id: String,
    video_id: String,
    member: bool,
    position: Option<usize>,
) -> Result<bool> {
    let done = state
        .set_in_playlist(&owner.0, &playlist_id, &video_id, member, position)
        .await
        .map_err(server_error)?;
    state.note_change(&owner.0, LibraryChange::Playlists);
    Ok(done)
}

#[post(
    "/api/v1/playlists/remove-watched",
    state: dioxus::fullstack::extract::State<crate::server::AppServerState>,
    owner: crate::auth::Owner
)]
pub async fn remove_watched_from_playlist(playlist_id: String) -> Result<usize> {
    let done = state
        .remove_watched_from_playlist(&owner.0, &playlist_id)
        .await
        .map_err(server_error)?;
    state.note_change(&owner.0, LibraryChange::Playlists);
    Ok(done)
}

#[post(
    "/api/v1/playlists/delete",
    state: dioxus::fullstack::extract::State<crate::server::AppServerState>,
    owner: crate::auth::Owner
)]
pub async fn delete_playlist(playlist_id: String) -> Result<()> {
    state
        .delete_playlist(&owner.0, &playlist_id)
        .await
        .map_err(server_error)?;
    state.note_change(&owner.0, LibraryChange::Playlists);
    state.note_change(&owner.0, LibraryChange::Queue);
    Ok(())
}

/// Watch progress and the audio-only choice, for the videos given.
#[post(
    "/api/v1/progress",
    state: dioxus::fullstack::extract::State<crate::server::AppServerState>,
    owner: crate::auth::Owner
)]
pub async fn save_progress(progress: Vec<VideoProgress>) -> Result<()> {
    let watched_changed = state
        .save_progress(&owner.0, &progress)
        .await
        .map_err(server_error)?;
    state.note_change(&owner.0, LibraryChange::Progress);
    if watched_changed {
        state.note_change(&owner.0, LibraryChange::Watched);
    }
    Ok(())
}

/// This account's saved place in one video, read fresh when it starts playing.
///
/// Lists show the position they were fetched with, which on a second device
/// can be older than where the viewer stopped on the first.
#[get(
    "/api/v1/progress/{video_id}",
    state: dioxus::fullstack::extract::State<crate::server::AppServerState>,
    owner: crate::auth::Owner
)]
pub async fn get_video_progress(video_id: String) -> Result<Option<VideoProgress>> {
    Ok(state
        .video_progress(&owner.0, &video_id)
        .await
        .map_err(server_error)?)
}

/// Create or replace subscription groups, by id.
#[post(
    "/api/v1/groups/save",
    state: dioxus::fullstack::extract::State<crate::server::AppServerState>,
    owner: crate::auth::Owner
)]
pub async fn save_groups(groups: Vec<SubscriptionGroup>) -> Result<()> {
    state
        .save_groups(&owner.0, &groups)
        .await
        .map_err(server_error)?;
    state.note_change(&owner.0, LibraryChange::Groups);
    Ok(())
}

#[post(
    "/api/v1/groups/membership",
    state: dioxus::fullstack::extract::State<crate::server::AppServerState>,
    owner: crate::auth::Owner
)]
pub async fn set_in_group(group_id: String, channel_id: String, member: bool) -> Result<bool> {
    let done = state
        .set_in_group(&owner.0, &group_id, &channel_id, member)
        .await
        .map_err(server_error)?;
    state.note_change(&owner.0, LibraryChange::Groups);
    Ok(done)
}

/// Several channels at once, as the group suggestions add them.
#[post(
    "/api/v1/groups/membership/many",
    state: dioxus::fullstack::extract::State<crate::server::AppServerState>,
    owner: crate::auth::Owner
)]
pub async fn set_many_in_group(
    group_id: String,
    channel_ids: Vec<String>,
    member: bool,
) -> Result<bool> {
    let done = state
        .set_many_in_group(&owner.0, &group_id, &channel_ids, member)
        .await
        .map_err(server_error)?;
    state.note_change(&owner.0, LibraryChange::Groups);
    Ok(done)
}

/// Followed channels that look like they belong in a group, best first.
///
/// The first call on a server fetches a 90 MB model and reads every followed
/// channel, which can take a minute; later calls only read what changed.
#[post(
    "/api/v1/groups/suggest",
    state: dioxus::fullstack::extract::State<crate::server::AppServerState>,
    owner: crate::auth::Owner
)]
pub async fn suggest_group_channels(group_id: String) -> Result<Vec<GroupSuggestion>> {
    Ok(state
        .suggest_group_channels(&owner.0, &group_id)
        .await
        .map_err(server_error)?)
}

#[post(
    "/api/v1/groups/rename",
    state: dioxus::fullstack::extract::State<crate::server::AppServerState>,
    owner: crate::auth::Owner
)]
pub async fn rename_group(group_id: String, name: String) -> Result<()> {
    state
        .rename_group(&owner.0, &group_id, &name)
        .await
        .map_err(server_error)?;
    state.note_change(&owner.0, LibraryChange::Groups);
    Ok(())
}

#[post(
    "/api/v1/groups/delete",
    state: dioxus::fullstack::extract::State<crate::server::AppServerState>,
    owner: crate::auth::Owner
)]
pub async fn delete_group(group_id: String) -> Result<()> {
    state
        .delete_group(&owner.0, &group_id)
        .await
        .map_err(server_error)?;
    state.note_change(&owner.0, LibraryChange::Groups);
    Ok(())
}

#[post(
    "/api/v1/queue",
    state: dioxus::fullstack::extract::State<crate::server::AppServerState>,
    owner: crate::auth::Owner
)]
pub async fn set_queue(queue: Vec<String>) -> Result<()> {
    state
        .set_queue(&owner.0, &queue)
        .await
        .map_err(server_error)?;
    state.note_change(&owner.0, LibraryChange::Queue);
    Ok(())
}

#[post(
    "/api/v1/history/record",
    state: dioxus::fullstack::extract::State<crate::server::AppServerState>,
    owner: crate::auth::Owner
)]
pub async fn record_history(video_id: String) -> Result<()> {
    state
        .record_history(&owner.0, &video_id)
        .await
        .map_err(server_error)?;
    state.note_change(&owner.0, LibraryChange::History);
    Ok(())
}

#[post(
    "/api/v1/history/clear",
    state: dioxus::fullstack::extract::State<crate::server::AppServerState>,
    owner: crate::auth::Owner
)]
pub async fn clear_history() -> Result<()> {
    state.clear_history(&owner.0).await.map_err(server_error)?;
    state.note_change(&owner.0, LibraryChange::History);
    Ok(())
}

/// Everything this account keeps, to move it to another backend: from this
/// device to a server, or back. See `server::library_move`.
#[get(
    "/api/v1/library/export",
    state: dioxus::fullstack::extract::State<crate::server::AppServerState>,
    owner: crate::auth::Owner
)]
pub async fn export_library() -> Result<LibraryExport> {
    Ok(state.export_library(&owner.0).await.map_err(server_error)?)
}

/// Add a library exported elsewhere to this account. It merges and never
/// removes, so importing the same library twice changes nothing.
#[post(
    "/api/v1/library/import",
    state: dioxus::fullstack::extract::State<crate::server::AppServerState>,
    owner: crate::auth::Owner
)]
pub async fn import_library(library: LibraryExport) -> Result<LibraryImport> {
    Ok(state
        .import_library(&owner.0, library)
        .await
        .map_err(server_error)?)
}
