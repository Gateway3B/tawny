//! One account's reads and writes: what it follows, its playlists, queue and
//! history, and the videos each screen shows.
//!
//! The catalog (every channel and video the instance has met) never travels
//! whole. A screen asks for what it shows, and every video handed out carries
//! this viewer's progress and its channel's avatar.
//!
//! Writes are "set to", never "toggle": a device showing stale state sends
//! what it saw, and a toggle would undo another device's change.

use super::*;
use crate::models::{
    DurationFilter, FEED_PAGE_SIZE, FeedFilter, FeedGroup, FeedPage, FeedQuery, HISTORY_LIMIT,
    PlaylistContents, PlaylistPreview, SubscriptionChange, Viewer,
};

/// Every column a `DbVideo` reads.
pub(super) const VIDEO_COLUMNS: &str = "video_id, title, channel_id, channel_name, thumbnail_url, published_at, published_sort, duration_seconds, view_count, is_live, is_short, progress_seconds, watched, audio_only";

/// Every column a `DbChannel` reads.
pub(super) const CHANNEL_COLUMNS: &str = "channel_id, name, handle, avatar_url, subscriber_count, subscribed, subscription_content, description, banner_url";

/// How many unknown lengths one feed page asks to have filled in.
const DURATION_LOOKAHEAD: usize = 48;

#[derive(Debug, Deserialize, SurrealValue)]
struct DbAvatar {
    channel_id: String,
    avatar_url: Option<String>,
}

#[derive(Debug, Deserialize, SurrealValue)]
struct DbCount {
    count: i64,
}

#[derive(Debug, Deserialize, SurrealValue)]
struct DbQueueHistory {
    queue_json: String,
    history_json: String,
}

impl AppServerState {
    // -----------------------------------------------------------------------
    // Reads
    // -----------------------------------------------------------------------

    /// This account's own state: follows, playlists, groups, queue, history.
    pub async fn viewer(&self, owner: &str) -> Result<Viewer> {
        let follows = self.user_subscriptions(owner).await?;
        let followed_ids = follows
            .iter()
            .filter(|(_, (subscribed, _))| *subscribed)
            .map(|(channel_id, _)| channel_id.clone())
            .collect::<Vec<_>>();
        let channels: Vec<DbChannel> = self
            .db
            .query(format!(
                "SELECT {CHANNEL_COLUMNS} FROM channel WHERE channel_id IN $ids ORDER BY name"
            ))
            .bind(("ids", followed_ids))
            .await?
            .check()?
            .take(0)?;
        let subscriptions = channels
            .into_iter()
            .map(Channel::from)
            .map(|mut channel| {
                let (_, content) = follows[&channel.id];
                channel.subscribed = true;
                channel.subscription_content = content;
                // Every screen reads the viewer, and with hundreds of follows
                // the descriptions were most of it. The channel page fetches
                // its own.
                channel.description.clear();
                channel.banner_url = None;
                channel
            })
            .collect();

        let mut playlists: Vec<DbPlaylist> = self
            .db
            .query("SELECT playlist_id, name, video_ids, sort_order FROM playlist WHERE owner = type::record($owner)")
            .bind(("owner", owner.to_string()))
            .await?
            .check()?
            .take(0)?;
        sort_playlists(&mut playlists);
        let groups: Vec<DbSubscriptionGroup> = self
            .db
            .query("SELECT group_id, name, channel_ids FROM subscription_group WHERE owner = type::record($owner) ORDER BY name")
            .bind(("owner", owner.to_string()))
            .await?
            .check()?
            .take(0)?;
        let (queue, history) = self.queue_and_history(owner).await?;

        Ok(Viewer {
            subscriptions,
            playlists: playlists.into_iter().map(Into::into).collect(),
            subscription_groups: groups.into_iter().map(Into::into).collect(),
            queue,
            history,
        })
    }

    async fn queue_and_history(&self, owner: &str) -> Result<(Vec<String>, Vec<HistoryEntry>)> {
        let rows: Vec<DbQueueHistory> = self
            .db
            .query("SELECT queue_json, history_json FROM library_state WHERE owner = type::record($owner) LIMIT 1")
            .bind(("owner", owner.to_string()))
            .await?
            .check()?
            .take(0)?;
        let Some(row) = rows.into_iter().next() else {
            return Ok(Default::default());
        };
        Ok((
            serde_json::from_str(&row.queue_json).unwrap_or_default(),
            serde_json::from_str(&row.history_json).unwrap_or_default(),
        ))
    }

