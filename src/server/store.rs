use super::*;

impl AppServerState {
    pub(crate) async fn seed_demo_if_empty(&self) -> Result<()> {
        let existing: Vec<DbChannel> = self
            .db
            .query(
                "SELECT channel_id, name, handle, avatar_url, subscriber_count, subscribed, subscription_content, description, banner_url FROM channel LIMIT 1",
            )
            .await?
            .take(0)?;
        if !existing.is_empty() {
            return Ok(());
        }
        let demo = crate::models::DemoLibrary::demo();
        for channel in &demo.channels {
            self.upsert_channel(channel).await?;
        }
        for (index, video) in demo.videos.iter().enumerate() {
            self.upsert_video(video, format!("demo-{index:04}")).await?;
        }
        Ok(())
    }

    pub(crate) async fn upsert_channel(&self, channel: &Channel) -> Result<()> {
        self.db
            .query(
                r#"UPSERT type::record('channel', $record_key) SET
                    channel_id = $channel_id,
                    name = $name,
                    handle = $handle,
                    avatar_url = $avatar_url,
                    subscriber_count = $subscriber_count,
                    subscribed = $subscribed,
                    subscription_content = $subscription_content,
                    description = $description,
                    banner_url = $banner_url"#,
            )
            .bind(("record_key", channel.id.clone()))
            .bind(("channel_id", channel.id.clone()))
            .bind(("name", channel.name.clone()))
            .bind(("handle", channel.handle.clone()))
            .bind(("avatar_url", channel.avatar_url.clone()))
            .bind(("subscriber_count", channel.subscriber_count.clone()))
            .bind(("subscribed", channel.subscribed))
            .bind((
                "subscription_content",
                channel.subscription_content.as_storage(),
            ))
            .bind(("description", channel.description.clone()))
            .bind(("banner_url", channel.banner_url.clone()))
            .await?
            .check()?;
        Ok(())
    }

    pub(crate) async fn upsert_video(&self, video: &Video, published_sort: String) -> Result<()> {
        let published_sort = canonical_sort_key(&published_sort);
        self.db
            .query(
                r#"UPSERT type::record('video', $record_key) SET
                    video_id = $video_id,
                    title = $title,
                    channel_id = $channel_id,
                    channel_name = $channel_name,
                    thumbnail_url = $thumbnail_url,
                    published_at = $published_at,
                    published_sort = $published_sort,
                    duration_seconds = $duration_seconds,
                    view_count = $view_count,
                    is_live = $is_live,
                    is_short = $is_short,
                    progress_seconds = $progress_seconds,
                    watched = $watched,
                    audio_only = $audio_only"#,
            )
            .bind(("record_key", video.id.clone()))
            .bind(("video_id", video.id.clone()))
            .bind(("title", video.title.clone()))
            .bind(("channel_id", video.channel_id.clone()))
            .bind(("channel_name", video.channel_name.clone()))
            .bind(("thumbnail_url", video.thumbnail_url.clone()))
            .bind(("published_at", video.published_at.clone()))
            .bind(("published_sort", published_sort))
            .bind(("duration_seconds", video.duration_seconds as i64))
            .bind(("view_count", video.view_count.clone()))
            .bind(("is_live", video.is_live))
            .bind(("is_short", video.is_short))
            .bind(("progress_seconds", video.progress_seconds as i64))
            .bind(("watched", video.watched))
            .bind(("audio_only", video.audio_only))
            .await?
            .check()?;
        Ok(())
    }

    /// `video` as it should be stored: with the stored publish date kept when
    /// that is more exact, and the sort key belonging to whichever date wins.
    ///
    /// The same upload reaches the feed from several sources: RSS with a
    /// timestamp, the watch page with a day, a channel tab or shelf with "2
    /// years ago". Taking whichever arrived last let browsing a channel's
    /// Oldest or Popular tab overwrite exact feed dates with guesses, which
    /// mislabelled and reordered the feed until the next refresh.
    async fn with_finer_publish_date(
        &self,
        video: &Video,
        published_sort: String,
    ) -> Result<(Video, String)> {
        let stored: Option<String> = self
            .db
            .query("SELECT VALUE published_at FROM type::record('video', $record_key)")
            .bind(("record_key", video.id.clone()))
            .await?
            .take(0)?;
        let mut kept = video.clone();
        if let Some(stored) = stored.filter(|stored| !stored.trim().is_empty()) {
            kept.keep_finer_publish_date(&Video {
                published_at: stored,
                ..video.clone()
            });
        }
        let sort = if kept.published_at == video.published_at {
            published_sort
        } else {
            video_sort_key(&kept)
        };
        Ok((kept, sort))
    }

