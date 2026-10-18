use super::*;

impl AppServerState {
    pub(crate) async fn refresh_channel_rss_fast(&self, channel: Channel) -> Result<usize> {
        let xml = self
            .http
            .get("https://www.youtube.com/feeds/videos.xml")
            .query(&[("channel_id", channel.id.as_str())])
            .send()
            .await
            .context("request YouTube channel RSS")?
            .error_for_status()
            .context("YouTube rejected channel RSS request")?
            .text()
            .await
            .context("read YouTube channel RSS")?;
        let entries = parse_youtube_feed(&xml)?;
        if entries.is_empty() {
            return Err(anyhow!("channel RSS is empty for {}", channel.id));
        }
        for entry in &entries {
            let video = feed_entry_video(entry, &channel);
            self.upsert_feed_hint(&video, entry.published.clone())
                .await?;
        }
        self.db
            .query("UPDATE channel SET last_polled_at = time::now() WHERE channel_id = $channel_id")
            .bind(("channel_id", channel.id))
            .await?
            .check()?;
        Ok(entries.len())
    }

    /// Channels with at least one video of unknown length, most recent first.
    ///
    /// Reads rows rather than grouping: the newest videos are the ones a viewer
    /// is about to filter or sort, and a scan of a few hundred is cheaper than
    /// an aggregate over the whole catalog.
    pub(crate) async fn channels_missing_durations(&self, limit: usize) -> Result<Vec<String>> {
        // `published_sort` is selected because SurrealDB 3.x refuses to order by
        // an idiom the projection does not carry.
        #[derive(Debug, Deserialize, SurrealValue)]
        struct Row {
            channel_id: String,
            #[allow(dead_code)]
            published_sort: String,
        }
        let rows: Vec<Row> = self
            .db
            .query(
                "SELECT channel_id, published_sort FROM video WHERE duration_seconds = 0 AND is_live = false ORDER BY published_sort DESC LIMIT $scan",
            )
            .bind(("scan", FEED_DURATION_SCAN_ROWS as i64))
            .await?
            .take(0)?;
        let mut seen = HashSet::new();
        let mut channels = Vec::new();
        for row in rows {
            if !row.channel_id.starts_with("UC") || row.channel_id.starts_with("UC-tawny") {
                continue;
            }
            if seen.insert(row.channel_id.clone()) {
                channels.push(row.channel_id);
            }
            if channels.len() >= limit {
                break;
            }
        }
        Ok(channels)
    }

    /// Channel ids whose uploads are part of this viewer's feed. The poller has
    /// no viewer, so its empty owner means every channel followed by anyone on
    /// the instance.
    pub(crate) async fn feed_channel_ids(&self, owner: &str) -> Result<Vec<String>> {
        if !owner.is_empty() {
            return Ok(self
                .user_subscriptions(owner)
                .await?
                .into_iter()
                .filter_map(|(channel_id, (subscribed, _))| subscribed.then_some(channel_id))
                .collect());
        }

        #[derive(Debug, Deserialize, SurrealValue)]
        struct Row {
            channel_id: String,
        }
        let rows: Vec<Row> = self
            .db
            .query("SELECT channel_id FROM channel WHERE subscribed = true")
            .await?
            .take(0)?;
        Ok(rows.into_iter().map(|row| row.channel_id).collect())
    }

    /// Every video this account has saved to a playlist.
    ///
    /// A saved video is this viewer's own reference to it, whatever channel it
    /// came from, so it earns the same hydration a subscribed upload gets. A
    /// playlist is the one place a reader assembles videos from channels they
    /// do not follow, which is exactly where a channel-only allowlist left the
    /// duration filters with nothing to filter.
    pub(crate) async fn user_playlist_video_ids(&self, owner: &str) -> Result<HashSet<String>> {
        if owner.is_empty() {
            return Ok(HashSet::new());
        }
        #[derive(Debug, Deserialize, SurrealValue)]
        struct Row {
            video_ids: Vec<String>,
        }
        let rows: Vec<Row> = self
            .db
            .query("SELECT video_ids FROM playlist WHERE owner = type::record($owner)")
            .bind(("owner", owner.to_string()))
            .await?
            .check()?
            .take(0)?;
        Ok(rows
            .into_iter()
            .flat_map(|row| row.video_ids.into_iter())
            .collect())
    }