    /// One page of the subscription feed, newest first.
    pub async fn feed_page(&self, owner: &str, query: &FeedQuery, page: usize) -> Result<FeedPage> {
        let viewer_follows = self.user_subscriptions(owner).await?;
        let groups: Vec<DbSubscriptionGroup> = self
            .db
            .query("SELECT group_id, name, channel_ids FROM subscription_group WHERE owner = type::record($owner)")
            .bind(("owner", owner.to_string()))
            .await?
            .check()?
            .take(0)?;
        let in_group = |channel_id: &str| match &query.group {
            FeedGroup::All => true,
            FeedGroup::Ungrouped => !groups
                .iter()
                .any(|group| group.channel_ids.iter().any(|id| id == channel_id)),
            FeedGroup::Group(id) => groups
                .iter()
                .find(|group| &group.group_id == id)
                .is_none_or(|group| group.channel_ids.iter().any(|id| id == channel_id)),
        };

        // Each subscription wants all of a channel's uploads, or only its
        // videos, or only its Shorts.
        let (mut all, mut videos_only, mut shorts_only) = (Vec::new(), Vec::new(), Vec::new());
        for (channel_id, (subscribed, content)) in &viewer_follows {
            if !subscribed || !in_group(channel_id) {
                continue;
            }
            match content {
                SubscriptionContent::All => all.push(channel_id.clone()),
                SubscriptionContent::Videos => videos_only.push(channel_id.clone()),
                SubscriptionContent::Shorts => shorts_only.push(channel_id.clone()),
            }
        }
        let watched = if query.hide_watched {
            self.watched_ids(owner).await?
        } else {
            Vec::new()
        };

        let mut base = String::from(
            "(channel_id IN $all OR (channel_id IN $videos_only AND is_short = false) OR (channel_id IN $shorts_only AND is_short = true))",
        );
        base.push_str(match query.kind {
            FeedFilter::All => "",
            FeedFilter::Videos => " AND is_live = false AND is_short = false",
            FeedFilter::Shorts => " AND is_short = true AND is_live = false",
            FeedFilter::Live => " AND is_live = true",
        });
        if query.hide_watched {
            base.push_str(" AND video_id NOT IN $watched");
        }
        let filtered = match query.duration {
            None => base.clone(),
            Some(duration) => format!("{base} AND {}", duration_clause(duration)),
        };

        let t = query.thresholds;
        // A macro, not a closure: the query borrows the database handle, and a
        // closure cannot say that its result lives as long as its argument.
        macro_rules! bind {
            ($query:expr) => {
                $query
                    .bind(("all", all.clone()))
                    .bind(("videos_only", videos_only.clone()))
                    .bind(("shorts_only", shorts_only.clone()))
                    .bind(("watched", watched.clone()))
                    .bind(("v_short", t.video_short_max_seconds as i64))
                    .bind(("v_medium", t.video_medium_max_seconds as i64))
                    .bind(("s_short", t.shorts_short_max_seconds as i64))
                    .bind(("s_medium", t.shorts_medium_max_seconds as i64))
            };
        }

        let rows: Vec<DbVideo> = bind!(self.db.query(format!(
            "SELECT {VIDEO_COLUMNS} FROM video WHERE {filtered} ORDER BY published_sort DESC LIMIT $limit START $start"
        )))
        .bind(("limit", (FEED_PAGE_SIZE + 1) as i64))
        .bind(("start", (page * FEED_PAGE_SIZE) as i64))
        .await?
        .check()?
        .take(0)?;
        let has_more = rows.len() > FEED_PAGE_SIZE;
        let mut videos = rows
            .into_iter()
            .take(FEED_PAGE_SIZE)
            .map(Video::from)
            .collect::<Vec<_>>();
        self.overlay_viewer(owner, &mut videos).await?;

        // What a length filter cannot speak for yet, so the screen can say so
        // and ask for those lengths.
        let (without_duration, unknown_durations) = if query.duration.is_some() {
            let unknown = format!("{base} AND duration_seconds = 0 AND is_live = false");
            let without = if page == 0 {
                let counts: Vec<DbCount> = bind!(self.db.query(format!(
                    "SELECT count() FROM video WHERE {unknown} GROUP ALL"
                )))
                .await?
                .check()?
                .take(0)?;
                counts.first().map_or(0, |row| row.count.max(0) as usize)
            } else {
                0
            };
            let ids: Vec<DbVideo> = bind!(self.db.query(format!(
                "SELECT {VIDEO_COLUMNS} FROM video WHERE {unknown} ORDER BY published_sort DESC LIMIT $limit"
            )))
            .bind(("limit", DURATION_LOOKAHEAD as i64))
            .await?
            .check()?
            .take(0)?;
            (without, ids.into_iter().map(|row| row.video_id).collect())
        } else {
            let ids = videos
                .iter()
                .filter(|video| !video.is_live && video.duration_seconds == 0)
                .map(|video| video.id.clone())
                .collect();
            (0, ids)
        };

        Ok(FeedPage {
            videos,
            has_more,
            without_duration,
            unknown_durations,
        })
    }

    async fn watched_ids(&self, owner: &str) -> Result<Vec<String>> {
        Ok(self
            .db
            .query("SELECT VALUE video_id FROM video_progress WHERE owner = type::record($owner) AND watched = true")
            .bind(("owner", owner.to_string()))
            .await?
            .check()?
            .take(0)?)
    }

