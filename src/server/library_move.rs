//! Moving one account's library between this device and a server.
//!
//! The viewer exports from the backend they are leaving and imports into the
//! one they arrive at, so the same two functions serve both directions.
//!
//! An import merges and never removes. The two sides may both have been used:
//! a phone tried out on its own before being pointed at a server that already
//! knows the viewer. Follows, playlist entries and the queue are unions,
//! history keeps each video's latest play, and a video watched on either side
//! stays watched. Because nothing is removed, running an import again changes
//! nothing, so an interrupted move can simply be repeated.

use super::*;
use crate::models::{
    HISTORY_LIMIT, LibraryChange, LibraryExport, LibraryImport, SubscriptionChange,
    queued_playlist_id,
};

impl AppServerState {
    /// Everything `owner` keeps, with the videos it points at.
    pub async fn export_library(&self, owner: &str) -> Result<LibraryExport> {
        let viewer = self.viewer(owner).await?;
        let progress: Vec<DbVideoProgress> = self
            .db
            .query("SELECT video_id, watched, progress_seconds, audio_only FROM video_progress WHERE owner = type::record($owner)")
            .bind(("owner", owner.to_string()))
            .await?
            .check()?
            .take(0)?;
        let progress = progress
            .into_iter()
            .map(|row| VideoProgress {
                video_id: row.video_id,
                watched: row.watched,
                progress_seconds: row.progress_seconds.max(0) as u64,
                audio_only: row.audio_only,
            })
            .collect::<Vec<_>>();

        let mut video_ids = Vec::new();
        let mut seen = HashSet::new();
        let referenced = viewer
            .playlists
            .iter()
            .flat_map(|playlist| playlist.video_ids.iter())
            .chain(
                viewer
                    .queue
                    .iter()
                    .filter(|entry| queued_playlist_id(entry).is_none()),
            )
            // Not every video with progress: an account that has watched for
            // years has thousands, and the import's request body is capped
            // (2 MB). Progress travels on its own and needs no catalog row.
            .chain(viewer.history.iter().map(|entry| &entry.video_id));
        for id in referenced {
            if seen.insert(id.clone()) {
                video_ids.push(id.clone());
            }
        }
        let videos = self.videos_by_id(owner, &video_ids).await?;

        Ok(LibraryExport {
            subscriptions: viewer.subscriptions,
            groups: viewer.subscription_groups,
            playlists: viewer.playlists,
            queue: viewer.queue,
            history: viewer.history,
            progress,
            videos,
        })
    }