    /// Newest exact rows that still need a runtime, scoped to the calling
    /// viewer rather than the server's entire shared catalog.
    pub(crate) async fn feed_duration_candidates(
        &self,
        owner: &str,
        limit: usize,
    ) -> Result<Vec<Video>> {
        let channel_ids = self.feed_channel_ids(owner).await?;
        if channel_ids.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let rows: Vec<DbVideo> = self
            .db
            .query(
                "SELECT video_id, title, channel_id, channel_name, thumbnail_url, published_at, published_sort, duration_seconds, view_count, is_live, is_short, progress_seconds, watched, audio_only FROM video WHERE duration_seconds = 0 AND is_live = false AND channel_id IN $channel_ids ORDER BY published_sort DESC LIMIT $limit",
            )
            .bind(("channel_ids", channel_ids))
            .bind(("limit", limit as i64))
            .await?
            .take(0)?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    /// Resolve each requested upload itself. Channel listings are efficient,
    /// but they can omit Shorts and older rows; this exact path is what makes
    /// progress deterministic for visible cards.
    pub(crate) async fn resolve_video_durations(&self, videos: Vec<Video>) -> Result<Vec<Video>> {
        use futures_util::{StreamExt, stream};

        let enriched = stream::iter(
            videos
                .into_iter()
                .map(|video| async move { self.enrich_video_player(video).await }),
        )
        .buffer_unordered(4)
        .collect::<Vec<_>>()
        .await;
        let mut resolved = Vec::new();
        for video in enriched {
            if video.duration_seconds == 0 && !video.is_live {
                continue;
            }
            self.upsert_feed_video(&video, video_sort_key(&video))
                .await?;
            resolved.push(video);
        }
        Ok(resolved)
    }

    /// Metadata hydration for the cards a viewer is looking at, wherever they
    /// are looking at them.
    ///
    /// Accepts only videos already in the catalog that this account has a claim
    /// on - an upload from a channel it follows, or a video it saved to a
    /// playlist - and caps the request before any extractor work begins, so the
    /// endpoint cannot be driven as a general-purpose extractor proxy.
    pub async fn hydrate_video_durations(
        &self,
        owner: &str,
        video_ids: Vec<String>,
    ) -> Result<Vec<Video>> {
        let (allowed_channels, allowed_videos) = tokio::try_join!(
            async {
                Ok::<_, anyhow::Error>(
                    self.feed_channel_ids(owner)
                        .await?
                        .into_iter()
                        .collect::<HashSet<_>>(),
                )
            },
            self.user_playlist_video_ids(owner),
        )?;
        if allowed_channels.is_empty() && allowed_videos.is_empty() {
            return Ok(Vec::new());
        }
        let mut seen = HashSet::new();
        let video_ids = video_ids
            .into_iter()
            .filter(|video_id| seen.insert(video_id.clone()))
            .take(DURATION_HYDRATION_LIMIT)
            .collect::<Vec<_>>();
        if video_ids.is_empty() {
            return Ok(Vec::new());
        }
        let rows: Vec<DbVideo> = self
            .db
            .query(
                "SELECT video_id, title, channel_id, channel_name, thumbnail_url, published_at, published_sort, duration_seconds, view_count, is_live, is_short, progress_seconds, watched, audio_only FROM video WHERE video_id IN $video_ids",
            )
            .bind(("video_ids", video_ids))
            .await?
            .take(0)?;
        let mut resolved = Vec::new();
        let mut missing = Vec::new();
        for video in rows.into_iter().map(Video::from) {
            if !allowed_channels.contains(&video.channel_id) && !allowed_videos.contains(&video.id)
            {
                continue;
            }
            if video.duration_seconds > 0 || video.is_live {
                resolved.push(video);
            } else {
                missing.push(video);
            }
        }
        resolved.extend(self.resolve_video_durations(missing).await?);
        Ok(resolved)
    }

    /// Write lengths a listing reported onto rows that still have none.
    ///
    /// Guarded on `duration_seconds = 0` in the statement rather than in Rust:
    /// the player writes an exact length when a video is opened, and a listing's
    /// rounded seconds must never overwrite it.
    pub(crate) async fn apply_video_durations(
        &self,
        rows: &[(String, u64, String)],
    ) -> Result<usize> {
        let mut filled = 0;
        for (video_id, duration_seconds, view_count) in rows {
            let updated: Vec<serde_json::Value> = self
                .db
                .query(
                    r#"UPDATE video SET
                        duration_seconds = $duration_seconds,
                        view_count = IF $view_count = "" THEN view_count ELSE $view_count END
                    WHERE video_id = $video_id AND duration_seconds = 0 RETURN video_id"#,
                )
                .bind(("video_id", video_id.clone()))
                .bind(("duration_seconds", *duration_seconds as i64))
                .bind(("view_count", view_count.clone()))
                .await?
                .take(0)?;
            filled += updated.len();
        }
        Ok(filled)
    }

    /// Fill in lengths for videos that arrived over RSS.
    ///
    /// YouTube's subscription feed is an Atom document, and the format carries
    /// no duration at all - which is why a video imported from it has none until
    /// something opens it, and why a duration filter used to discard almost the
    /// whole library. A channel's own uploads tab does report lengths, and one
    /// call covers its recent uploads, so this costs one request per channel
    /// with a gap rather than one per video.
    ///
    /// Best effort throughout. A refresh that cannot reach the extractor still
    /// returns the feed it did reconcile; the lengths are simply filled in on a
    /// later refresh.
    pub(crate) async fn backfill_durations(&self, owner: &str) -> usize {
        use futures_util::{StreamExt, stream};

        let Ok(channels) = self
            .channels_missing_durations(FEED_DURATION_BACKFILL_CHANNELS)
            .await
        else {
            return 0;
        };
        let listings = stream::iter(channels.into_iter().map(|channel_id| async move {
            youtube_call(
                self.youtube.query().channel_videos(&channel_id),
                "extract channel videos for lengths",
            )
            .await
        }))
        .buffer_unordered(8)
        .collect::<Vec<_>>()
        .await;

        let mut filled = 0;
        for listing in listings.into_iter().flatten() {
            let rows = listing
                .content
                .items
                .iter()
                .filter(|item| !item.is_live)
                .filter_map(|item| {
                    let duration = u64::from(item.duration?);
                    if duration == 0 {
                        return None;
                    }
                    let views = item
                        .view_count
                        .map(|count| compact_count(count as i64, " views"))
                        .unwrap_or_default();
                    Some((item.id.clone(), duration, views))
                })
                .collect::<Vec<_>>();
            filled += self.apply_video_durations(&rows).await.unwrap_or(0);
        }
        // A listing can leave exactly the same unknown Shorts/older uploads at
        // the top forever. Resolve the newest visible page directly so every
        // successful refresh advances the feed even when that happens.
        if let Ok(candidates) = self
            .feed_duration_candidates(owner, FEED_DURATION_VISIBLE_BATCH)
            .await
            && let Ok(resolved) = self.resolve_video_durations(candidates).await
        {
            filled += resolved.len();
        }
        filled
    }

    pub(crate) async fn reconcile_channels_rss(
        &self,
        channels: Vec<Channel>,
    ) -> (usize, Vec<String>, Vec<String>) {
        use futures_util::{StreamExt, stream};

        let results = stream::iter(channels.into_iter().map(|channel| async move {
            let channel_id = channel.id.clone();
            (channel_id, self.refresh_channel_rss_fast(channel).await)
        }))
        .buffer_unordered(FEED_RSS_CONCURRENCY)
        .collect::<Vec<_>>()
        .await;
        let mut imported = 0;
        let mut refreshed = Vec::new();
        let mut failed = Vec::new();
        for (channel_id, result) in results {
            match result {
                Ok(count) => {
                    imported += count;
                    refreshed.push(channel_id);
                }
                Err(_) => failed.push(channel_id),
            }
        }
        (imported, refreshed, failed)
    }

    pub(crate) async fn refresh_channel_local(&self, channel: Channel) -> Result<usize> {
        let query = self.youtube.query();
        let (rss_result, videos_result, shorts_result, live_result) = tokio::join!(
            youtube_call(query.channel_rss(&channel.id), "extract channel RSS"),
            youtube_call(query.channel_videos(&channel.id), "extract channel videos"),
            youtube_call(
                query.channel_videos_tab(&channel.id, ChannelVideoTab::Shorts),
                "extract channel Shorts",
            ),
            youtube_call(
                query.channel_videos_tab(&channel.id, ChannelVideoTab::Live),
                "extract channel livestreams",
            ),
        );
        let mut normalized = HashMap::<String, (Video, String)>::new();

        if let Ok(shorts) = &shorts_result {
            for item in &shorts.content.items {
                let video =
                    rusty_video_item_to_video(item, Some((&channel.id, &channel.name)), true);
                let sort = item
                    .publish_date
                    .map(|date| date.to_string())
                    .unwrap_or_else(search_sort_key);
                normalized.insert(video.id.clone(), (video, sort));
            }
        }
        if let Ok(videos) = &videos_result {
            let mut enriched_channel = rusty_channel_to_channel(videos, channel.subscribed);
            enriched_channel.subscribed = true;
            enriched_channel.subscription_content = channel.subscription_content;
            self.upsert_channel(&enriched_channel).await?;
            for item in &videos.content.items {
                let video = rusty_video_item_to_video(
                    item,
                    Some((&channel.id, &channel.name)),
                    item.is_short,
                );
                let sort = item
                    .publish_date
                    .map(|date| date.to_string())
                    .unwrap_or_else(search_sort_key);
                normalized.insert(video.id.clone(), (video, sort));
            }
        }
        if let Ok(live) = &live_result {
            for item in &live.content.items {
                let video =
                    rusty_video_item_to_video(item, Some((&channel.id, &channel.name)), false);
                let sort = item
                    .publish_date
                    .map(|date| date.to_string())
                    .unwrap_or_else(search_sort_key);
                normalized.insert(video.id.clone(), (video, sort));
            }
        }
        if let Ok(rss) = rss_result {
            let uncategorized = rss
                .videos
                .into_iter()
                .filter(|item| !normalized.contains_key(&item.id))
                .collect::<Vec<_>>();
            for (video, published) in self.classify_rss_videos(&channel, uncategorized).await {
                normalized
                    .entry(video.id.clone())
                    .or_insert((video, published));
            }
        }

        if normalized.is_empty() {
            return Err(anyhow!("no feed items extracted for {}", channel.id));
        }
        let count = normalized.len();
        for (_, (video, published_sort)) in normalized {
            self.upsert_feed_video(&video, published_sort).await?;
        }
        self.db
            .query(
                "UPDATE channel SET last_polled_at = time::now(), last_full_refresh_at = time::now() WHERE channel_id = $channel_id",
            )
            .bind(("channel_id", channel.id))
            .await?
            .check()?;
        Ok(count)
    }

    pub async fn refresh_feed(&self, owner: &str) -> Result<FeedRefreshResult> {
        let channels: Vec<DbChannel> = self
            .db
            .query(
                "SELECT channel_id, name, handle, avatar_url, subscriber_count, subscribed, subscription_content, description, banner_url, last_polled_at FROM channel WHERE subscribed = true ORDER BY last_polled_at ASC",
            )
            .await?
            .take(0)?;
        let mut real_channels = channels
            .into_iter()
            .map(Channel::from)
            .filter(|channel| channel.id.starts_with("UC") && !channel.id.starts_with("UC-tawny"))
            .collect::<Vec<_>>();
        // The person pulling to refresh wants their own follows current. The
        // list above is every follow on the instance, stalest first, and a
        // capped reconciliation could leave theirs out entirely, so theirs
        // go first and always fit. The sort is stable: staleness orders the
        // rest.
        let own = self
            .user_subscriptions(owner)
            .await?
            .into_iter()
            .filter(|(_, (subscribed, _))| *subscribed)
            .map(|(channel_id, _)| channel_id)
            .collect::<HashSet<_>>();
        real_channels.sort_by_key(|channel| !own.contains(&channel.id));
        let own_count = real_channels
            .iter()
            .filter(|channel| own.contains(&channel.id))
            .count();
        let channel_ids = real_channels
            .iter()
            .map(|channel| channel.id.clone())
            .collect::<Vec<_>>();
        let total_channels = real_channels.len();
        let mut imported = 0;
        let mut sources = Vec::new();
        let mut covered_channels = HashSet::<String>::new();

        if self.websub_callback_url.is_some() {
            sources.push("YouTube WebSub".into());
            covered_channels.extend(channel_ids.iter().cloned());
        }

        // Without a push subscription there is no accelerated source, so every
        // subscribed channel is reconciled over RSS rather than a capped slice.
        let accelerated = self.websub_callback_url.is_some();
        let reconciliation_count = reconciliation_limit(total_channels, accelerated).max(own_count);
        let rss_ids = real_channels
            .iter()
            .take(reconciliation_count)
            .map(|channel| channel.id.clone())
            .collect::<HashSet<_>>();
        let rss_channels = real_channels
            .into_iter()
            .filter(|channel| rss_ids.contains(&channel.id))
            .collect::<Vec<_>>();
        if !rss_channels.is_empty() {
            let (rss_imported, rss_refreshed, _) = self.reconcile_channels_rss(rss_channels).await;
            imported += rss_imported;
            covered_channels.extend(rss_refreshed);
            sources.push("YouTube RSS reconciliation".into());
        }
        if sources.is_empty() {
            sources.push("Local cache".into());
        }
        // After reconciliation, so it sees the videos this refresh just
        // imported, and before the snapshot, so their lengths travel with them.
        self.backfill_durations(owner).await;
        let refreshed_channels = covered_channels.len();
        let failed_channels = total_channels.saturating_sub(refreshed_channels);
        Ok(FeedRefreshResult {
            imported,
            refreshed_channels,
            failed_channels,
            sources,
        })
    }
}
#[derive(Default)]
pub(crate) struct FeedEntry {
    pub(crate) video_id: String,
    pub(crate) channel_id: String,
    pub(crate) title: String,
    pub(crate) published: String,
}

pub(crate) fn feed_entry_video(entry: &FeedEntry, channel: &Channel) -> Video {
    Video {
        id: entry.video_id.clone(),
        title: entry.title.clone(),
        channel_id: channel.id.clone(),
        channel_name: channel.name.clone(),
        thumbnail_url: format!("https://i.ytimg.com/vi/{}/hqdefault.jpg", entry.video_id),
        published_at: entry.published.clone(),
        duration_seconds: 0,
        view_count: String::new(),
        progress_seconds: 0,
        watched: false,
        is_live: false,
        is_short: false,
        audio_only: false,
        channel_avatar_url: None,
    }
}

pub(crate) fn parse_youtube_feed(xml: &str) -> Result<Vec<FeedEntry>> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut entries = Vec::new();
    let mut entry: Option<FeedEntry> = None;
    let mut active_tag = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(tag)) => {
                let name = String::from_utf8_lossy(tag.local_name().as_ref()).into_owned();
                if name == "entry" {
                    entry = Some(FeedEntry::default());
                }
                active_tag = name;
            }
            Ok(Event::Text(text)) => {
                if let Some(entry) = entry.as_mut() {
                    let value = text.unescape()?.into_owned();
                    match active_tag.as_str() {
                        "videoId" => entry.video_id = value,
                        "channelId" => entry.channel_id = value,
                        "title" => entry.title = value,
                        "published" => entry.published = value,
                        _ => {}
                    }
                }
            }
            Ok(Event::End(tag)) => {
                let name = String::from_utf8_lossy(tag.local_name().as_ref()).into_owned();
                if name == "entry"
                    && let Some(entry) = entry.take().filter(|entry| !entry.video_id.is_empty())
                {
                    entries.push(entry);
                }
                active_tag.clear();
            }
            Ok(Event::Eof) => break,
            Err(error) => return Err(error.into()),
            _ => {}
        }
    }
    Ok(entries)
}