    /// Videos by id, in the order asked, with this viewer's progress. Ids the
    /// catalog does not hold are left out.
    pub async fn videos_by_id(&self, owner: &str, ids: &[String]) -> Result<Vec<Video>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let rows: Vec<DbVideo> = self
            .db
            .query(format!(
                "SELECT {VIDEO_COLUMNS} FROM video WHERE video_id IN $ids"
            ))
            .bind(("ids", ids.to_vec()))
            .await?
            .check()?
            .take(0)?;
        let mut by_id = rows
            .into_iter()
            .map(|row| (row.video_id.clone(), Video::from(row)))
            .collect::<HashMap<_, _>>();
        let mut videos = ids
            .iter()
            .filter_map(|id| by_id.remove(id))
            .collect::<Vec<_>>();
        self.overlay_viewer(owner, &mut videos).await?;
        Ok(videos)
    }

    /// A playlist and its videos, or `None` when this account has no such
    /// playlist.
    pub async fn playlist_contents(
        &self,
        owner: &str,
        playlist_id: &str,
    ) -> Result<Option<PlaylistContents>> {
        let Some(playlist) = self.find_playlist(owner, playlist_id).await? else {
            return Ok(None);
        };
        let videos = self.videos_by_id(owner, &playlist.video_ids).await?;
        Ok(Some(PlaylistContents { playlist, videos }))
    }

    /// Every playlist's cover and counts, for the playlists page. The
    /// thumbnails come from the first `thumbnails` videos the catalog holds.
    pub async fn playlist_previews(
        &self,
        owner: &str,
        thumbnails: usize,
    ) -> Result<Vec<PlaylistPreview>> {
        let mut playlists: Vec<DbPlaylist> = self
            .db
            .query("SELECT playlist_id, name, video_ids, sort_order FROM playlist WHERE owner = type::record($owner)")
            .bind(("owner", owner.to_string()))
            .await?
            .check()?
            .take(0)?;
        sort_playlists(&mut playlists);
        let watched = self
            .watched_ids(owner)
            .await?
            .into_iter()
            .collect::<HashSet<_>>();
        let wanted = playlists
            .iter()
            .flat_map(|playlist| playlist.video_ids.iter().take(thumbnails * 2).cloned())
            .collect::<Vec<_>>();
        let covers: Vec<DbVideo> = self
            .db
            .query(format!(
                "SELECT {VIDEO_COLUMNS} FROM video WHERE video_id IN $ids"
            ))
            .bind(("ids", wanted))
            .await?
            .check()?
            .take(0)?;
        Ok(playlists
            .into_iter()
            .map(|playlist| PlaylistPreview {
                count: playlist.video_ids.len(),
                unwatched: playlist
                    .video_ids
                    .iter()
                    .filter(|id| !watched.contains(*id))
                    .count(),
                thumbnails: playlist
                    .video_ids
                    .iter()
                    .filter_map(|id| covers.iter().find(|video| &video.video_id == id))
                    .take(thumbnails)
                    .map(|video| video.thumbnail_url.clone())
                    .collect(),
                id: playlist.playlist_id,
                name: playlist.name,
            })
            .collect())
    }

    async fn find_playlist(&self, owner: &str, playlist_id: &str) -> Result<Option<Playlist>> {
        let rows: Vec<DbPlaylist> = self
            .db
            .query("SELECT playlist_id, name, video_ids, sort_order FROM playlist WHERE owner = type::record($owner) AND playlist_id = $playlist_id LIMIT 1")
            .bind(("owner", owner.to_string()))
            .bind(("playlist_id", playlist_id.to_string()))
            .await?
            .check()?
            .take(0)?;
        Ok(rows.into_iter().next().map(Into::into))
    }

    /// Set each video's progress to this viewer's own, and its channel's
    /// avatar. The shared rows carry whatever the last writer left in their
    /// per-user columns, which for this viewer is somebody else's.
    pub async fn overlay_viewer(&self, owner: &str, videos: &mut [Video]) -> Result<()> {
        if videos.is_empty() {
            return Ok(());
        }
        let ids = videos
            .iter()
            .map(|video| video.id.clone())
            .collect::<Vec<_>>();
        let progress: Vec<DbVideoProgress> = if owner.is_empty() {
            Vec::new()
        } else {
            self.db
                .query("SELECT video_id, watched, progress_seconds, audio_only FROM video_progress WHERE owner = type::record($owner) AND video_id IN $ids")
                .bind(("owner", owner.to_string()))
                .bind(("ids", ids))
                .await?
                .check()?
                .take(0)?
        };
        let channel_ids = videos
            .iter()
            .map(|video| video.channel_id.clone())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let avatars: Vec<DbAvatar> = self
            .db
            .query("SELECT channel_id, avatar_url FROM channel WHERE channel_id IN $ids")
            .bind(("ids", channel_ids))
            .await?
            .check()?
            .take(0)?;
        for video in videos.iter_mut() {
            match progress.iter().find(|row| row.video_id == video.id) {
                Some(row) => {
                    video.watched = row.watched;
                    video.progress_seconds = row.progress_seconds.max(0) as u64;
                    video.audio_only = row.audio_only;
                }
                None => {
                    video.watched = false;
                    video.progress_seconds = 0;
                    video.audio_only = false;
                }
            }
            video.channel_avatar_url = avatars
                .iter()
                .find(|row| row.channel_id == video.channel_id)
                .and_then(|row| row.avatar_url.clone());
        }
        Ok(())
    }

    /// Keep the catalog's publish date for each video where it is more exact
    /// than the one about to be written over it.
    ///
    /// A video's date does not change, but the sources describing it do: the
    /// feed gives a timestamp, the watch page a day, a related shelf "11 months
    /// ago". Taking whichever arrived last moved a video within a newest-first
    /// playlist just for having been opened.
    pub(super) async fn keep_finer_dates(&self, videos: &mut [Video]) -> Result<()> {
        if videos.is_empty() {
            return Ok(());
        }
        let ids = videos
            .iter()
            .map(|video| video.id.clone())
            .collect::<Vec<_>>();
        let stored = self.videos_by_id("", &ids).await?;
        for video in videos.iter_mut() {
            if let Some(previous) = stored.iter().find(|stored| stored.id == video.id) {
                video.keep_finer_publish_date(previous);
            }
        }
        Ok(())
    }

    /// One cached channel, as the catalog holds it.
    pub(super) async fn catalog_channel(&self, channel_id: &str) -> Result<Option<Channel>> {
        let rows: Vec<DbChannel> = self
            .db
            .query(format!(
                "SELECT {CHANNEL_COLUMNS} FROM channel WHERE channel_id = $channel_id LIMIT 1"
            ))
            .bind(("channel_id", channel_id.to_string()))
            .await?
            .check()?
            .take(0)?;
        Ok(rows.into_iter().next().map(Into::into))
    }

    /// A channel's cached uploads, newest first.
    pub(super) async fn catalog_channel_videos(
        &self,
        channel_id: &str,
        limit: usize,
    ) -> Result<Vec<Video>> {
        let rows: Vec<DbVideo> = self
            .db
            .query(format!(
                "SELECT {VIDEO_COLUMNS} FROM video WHERE channel_id = $channel_id ORDER BY published_sort DESC LIMIT $limit"
            ))
            .bind(("channel_id", channel_id.to_string()))
            .bind(("limit", limit as i64))
            .await?
            .check()?
            .take(0)?;
        Ok(rows.into_iter().map(Into::into).collect())
    }
}