    /// Add `library` to what `owner` already has. See the module notes for
    /// how each part merges.
    pub async fn import_library(
        &self,
        owner: &str,
        library: LibraryExport,
    ) -> Result<LibraryImport> {
        let mut added = LibraryImport::default();
        self.import_videos(&library.videos).await?;

        let follows = self.user_subscriptions(owner).await?;
        let new_follows = library
            .subscriptions
            .into_iter()
            .filter(|channel| {
                !follows
                    .get(&channel.id)
                    .is_some_and(|(followed, _)| *followed)
            })
            .map(|channel| SubscriptionChange {
                content: channel.subscription_content,
                subscribed: true,
                channel,
            })
            .collect::<Vec<_>>();
        added.channels = new_follows.len();
        if !new_follows.is_empty() {
            self.set_subscriptions(owner, new_follows).await?;
        }

        let viewer = self.viewer(owner).await?;
        let groups = merge_groups(&viewer.subscription_groups, library.groups);
        if !groups.is_empty() {
            self.save_groups(owner, &groups).await?;
        }

        for playlist in library.playlists {
            let existing = viewer
                .playlists
                .iter()
                .find(|mine| mine.id == playlist.id)
                .or_else(|| {
                    viewer
                        .playlists
                        .iter()
                        .find(|mine| same_name(&mine.name, &playlist.name))
                });
            let (target, has) = match existing {
                Some(mine) => (mine.id.clone(), mine.video_ids.clone()),
                None => {
                    if playlist.video_ids.is_empty() && is_default_playlist(&playlist.id) {
                        continue;
                    }
                    self.save_playlist(owner, &playlist.id, &playlist.name)
                        .await?;
                    added.playlists += 1;
                    (playlist.id.clone(), Vec::new())
                }
            };
            for video_id in playlist.video_ids {
                if has.contains(&video_id) {
                    continue;
                }
                // At the end, in the order the other side kept them: a new
                // playlist comes out as it was, and an existing one keeps
                // its own order ahead of what was added.
                if self
                    .set_in_playlist(owner, &target, &video_id, true, Some(usize::MAX))
                    .await?
                {
                    added.playlist_videos += 1;
                }
            }
        }

        let _guard = self.sync_lock.lock().await;
        let (queue, history) = self.queue_and_history(owner).await?;
        let mut merged_queue = queue.clone();
        for entry in library.queue {
            if !merged_queue.contains(&entry) {
                merged_queue.push(entry);
            }
        }
        if merged_queue != queue {
            self.write_queue(owner, &merged_queue).await?;
        }
        let merged_history = merge_history(&history, library.history);
        added.history = merged_history
            .iter()
            .filter(|entry| !history.iter().any(|had| had.video_id == entry.video_id))
            .count();
        if merged_history != history {
            self.write_history(owner, &merged_history).await?;
        }
        drop(_guard);

        let ids = library
            .progress
            .iter()
            .map(|entry| entry.video_id.clone())
            .collect::<Vec<_>>();
        let mine: Vec<DbVideoProgress> = self
            .db
            .query("SELECT video_id, watched, progress_seconds, audio_only FROM video_progress WHERE owner = type::record($owner) AND video_id IN $ids")
            .bind(("owner", owner.to_string()))
            .bind(("ids", ids))
            .await?
            .check()?
            .take(0)?;
        let progress = library
            .progress
            .into_iter()
            .filter_map(|theirs| {
                let Some(mine) = mine.iter().find(|row| row.video_id == theirs.video_id) else {
                    return Some(theirs);
                };
                let merged = VideoProgress {
                    video_id: theirs.video_id,
                    watched: mine.watched || theirs.watched,
                    progress_seconds: (mine.progress_seconds.max(0) as u64)
                        .max(theirs.progress_seconds),
                    audio_only: mine.audio_only,
                };
                let changed = merged.watched != mine.watched
                    || merged.progress_seconds != mine.progress_seconds.max(0) as u64;
                changed.then_some(merged)
            })
            .collect::<Vec<_>>();
        self.write_user_progress(owner, &progress).await?;

        for change in [
            LibraryChange::Subscriptions,
            LibraryChange::Groups,
            LibraryChange::Playlists,
            LibraryChange::Queue,
            LibraryChange::History,
            LibraryChange::Watched,
            LibraryChange::Progress,
        ] {
            self.note_change(owner, change);
        }
        Ok(added)
    }

    /// Store the catalog rows this side has not met. Rows it has are newer
    /// than the export's copy, or as new, and are left alone.
    async fn import_videos(&self, videos: &[Video]) -> Result<()> {
        let ids = videos
            .iter()
            .map(|video| video.id.clone())
            .collect::<Vec<_>>();
        let known: Vec<String> = self
            .db
            .query("SELECT VALUE video_id FROM video WHERE video_id IN $ids")
            .bind(("ids", ids))
            .await?
            .check()?
            .take(0)?;
        let known = known.into_iter().collect::<HashSet<_>>();
        for video in videos.iter().filter(|video| !known.contains(&video.id)) {
            // The export's copy carries the exporting viewer's progress, which
            // is per account and travels separately.
            let video = Video {
                progress_seconds: 0,
                watched: false,
                audio_only: false,
                ..video.clone()
            };
            self.upsert_video(&video, video.published_at.clone())
                .await?;
        }
        Ok(())
    }
}

fn same_name(left: &str, right: &str) -> bool {
    left.trim().eq_ignore_ascii_case(right.trim())
}

/// Every account starts with these, so an empty one on the other side is not
/// worth creating where the viewer has deleted theirs.
fn is_default_playlist(id: &str) -> bool {
    matches!(id, "watch-later" | "favorites")
}