    pub(crate) async fn upsert_feed_video(
        &self,
        video: &Video,
        published_sort: String,
    ) -> Result<()> {
        let (video, published_sort) = self.with_finer_publish_date(video, published_sort).await?;
        let video = &video;
        let published_sort = canonical_sort_key(&published_sort);
        self.db
            .query(
                r#"UPSERT type::record('video', $record_key) SET
                    video_id = $video_id,
                    title = $title,
                    channel_id = $channel_id,
                    channel_name = $channel_name,
                    thumbnail_url = $thumbnail_url,
                    published_at = $published_at,
                    published_sort = $published_sort,
                    duration_seconds = $duration_seconds,
                    view_count = $view_count,
                    is_live = $is_live,
                    is_short = $is_short"#,
            )
            .bind(("record_key", video.id.clone()))
            .bind(("video_id", video.id.clone()))
            .bind(("title", video.title.clone()))
            .bind(("channel_id", video.channel_id.clone()))
            .bind(("channel_name", video.channel_name.clone()))
            .bind(("thumbnail_url", video.thumbnail_url.clone()))
            .bind(("published_at", video.published_at.clone()))
            .bind(("published_sort", published_sort))
            .bind(("duration_seconds", video.duration_seconds as i64))
            .bind(("view_count", video.view_count.clone()))
            .bind(("is_live", video.is_live))
            .bind(("is_short", video.is_short))
            .await?
            .check()?;
        Ok(())
    }

    pub(crate) async fn upsert_feed_hint(
        &self,
        video: &Video,
        published_sort: String,
    ) -> Result<()> {
        let (video, published_sort) = self.with_finer_publish_date(video, published_sort).await?;
        let video = &video;
        let published_sort = canonical_sort_key(&published_sort);
        self.db
            .query(
                r#"UPSERT type::record('video', $record_key) SET
                    video_id = $video_id,
                    title = $title,
                    channel_id = $channel_id,
                    channel_name = $channel_name,
                    thumbnail_url = $thumbnail_url,
                    published_at = $published_at,
                    published_sort = $published_sort"#,
            )
            .bind(("record_key", video.id.clone()))
            .bind(("video_id", video.id.clone()))
            .bind(("title", video.title.clone()))
            .bind(("channel_id", video.channel_id.clone()))
            .bind(("channel_name", video.channel_name.clone()))
            .bind(("thumbnail_url", video.thumbnail_url.clone()))
            .bind(("published_at", video.published_at.clone()))
            .bind(("published_sort", published_sort))
            .await?
            .check()?;
        Ok(())
    }

    pub(crate) async fn upsert_discovered_channel(&self, channel: &Channel) -> Result<()> {
        let existing: Vec<DbChannel> = self
            .db
            .query("SELECT * FROM channel WHERE channel_id = $channel_id LIMIT 1")
            .bind(("channel_id", channel.id.clone()))
            .await?
            .take(0)?;
        let mut merged = existing
            .into_iter()
            .next()
            .map(Channel::from)
            .unwrap_or_else(|| channel.clone());
        if merged.id == channel.id {
            merged.merge_metadata_from(channel);
        }
        self.db
            .query(
                r#"UPSERT type::record('channel', $record_key) SET
                    channel_id = $channel_id,
                    name = $name,
                    handle = $handle,
                    avatar_url = $avatar_url,
                    subscriber_count = $subscriber_count,
                    description = $description,
                    banner_url = $banner_url,
                    subscribed = $subscribed,
                    subscription_content = $subscription_content"#,
            )
            .bind(("record_key", channel.id.clone()))
            .bind(("channel_id", channel.id.clone()))
            .bind(("name", merged.name))
            .bind(("handle", merged.handle))
            .bind(("avatar_url", merged.avatar_url))
            .bind(("subscriber_count", merged.subscriber_count))
            .bind(("description", merged.description))
            .bind(("banner_url", merged.banner_url))
            .bind(("subscribed", merged.subscribed))
            .bind((
                "subscription_content",
                merged.subscription_content.as_storage(),
            ))
            .await?
            .check()?;
        Ok(())
    }