/// The SQL for one length bucket. Videos and Shorts have separate edges; a
/// live stream or an unknown length belongs to no bucket.
fn duration_clause(duration: DurationFilter) -> &'static str {
    match duration {
        DurationFilter::Short => {
            "(duration_seconds > 0 AND is_live = false AND ((is_short = true AND duration_seconds <= $s_short) OR (is_short = false AND duration_seconds <= $v_short)))"
        }
        DurationFilter::Medium => {
            "(duration_seconds > 0 AND is_live = false AND ((is_short = true AND duration_seconds > $s_short AND duration_seconds <= $s_medium) OR (is_short = false AND duration_seconds > $v_short AND duration_seconds <= $v_medium)))"
        }
        DurationFilter::Long => {
            "(is_live = false AND ((is_short = true AND duration_seconds > $s_medium) OR (is_short = false AND duration_seconds > $v_medium)))"
        }
    }
}

impl AppServerState {
    // -----------------------------------------------------------------------
    // Writes
    // -----------------------------------------------------------------------

    /// Set this account's answer for each channel. A channel the catalog has
    /// not met yet (an import, a search result) is added to it first.
    pub async fn set_subscriptions(
        &self,
        owner: &str,
        changes: Vec<SubscriptionChange>,
    ) -> Result<()> {
        let prior = self.user_subscriptions(owner).await?;
        let mut changed = Vec::new();
        for change in &changes {
            let channel_id = &change.channel.id;
            if channel_id.is_empty() {
                continue;
            }
            if self.catalog_channel(channel_id).await?.is_none() {
                // `subscribed` on the catalog row is derived: see
                // `refresh_channel_interest`, which sets it below.
                self.upsert_channel(&Channel {
                    subscribed: false,
                    ..change.channel.clone()
                })
                .await?;
            }
            let was = prior.get(channel_id).map(|(subscribed, _)| *subscribed);
            if was.unwrap_or(false) != change.subscribed {
                changed.push(channel_id.clone());
            }
            self.db
                .query(
                    r#"UPSERT type::record('user_subscription', $record_key) SET
                        owner = type::record($owner),
                        channel_id = $channel_id,
                        subscribed = $subscribed,
                        content = $content,
                        updated_at = time::now()"#,
                )
                .bind(("record_key", Self::owner_key(owner, channel_id)))
                .bind(("owner", owner.to_string()))
                .bind(("channel_id", channel_id.clone()))
                .bind(("subscribed", change.subscribed))
                .bind(("content", change.content.as_storage().to_string()))
                .await?
                .check()?;
        }
        self.refresh_channel_interest(&changed).await?;

        // The feed reloads as soon as this returns. Without the new channel's
        // latest uploads it showed whatever the catalog held for it, such as
        // the Popular tab just browsed, until the background fetch below, and
        // nothing reloaded it then. RSS is one quick request with exact
        // times, so a few new follows wait for it; a bulk import does not.
        let newly_subscribed = changes
            .iter()
            .filter(|change| change.subscribed && changed.contains(&change.channel.id))
            .filter(|change| {
                change.channel.id.starts_with("UC") && !change.channel.id.starts_with("UC-tawny")
            })
            .map(|change| change.channel.clone())
            .collect::<Vec<_>>();
        if newly_subscribed.len() <= FIRST_FETCH_WAIT_MAX {
            use futures_util::{StreamExt, stream};
            let _ = stream::iter(
                newly_subscribed
                    .into_iter()
                    .map(|channel| self.refresh_channel_rss_fast(channel)),
            )
            .buffer_unordered(FIRST_FETCH_WAIT_MAX)
            .collect::<Vec<_>>()
            .await;
        }

        // Subscribing starts WebSub and a first fetch; unsubscribing the last
        // follower ends them. That side needs the instance-wide answer.
        let changed_channels = changes
            .into_iter()
            .filter(|change| changed.contains(&change.channel.id))
            .map(|change| change.channel)
            .filter(|channel| channel.id.starts_with("UC") && !channel.id.starts_with("UC-tawny"))
            .collect::<Vec<_>>();
        self.spawn_subscription_changes(changed_channels);
        Ok(())
    }

    /// Create a playlist, or rename one.
    pub async fn save_playlist(
        &self,
        owner: &str,
        playlist_id: &str,
        name: &str,
    ) -> Result<Playlist> {
        let _guard = self.sync_lock.lock().await;
        let existing = self.find_playlist(owner, playlist_id).await?;
        let created = existing.is_none();
        let mut playlist = existing.unwrap_or_else(|| Playlist {
            id: playlist_id.to_string(),
            name: String::new(),
            video_ids: Vec::new(),
        });
        playlist.name = name.to_string();
        if created {
            // A new playlist goes last. The rest are numbered as they show
            // now first, since one never arranged has no position, and a
            // newcomer numbered after the arranged ones would land above it.
            let mut order = self.playlist_order(owner).await?;
            self.upsert_playlist(owner, &playlist).await?;
            order.push(playlist.id.clone());
            self.write_playlist_order(owner, &order).await?;
        } else {
            self.upsert_playlist(owner, &playlist).await?;
        }
        Ok(playlist)
    }