/// The groups to save: theirs that this account lacks, and this account's
/// that gain channels from a group of the same name.
fn merge_groups(
    mine: &[SubscriptionGroup],
    theirs: Vec<SubscriptionGroup>,
) -> Vec<SubscriptionGroup> {
    let mut changed: Vec<SubscriptionGroup> = Vec::new();
    for group in theirs {
        let existing = changed
            .iter()
            .chain(mine.iter())
            .find(|candidate| candidate.id == group.id || same_name(&candidate.name, &group.name))
            .cloned();
        match existing {
            Some(mut target) => {
                let before = target.channel_ids.len();
                for channel_id in group.channel_ids {
                    if !target.channel_ids.contains(&channel_id) {
                        target.channel_ids.push(channel_id);
                    }
                }
                if target.channel_ids.len() != before {
                    changed.retain(|candidate| candidate.id != target.id);
                    changed.push(target);
                }
            }
            None => changed.push(group),
        }
    }
    changed
}

/// Both histories, each video once at its latest play, newest first.
fn merge_history(mine: &[HistoryEntry], theirs: Vec<HistoryEntry>) -> Vec<HistoryEntry> {
    let mut merged: Vec<HistoryEntry> = mine.to_vec();
    for entry in theirs {
        match merged.iter_mut().find(|had| had.video_id == entry.video_id) {
            Some(had) if later(&entry.played_at, &had.played_at) => *had = entry,
            Some(_) => {}
            None => merged.push(entry),
        }
    }
    // Stable, so entries without a readable time keep their order.
    merged.sort_by_key(|entry| std::cmp::Reverse(played_at(&entry.played_at)));
    merged.truncate(HISTORY_LIMIT);
    merged
}

fn played_at(value: &str) -> Option<time::OffsetDateTime> {
    time::OffsetDateTime::parse(value, &time::format_description::well_known::Rfc3339).ok()
}