    /// Fill in the avatar and subscriber count of followed channels missing
    /// either, and report how many rows gained something.
    ///
    /// An imported subscription arrives as a stub holding only an id and a name.
    /// Following a few channels reads each channel page, which is where those
    /// facts come from, but a bulk import is reconciled over RSS - which has
    /// neither - so imported channels stayed faceless for good. `only` narrows a
    /// pass to channels that just changed; `limit` caps the channel pages one
    /// pass requests.
    pub(crate) async fn backfill_channel_metadata(
        &self,
        only: Option<&[String]>,
        limit: usize,
    ) -> usize {
        let rows: Vec<DbChannel> = match self
            .db
            .query("SELECT channel_id, name, handle, avatar_url, subscriber_count, subscribed, subscription_content, description, banner_url FROM channel WHERE subscribed = true AND (subscriber_count_verified_at = NONE OR avatar_url = NONE)")
            .await
            .and_then(|mut response| response.take(0))
        {
            Ok(rows) => rows,
            Err(error) => {
                eprintln!("could not list channels for a metadata backfill: {error}");
                return 0;
            }
        };
        let candidates = rows
            .into_iter()
            .map(Channel::from)
            .filter(|channel| channel.id.starts_with("UC") && !channel.id.starts_with("UC-tawny"))
            .filter(|channel| only.is_none_or(|ids| ids.contains(&channel.id)))
            .take(limit)
            .collect::<Vec<_>>();
        if candidates.is_empty() {
            return 0;
        }

        use futures_util::{StreamExt, stream};
        stream::iter(candidates.into_iter().map(|channel| async move {
            let query = self.youtube.query();
            let page = match youtube_call(
                query.channel_videos(&channel.id),
                "extract channel metadata",
            )
            .await
            {
                Ok(page) => page,
                Err(error) => {
                    eprintln!("metadata backfill for {} failed: {error:#}", channel.id);
                    return false;
                }
            };
            let mut discovered = rusty_channel_to_channel(&page, channel.subscribed);
            let info = youtube_call(
                query.channel_info(&channel.id),
                "extract channel subscriber metadata",
            )
            .await;
            if let Ok(info) = &info {
                discovered.subscriber_count = info
                    .subscriber_count
                    .map(|count| compact_count(count as i64, ""))
                    .unwrap_or_default();
            } else {
                discovered.subscriber_count = channel.subscriber_count.clone();
            }
            match self.store_channel_metadata(&channel, &discovered).await {
                Ok(changed) => {
                    if let Ok(info) = info {
                        match self
                            .store_subscriber_count(&channel.id, info.subscriber_count)
                            .await
                        {
                            Ok(()) => true,
                            Err(error) => {
                                eprintln!(
                                    "could not verify subscriber count for {}: {error:#}",
                                    channel.id
                                );
                                changed
                            }
                        }
                    } else {
                        changed
                    }
                }
                Err(error) => {
                    eprintln!("could not store metadata for {}: {error:#}", channel.id);
                    false
                }
            }
        }))
        .buffer_unordered(CHANNEL_METADATA_CONCURRENCY)
        .filter(|changed| std::future::ready(*changed))
        .count()
        .await
    }

    /// About's named field is authoritative, including a hidden count. Mark
    /// verified rows so the background worker repairs old header counts once.
    pub(crate) async fn store_subscriber_count(
        &self,
        channel_id: &str,
        count: Option<u64>,
    ) -> Result<()> {
        self.db.query("UPDATE channel SET subscriber_count = $count, subscriber_count_verified_at = time::now() WHERE channel_id = $channel_id")
            .bind(("channel_id", channel_id.to_string()))
            .bind(("count", count.map(|count| compact_count(count as i64, "")).unwrap_or_default()))
            .await?.check()?;
        Ok(())
    }

    /// Write fetched channel facts onto an existing row without touching who
    /// follows it.
    ///
    /// `channel.subscribed` is derived from every account's own rows, so a
    /// metadata write must never carry it. Merged rather than replaced, so a
    /// sparse response cannot blank what a richer one stored earlier.
    pub(crate) async fn store_channel_metadata(
        &self,
        existing: &Channel,
        discovered: &Channel,
    ) -> Result<bool> {
        let mut merged = existing.clone();
        if !merged.merge_metadata_from(discovered) {
            return Ok(false);
        }
        self.db
            .query(
                r#"UPDATE channel SET
                    name = $name,
                    handle = $handle,
                    avatar_url = $avatar_url,
                    subscriber_count = $subscriber_count,
                    description = $description,
                    banner_url = $banner_url
                WHERE channel_id = $channel_id"#,
            )
            .bind(("channel_id", merged.id))
            .bind(("name", merged.name))
            .bind(("handle", merged.handle))
            .bind(("avatar_url", merged.avatar_url))
            .bind(("subscriber_count", merged.subscriber_count))
            .bind(("description", merged.description))
            .bind(("banner_url", merged.banner_url))
            .await?
            .check()?;
        Ok(true)
    }

    pub(crate) async fn upsert_playlist(&self, owner: &str, playlist: &Playlist) -> Result<()> {
        self.db
            .query(
                r#"UPSERT type::record('playlist', $record_key) SET
                    owner = type::record($owner),
                    playlist_id = $playlist_id,
                    name = $name,
                    video_ids = $video_ids,
                    updated_at = time::now()"#,
            )
            .bind(("owner", owner.to_string()))
            .bind(("record_key", Self::owner_key(owner, &playlist.id)))
            .bind(("playlist_id", playlist.id.clone()))
            .bind(("name", playlist.name.clone()))
            .bind(("video_ids", playlist.video_ids.clone()))
            .await?
            .check()?;
        Ok(())
    }

    pub(crate) async fn upsert_subscription_group(
        &self,
        owner: &str,
        group: &SubscriptionGroup,
    ) -> Result<()> {
        self.db
            .query(
                r#"UPSERT type::record('subscription_group', $record_key) SET
                    owner = type::record($owner),
                    group_id = $group_id,
                    name = $name,
                    channel_ids = $channel_ids,
                    updated_at = time::now()"#,
            )
            .bind(("owner", owner.to_string()))
            .bind(("record_key", Self::owner_key(owner, &group.id)))
            .bind(("group_id", group.id.clone()))
            .bind(("name", group.name.clone()))
            .bind(("channel_ids", group.channel_ids.clone()))
            .await?
            .check()?;
        Ok(())
    }
}