    /// The owner's playlist ids, in the order they are shown.
    async fn playlist_order(&self, owner: &str) -> Result<Vec<String>> {
        let mut playlists: Vec<DbPlaylist> = self
            .db
            .query("SELECT playlist_id, name, video_ids, sort_order FROM playlist WHERE owner = type::record($owner)")
            .bind(("owner", owner.to_string()))
            .await?
            .check()?
            .take(0)?;
        sort_playlists(&mut playlists);
        Ok(playlists
            .into_iter()
            .map(|playlist| playlist.playlist_id)
            .collect())
    }

    /// Put a video in a playlist (at the top) or take it out. Returns whether
    /// that changed anything.
    pub async fn set_in_playlist(
        &self,
        owner: &str,
        playlist_id: &str,
        video_id: &str,
        member: bool,
    ) -> Result<bool> {
        let _guard = self.sync_lock.lock().await;
        let mut playlist = self
            .find_playlist(owner, playlist_id)
            .await?
            .ok_or_else(|| anyhow!("that playlist no longer exists"))?;
        let present = playlist.video_ids.iter().any(|id| id == video_id);
        if present == member {
            return Ok(false);
        }
        if member {
            playlist.video_ids.insert(0, video_id.to_string());
        } else {
            playlist.video_ids.retain(|id| id != video_id);
        }
        self.upsert_playlist(owner, &playlist).await?;
        Ok(true)
    }

    /// Take every video this viewer has watched out of a playlist. Returns
    /// how many were removed.
    pub async fn remove_watched_from_playlist(
        &self,
        owner: &str,
        playlist_id: &str,
    ) -> Result<usize> {
        let _guard = self.sync_lock.lock().await;
        let Some(mut playlist) = self.find_playlist(owner, playlist_id).await? else {
            return Ok(0);
        };
        let watched = self
            .watched_ids(owner)
            .await?
            .into_iter()
            .collect::<HashSet<_>>();
        let before = playlist.video_ids.len();
        playlist.video_ids.retain(|id| !watched.contains(id));
        let removed = before - playlist.video_ids.len();
        if removed > 0 {
            self.upsert_playlist(owner, &playlist).await?;
        }
        Ok(removed)
    }

    /// Put the owner's playlists in the order given. A playlist the list does
    /// not name (made on another device since it was read) keeps no position
    /// and falls in after the ones that have one, rather than being lost.
    pub async fn reorder_playlists(&self, owner: &str, playlist_ids: &[String]) -> Result<()> {
        let _guard = self.sync_lock.lock().await;
        self.write_playlist_order(owner, playlist_ids).await
    }

    /// [`Self::reorder_playlists`], for a caller already holding the lock.
    async fn write_playlist_order(&self, owner: &str, playlist_ids: &[String]) -> Result<()> {
        for (position, playlist_id) in playlist_ids.iter().enumerate() {
            self.db
                .query(
                    "UPDATE playlist SET sort_order = $position \
                     WHERE owner = type::record($owner) AND playlist_id = $playlist_id",
                )
                .bind(("position", position as i64))
                .bind(("owner", owner.to_string()))
                .bind(("playlist_id", playlist_id.clone()))
                .await?
                .check()?;
        }
        Ok(())
    }

    /// Delete a playlist, and any queued run of it.
    pub async fn delete_playlist(&self, owner: &str, playlist_id: &str) -> Result<()> {
        let _guard = self.sync_lock.lock().await;
        self.db
            .query(
                "DELETE playlist WHERE owner = type::record($owner) AND playlist_id = $playlist_id",
            )
            .bind(("owner", owner.to_string()))
            .bind(("playlist_id", playlist_id.to_string()))
            .await?
            .check()?;
        let (mut queue, _) = self.queue_and_history(owner).await?;
        let before = queue.len();
        queue.retain(|entry| crate::models::queued_playlist_id(entry) != Some(playlist_id));
        if queue.len() != before {
            self.write_queue(owner, &queue).await?;
        }
        Ok(())
    }

    /// Set watch progress (and the audio-only choice) for the videos given.
    /// Videos not mentioned keep theirs.
    pub async fn save_progress(&self, owner: &str, progress: &[VideoProgress]) -> Result<()> {
        self.write_user_progress(owner, progress).await
    }

    /// This account's saved place in one video, if it has one.
    pub async fn video_progress(
        &self,
        owner: &str,
        video_id: &str,
    ) -> Result<Option<VideoProgress>> {
        if owner.is_empty() {
            return Ok(None);
        }
        let rows: Vec<DbVideoProgress> = self
            .db
            .query("SELECT video_id, watched, progress_seconds, audio_only FROM video_progress WHERE owner = type::record($owner) AND video_id = $video_id LIMIT 1")
            .bind(("owner", owner.to_string()))
            .bind(("video_id", video_id.to_string()))
            .await?
            .check()?
            .take(0)?;
        Ok(rows.into_iter().next().map(|row| VideoProgress {
            video_id: row.video_id,
            watched: row.watched,
            progress_seconds: row.progress_seconds.max(0) as u64,
            audio_only: row.audio_only,
        }))
    }

    /// Create or replace subscription groups, by id.
    pub async fn save_groups(&self, owner: &str, groups: &[SubscriptionGroup]) -> Result<()> {
        let _guard = self.sync_lock.lock().await;
        for group in groups {
            self.upsert_subscription_group(owner, group).await?;
        }
        Ok(())
    }