fn later(candidate: &str, than: &str) -> bool {
    played_at(candidate) > played_at(than)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, at: &str) -> HistoryEntry {
        HistoryEntry {
            video_id: id.into(),
            played_at: at.into(),
        }
    }

    fn group(id: &str, name: &str, channels: &[&str]) -> SubscriptionGroup {
        SubscriptionGroup {
            id: id.into(),
            name: name.into(),
            channel_ids: channels.iter().map(|channel| channel.to_string()).collect(),
        }
    }

    #[test]
    fn history_keeps_each_video_once_at_its_latest_play() {
        let mine = vec![
            entry("a", "2026-03-03T00:00:00Z"),
            entry("b", "2026-03-01T00:00:00Z"),
        ];
        let theirs = vec![
            entry("b", "2026-03-04T00:00:00Z"),
            entry("c", "2026-03-02T00:00:00Z"),
            entry("a", "2026-01-01T00:00:00Z"),
        ];
        let merged = merge_history(&mine, theirs);
        let order = merged
            .iter()
            .map(|entry| (entry.video_id.as_str(), entry.played_at.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(
            order,
            vec![
                ("b", "2026-03-04T00:00:00Z"),
                ("a", "2026-03-03T00:00:00Z"),
                ("c", "2026-03-02T00:00:00Z"),
            ]
        );
    }

    #[test]
    fn groups_merge_by_name_and_only_changed_ones_are_saved() {
        let mine = vec![
            group("g1", "Music", &["UCa"]),
            group("g2", "News", &["UCn"]),
        ];
        let theirs = vec![
            group("other-id", "music ", &["UCa", "UCb"]),
            group("g2", "News", &["UCn"]),
            group("g3", "Games", &["UCg"]),
        ];
        let changed = merge_groups(&mine, theirs);
        assert_eq!(
            changed,
            vec![
                group("g1", "Music", &["UCa", "UCb"]),
                group("g3", "Games", &["UCg"])
            ]
        );
    }

    fn video(id: &str) -> Video {
        Video {
            id: id.into(),
            title: format!("Video {id}"),
            channel_id: "UCa".into(),
            channel_name: "Channel".into(),
            thumbnail_url: String::new(),
            published_at: "2026-01-01T00:00:00Z".into(),
            duration_seconds: 600,
            view_count: String::new(),
            progress_seconds: 0,
            watched: false,
            is_live: false,
            is_short: false,
            audio_only: false,
            channel_avatar_url: None,
        }
    }

    fn channel(id: &str) -> Channel {
        Channel {
            id: id.into(),
            name: format!("Channel {id}"),
            handle: format!("@{id}"),
            avatar_url: None,
            subscriber_count: String::new(),
            subscribed: false,
            description: String::new(),
            banner_url: None,
            subscription_content: SubscriptionContent::All,
        }
    }

    fn progress(id: &str, seconds: u64, watched: bool) -> VideoProgress {
        VideoProgress {
            video_id: id.into(),
            watched,
            progress_seconds: seconds,
            audio_only: false,
        }
    }

    /// The move a tester makes: a phone used on its own, then pointed at a
    /// server where they already follow something and kept a playlist.
    #[tokio::test]
    async fn a_device_library_merges_into_an_account_that_has_its_own() {
        let state = AppServerState::initialize().await.unwrap();
        let device = state.accounts().create_guest().await.unwrap().id;
        let server = state.accounts().create_guest().await.unwrap().id;
        // As `create_guest_account` does: Watch Later and Favorites.
        for owner in [&device, &server] {
            state.seed_new_account(owner).await.unwrap();
        }
        for id in ["v1", "v2", "v3"] {
            state
                .upsert_video(&video(id), "2026-01-01".into())
                .await
                .unwrap();
        }

        // The device: two follows, a playlist, history, progress and a queue.
        state
            .set_subscriptions(
                &device,
                ["UCa", "UCb"]
                    .map(|id| SubscriptionChange {
                        channel: channel(id),
                        subscribed: true,
                        content: SubscriptionContent::All,
                    })
                    .to_vec(),
            )
            .await
            .unwrap();
        state
            .save_playlist(&device, "trip", "Road trip")
            .await
            .unwrap();
        for id in ["v1", "v2"] {
            state
                .set_in_playlist(&device, "trip", id, true, None)
                .await
                .unwrap();
        }
        state
            .set_in_playlist(&device, "watch-later", "v3", true, None)
            .await
            .unwrap();
        state.record_history(&device, "v1").await.unwrap();
        state
            .save_progress(
                &device,
                &[progress("v1", 300, false), progress("v2", 50, true)],
            )
            .await
            .unwrap();
        state.set_queue(&device, &["v2".into()]).await.unwrap();

        // The server account: one of the same follows, a playlist of the
        // same name, and further along in v1.
        state
            .set_subscriptions(
                &server,
                vec![SubscriptionChange {
                    channel: channel("UCa"),
                    subscribed: true,
                    content: SubscriptionContent::Videos,
                }],
            )
            .await
            .unwrap();
        state
            .save_playlist(&server, "mine", "road trip")
            .await
            .unwrap();
        state
            .set_in_playlist(&server, "mine", "v2", true, None)
            .await
            .unwrap();
        state
            .save_progress(&server, &[progress("v1", 900, false)])
            .await
            .unwrap();

        let export = state.export_library(&device).await.unwrap();
        // v1 and v2 from the playlist and history, v3 from Watch Later.
        assert_eq!(export.videos.len(), 3);
        let added = state.import_library(&server, export.clone()).await.unwrap();
        assert_eq!(added.channels, 1);
        assert_eq!(added.playlists, 0);
        // v1 into "road trip", v3 into Watch Later.
        assert_eq!(added.playlist_videos, 2);

        let viewer = state.viewer(&server).await.unwrap();
        let follows = viewer
            .subscriptions
            .iter()
            .map(|channel| (channel.id.as_str(), channel.subscription_content))
            .collect::<Vec<_>>();
        // The server's own choice for UCa stands.
        assert!(follows.contains(&("UCa", SubscriptionContent::Videos)));
        assert!(follows.contains(&("UCb", SubscriptionContent::All)));
        assert_eq!(viewer.playlist("mine").unwrap().video_ids, vec!["v2", "v1"]);
        assert_eq!(
            viewer.playlist("watch-later").unwrap().video_ids,
            vec!["v3"]
        );
        assert_eq!(viewer.queue, vec!["v2"]);
        assert_eq!(viewer.history[0].video_id, "v1");

        let v1 = state.video_progress(&server, "v1").await.unwrap().unwrap();
        assert_eq!(v1.progress_seconds, 900);
        assert!(
            state
                .video_progress(&server, "v2")
                .await
                .unwrap()
                .unwrap()
                .watched
        );

        // A second run adds nothing.
        let again = state.import_library(&server, export).await.unwrap();
        assert_eq!(again, LibraryImport::default());
        assert_eq!(state.viewer(&server).await.unwrap(), viewer);
    }
}