    /// Put a channel in a group or take it out. Returns whether that changed
    /// anything.
    pub async fn set_in_group(
        &self,
        owner: &str,
        group_id: &str,
        channel_id: &str,
        member: bool,
    ) -> Result<bool> {
        let _guard = self.sync_lock.lock().await;
        let rows: Vec<DbSubscriptionGroup> = self
            .db
            .query("SELECT group_id, name, channel_ids FROM subscription_group WHERE owner = type::record($owner) AND group_id = $group_id LIMIT 1")
            .bind(("owner", owner.to_string()))
            .bind(("group_id", group_id.to_string()))
            .await?
            .check()?
            .take(0)?;
        let mut group = SubscriptionGroup::from(
            rows.into_iter()
                .next()
                .ok_or_else(|| anyhow!("that group no longer exists"))?,
        );
        let present = group.channel_ids.iter().any(|id| id == channel_id);
        if present == member {
            return Ok(false);
        }
        if member {
            group.channel_ids.push(channel_id.to_string());
        } else {
            group.channel_ids.retain(|id| id != channel_id);
        }
        self.upsert_subscription_group(owner, &group).await?;
        Ok(true)
    }

    pub async fn delete_group(&self, owner: &str, group_id: &str) -> Result<()> {
        self.db
            .query(
                "DELETE subscription_group WHERE owner = type::record($owner) AND group_id = $group_id",
            )
            .bind(("owner", owner.to_string()))
            .bind(("group_id", group_id.to_string()))
            .await?
            .check()?;
        Ok(())
    }

    /// Replace the queue. It is one ordered list the viewer arranges as a
    /// whole, so the whole list is the answer to set.
    pub async fn set_queue(&self, owner: &str, queue: &[String]) -> Result<()> {
        let _guard = self.sync_lock.lock().await;
        self.write_queue(owner, queue).await
    }

    /// Put a video at the top of history.
    pub async fn record_history(&self, owner: &str, video_id: &str) -> Result<()> {
        let _guard = self.sync_lock.lock().await;
        let (_, mut history) = self.queue_and_history(owner).await?;
        history.retain(|entry| entry.video_id != video_id);
        history.insert(
            0,
            HistoryEntry {
                video_id: video_id.to_string(),
                played_at: time::OffsetDateTime::now_utc()
                    .format(&time::format_description::well_known::Rfc3339)
                    .unwrap_or_default(),
            },
        );
        history.truncate(HISTORY_LIMIT);
        self.write_history(owner, &history).await
    }

    pub async fn clear_history(&self, owner: &str) -> Result<()> {
        let _guard = self.sync_lock.lock().await;
        self.write_history(owner, &[]).await
    }

    async fn write_queue(&self, owner: &str, queue: &[String]) -> Result<()> {
        self.write_state_field(owner, "queue_json", serde_json::to_string(queue)?)
            .await
    }

    async fn write_history(&self, owner: &str, history: &[HistoryEntry]) -> Result<()> {
        self.write_state_field(owner, "history_json", serde_json::to_string(history)?)
            .await
    }

    /// Write one column of this account's `library_state` row, creating the
    /// row when it has none. Callers hold `sync_lock`, and the unique index on
    /// the owner keeps a race from making two.
    async fn write_state_field(
        &self,
        owner: &str,
        field: &'static str,
        json: String,
    ) -> Result<()> {
        let updated: Vec<DbCount> = self
            .db
            .query(format!(
                "UPDATE library_state SET {field} = $json, updated_at = time::now() WHERE owner = type::record($owner) RETURN 1 AS count"
            ))
            .bind(("owner", owner.to_string()))
            .bind(("json", json.clone()))
            .await?
            .check()?
            .take(0)?;
        if updated.is_empty() {
            self.db
                .query(format!(
                    "CREATE library_state SET owner = type::record($owner), {field} = $json, updated_at = time::now()"
                ))
                .bind(("owner", owner.to_string()))
                .bind(("json", json))
                .await?
                .check()?;
        }
        Ok(())
    }

    /// What a new account starts with: Watch Later and Favorites, both
    /// empty. Its queue is empty and it follows nothing, so its feed starts
    /// empty too.
    pub async fn seed_new_account(&self, owner: &str) -> Result<()> {
        // Created one after another, so each takes the next place.
        for playlist in crate::models::demo_viewer().playlists {
            self.save_playlist(owner, &playlist.id, &playlist.name)
                .await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{DurationThresholds, playlist_queue_entry};

    async fn state_and_owner() -> (AppServerState, String) {
        let state = AppServerState::initialize().await.unwrap();
        let owner = state.accounts().create_guest().await.unwrap().id;
        (state, owner)
    }

    fn channel(id: &str) -> Channel {
        Channel {
            id: id.into(),
            name: format!("Channel {id}"),
            handle: format!("@{id}"),
            avatar_url: Some(format!("https://avatars/{id}")),
            subscriber_count: String::new(),
            subscribed: false,
            description: String::new(),
            banner_url: None,
            subscription_content: SubscriptionContent::All,
        }
    }

    fn video(id: &str, channel_id: &str, day: u32, seconds: u64, short: bool) -> Video {
        Video {
            id: id.into(),
            title: format!("Video {id}"),
            channel_id: channel_id.into(),
            channel_name: format!("Channel {channel_id}"),
            thumbnail_url: String::new(),
            published_at: format!("2026-01-{day:02}T00:00:00Z"),
            duration_seconds: seconds,
            view_count: String::new(),
            progress_seconds: 0,
            watched: false,
            is_live: false,
            is_short: short,
            audio_only: false,
            channel_avatar_url: None,
        }
    }

    async fn follow(
        state: &AppServerState,
        owner: &str,
        channel_id: &str,
        content: SubscriptionContent,
    ) {
        state
            .set_subscriptions(
                owner,
                vec![SubscriptionChange {
                    channel: channel(channel_id),
                    subscribed: true,
                    content,
                }],
            )
            .await
            .unwrap();
    }

    async fn store(state: &AppServerState, videos: &[Video]) {
        for video in videos {
            state
                .upsert_video(video, video.published_at.clone())
                .await
                .unwrap();
        }
    }

    fn query() -> FeedQuery {
        FeedQuery {
            group: FeedGroup::All,
            kind: FeedFilter::All,
            duration: None,
            hide_watched: false,
            thresholds: DurationThresholds {
                video_short_max_seconds: 4 * 60,
                video_medium_max_seconds: 20 * 60,
                shorts_short_max_seconds: 20,
                shorts_medium_max_seconds: 40,
            },
        }
    }

    fn ids(videos: &[Video]) -> Vec<&str> {
        videos.iter().map(|video| video.id.as_str()).collect()
    }

    #[tokio::test]
    async fn the_feed_holds_followed_uploads_newest_first() {
        let (state, owner) = state_and_owner().await;
        follow(&state, &owner, "UCa", SubscriptionContent::All).await;
        follow(&state, &owner, "UCb", SubscriptionContent::Videos).await;
        store(
            &state,
            &[
                video("a-long", "UCa", 1, 600, false),
                video("a-short", "UCa", 3, 30, true),
                video("b-long", "UCb", 2, 900, false),
                video("b-short", "UCb", 4, 30, true),
                video("c-long", "UCc", 5, 600, false),
            ],
        )
        .await;

        let page = state.feed_page(&owner, &query(), 0).await.unwrap();

        // b-short: that subscription wants videos only. c-long: not followed.
        assert_eq!(ids(&page.videos), ["a-short", "b-long", "a-long"]);
        assert!(!page.has_more);
        assert_eq!(
            page.videos[0].channel_avatar_url.as_deref(),
            Some("https://avatars/UCa"),
            "a card should not have to look its avatar up"
        );
    }

    #[tokio::test]
    async fn one_account_sees_nothing_of_another() {
        let (state, first) = state_and_owner().await;
        let second = state.accounts().create_guest().await.unwrap().id;
        follow(&state, &first, "UCa", SubscriptionContent::All).await;
        store(&state, &[video("a1", "UCa", 1, 600, false)]).await;
        state
            .save_progress(
                &first,
                &[VideoProgress {
                    video_id: "a1".into(),
                    watched: true,
                    progress_seconds: 600,
                    audio_only: false,
                }],
            )
            .await
            .unwrap();
        state.record_history(&first, "a1").await.unwrap();

        let theirs = state.viewer(&second).await.unwrap();
        assert!(theirs.subscriptions.is_empty());
        assert!(theirs.history.is_empty());
        assert!(
            state
                .feed_page(&second, &query(), 0)
                .await
                .unwrap()
                .videos
                .is_empty()
        );
        let seen_by_second = state.videos_by_id(&second, &["a1".into()]).await.unwrap();
        assert!(
            !seen_by_second[0].watched,
            "progress belongs to the first account"
        );

        let mine = state.videos_by_id(&first, &["a1".into()]).await.unwrap();
        assert!(mine[0].watched);
    }

    #[tokio::test]
    async fn hide_watched_and_the_length_filters_are_applied_by_the_server() {
        let (state, owner) = state_and_owner().await;
        follow(&state, &owner, "UCa", SubscriptionContent::All).await;
        store(
            &state,
            &[
                video("short", "UCa", 1, 120, false),
                video("medium", "UCa", 2, 600, false),
                video("long", "UCa", 3, 3600, false),
                video("unknown", "UCa", 4, 0, false),
            ],
        )
        .await;
        state
            .save_progress(
                &owner,
                &[VideoProgress {
                    video_id: "medium".into(),
                    watched: true,
                    progress_seconds: 600,
                    audio_only: false,
                }],
            )
            .await
            .unwrap();

        let mut unwatched = query();
        unwatched.hide_watched = true;
        let page = state.feed_page(&owner, &unwatched, 0).await.unwrap();
        assert_eq!(ids(&page.videos), ["unknown", "long", "short"]);

        let mut long = query();
        long.duration = Some(DurationFilter::Long);
        let page = state.feed_page(&owner, &long, 0).await.unwrap();
        assert_eq!(ids(&page.videos), ["long"]);
        assert_eq!(page.without_duration, 1);
        assert_eq!(page.unknown_durations, ["unknown"]);
    }

    #[tokio::test]
    async fn the_feed_pages() {
        let (state, owner) = state_and_owner().await;
        follow(&state, &owner, "UCa", SubscriptionContent::All).await;
        let videos = (1..=30)
            .map(|day| video(&format!("v{day:02}"), "UCa", day, 600, false))
            .collect::<Vec<_>>();
        store(&state, &videos).await;

        let first = state.feed_page(&owner, &query(), 0).await.unwrap();
        let second = state.feed_page(&owner, &query(), 1).await.unwrap();
        assert_eq!(first.videos.len(), FEED_PAGE_SIZE);
        assert!(first.has_more);
        assert_eq!(first.videos[0].id, "v30");
        assert_eq!(second.videos.len(), 30 - FEED_PAGE_SIZE);
        assert!(!second.has_more);
        assert_eq!(second.videos.last().unwrap().id, "v01");
    }

    #[tokio::test]
    async fn playlist_membership_is_set_not_toggled() {
        let (state, owner) = state_and_owner().await;
        store(&state, &[video("a1", "UCa", 1, 600, false)]).await;
        state.save_playlist(&owner, "later", "Later").await.unwrap();

        assert!(
            state
                .set_in_playlist(&owner, "later", "a1", true)
                .await
                .unwrap()
        );
        // A second device that saw the old state sends the same answer.
        assert!(
            !state
                .set_in_playlist(&owner, "later", "a1", true)
                .await
                .unwrap()
        );
        let contents = state
            .playlist_contents(&owner, "later")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(contents.playlist.video_ids, ["a1"]);
        assert_eq!(ids(&contents.videos), ["a1"]);

        assert!(
            state
                .set_in_playlist(&owner, "later", "a1", false)
                .await
                .unwrap()
        );
        assert!(
            !state
                .set_in_playlist(&owner, "later", "a1", false)
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn playlists_keep_the_order_they_were_put_in() {
        let (state, owner) = state_and_owner().await;
        for (id, name) in [("b", "Bravo"), ("a", "Alpha"), ("c", "Charlie")] {
            state.save_playlist(&owner, id, name).await.unwrap();
        }
        let names = |viewer: Viewer| {
            viewer
                .playlists
                .into_iter()
                .filter(|playlist| ["a", "b", "c", "d"].contains(&playlist.id.as_str()))
                .map(|playlist| playlist.id)
                .collect::<Vec<_>>()
        };
        // Never arranged: in the order they were made, not by name.
        assert_eq!(names(state.viewer(&owner).await.unwrap()), ["b", "a", "c"]);

        state
            .reorder_playlists(&owner, &["c".into(), "a".into(), "b".into()])
            .await
            .unwrap();
        assert_eq!(names(state.viewer(&owner).await.unwrap()), ["c", "a", "b"]);
        let previews = state.playlist_previews(&owner, 3).await.unwrap();
        assert_eq!(
            previews
                .iter()
                .filter(|preview| ["a", "b", "c"].contains(&preview.id.as_str()))
                .map(|preview| preview.id.as_str())
                .collect::<Vec<_>>(),
            ["c", "a", "b"]
        );

        // Renaming keeps the place, and a playlist made later goes last.
        state.save_playlist(&owner, "a", "Zulu").await.unwrap();
        state.save_playlist(&owner, "d", "Delta").await.unwrap();
        assert_eq!(
            names(state.viewer(&owner).await.unwrap()),
            ["c", "a", "b", "d"]
        );
    }

    /// Playlists made before they had a position list by name. A new one
    /// still goes after them, rather than above them for being the only one
    /// with a number.
    #[tokio::test]
    async fn a_new_playlist_goes_after_ones_that_never_had_a_place() {
        let (state, owner) = state_and_owner().await;
        for (id, name) in [("y", "Yankee"), ("x", "Xray")] {
            let playlist = Playlist {
                id: id.into(),
                name: name.into(),
                video_ids: Vec::new(),
            };
            state.upsert_playlist(&owner, &playlist).await.unwrap();
        }
        state.save_playlist(&owner, "n", "New").await.unwrap();
        let ids = state
            .viewer(&owner)
            .await
            .unwrap()
            .playlists
            .into_iter()
            .map(|playlist| playlist.id)
            .collect::<Vec<_>>();
        assert_eq!(ids, ["x", "y", "n"]);
    }

    #[tokio::test]
    async fn deleting_a_playlist_drops_its_queued_run() {
        let (state, owner) = state_and_owner().await;
        state.save_playlist(&owner, "later", "Later").await.unwrap();
        state
            .set_queue(&owner, &["a1".into(), playlist_queue_entry("later")])
            .await
            .unwrap();

        state.delete_playlist(&owner, "later").await.unwrap();

        let viewer = state.viewer(&owner).await.unwrap();
        assert!(viewer.playlist("later").is_none());
        assert_eq!(viewer.queue, ["a1"]);
    }

    #[tokio::test]
    async fn history_is_newest_first_without_repeats() {
        let (state, owner) = state_and_owner().await;
        for video_id in ["a", "b", "a"] {
            state.record_history(&owner, video_id).await.unwrap();
        }
        let history = state.viewer(&owner).await.unwrap().history;
        let order = history
            .iter()
            .map(|entry| entry.video_id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(order, ["a", "b"]);

        state.clear_history(&owner).await.unwrap();
        assert!(state.viewer(&owner).await.unwrap().history.is_empty());
    }

    #[tokio::test]
    async fn a_group_narrows_the_feed() {
        let (state, owner) = state_and_owner().await;
        follow(&state, &owner, "UCa", SubscriptionContent::All).await;
        follow(&state, &owner, "UCb", SubscriptionContent::All).await;
        store(
            &state,
            &[
                video("a1", "UCa", 1, 600, false),
                video("b1", "UCb", 2, 600, false),
            ],
        )
        .await;
        state
            .save_groups(
                &owner,
                &[SubscriptionGroup {
                    id: "g".into(),
                    name: "G".into(),
                    channel_ids: Vec::new(),
                }],
            )
            .await
            .unwrap();
        assert!(state.set_in_group(&owner, "g", "UCa", true).await.unwrap());

        let mut grouped = query();
        grouped.group = FeedGroup::Group("g".into());
        assert_eq!(
            ids(&state.feed_page(&owner, &grouped, 0).await.unwrap().videos),
            ["a1"]
        );
        grouped.group = FeedGroup::Ungrouped;
        assert_eq!(
            ids(&state.feed_page(&owner, &grouped, 0).await.unwrap().videos),
            ["b1"]
        );
    }

    #[tokio::test]
    async fn a_new_account_starts_with_two_empty_playlists_and_nothing_else() {
        let (state, owner) = state_and_owner().await;
        state.seed_new_account(&owner).await.unwrap();
        let viewer = state.viewer(&owner).await.unwrap();
        let playlists = viewer
            .playlists
            .iter()
            .map(|playlist| (playlist.name.as_str(), playlist.video_ids.len()))
            .collect::<Vec<_>>();
        assert_eq!(playlists, [("Watch Later", 0), ("Favorites", 0)]);
        assert!(viewer.queue.is_empty());
        assert!(viewer.subscriptions.is_empty());
        assert!(viewer.subscription_groups.is_empty());
        let feed = state.feed_page(&owner, &query(), 0).await.unwrap();
        assert!(feed.videos.is_empty());
    }
}
