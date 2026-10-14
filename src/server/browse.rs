use super::*;

impl AppServerState {
    pub async fn comments_page(&self, _video_id: &str, next_page: &str) -> Result<CommentsPage> {
        if next_page.starts_with("local-comments:") {
            let paginator = decode_page::<RustyComment>(next_page, "local-comments")?;
            let page = youtube_call(
                paginator.next(self.youtube.query()),
                "fetch next direct comments page",
            )
            .await?
            .ok_or_else(|| anyhow!("comments are exhausted"))?;
            return Ok(rusty_comments_page(&page));
        }
        Ok(CommentsPage::default())
    }

    pub(crate) async fn fetch_direct_video_details(
        &self,
        video_id: &str,
        fallback_video: Option<Video>,
        fallback_channel: Option<Channel>,
    ) -> Result<VideoDetails> {
        let query = self.youtube.query();
        let (details, player) = tokio::join!(
            youtube_call(
                query.video_details(video_id),
                "extract direct YouTube video details"
            ),
            youtube_call(
                query.player_from_clients(video_id, PLAYER_CLIENTS),
                "extract direct YouTube player"
            ),
        );
        let details = details?;
        let comments = match youtube_call(
            details.top_comments.clone().next(query),
            "extract direct YouTube comments",
        )
        .await
        {
            Ok(Some(page)) => rusty_comments_page(&page),
            _ => CommentsPage {
                disabled: details.top_comments.ctoken.is_none(),
                remote_available: true,
                ..CommentsPage::default()
            },
        };
        Ok(normalize_rusty_video_details(
            video_id,
            details,
            player.ok(),
            comments,
            fallback_video,
            fallback_channel,
        ))
    }

    pub async fn video_details(&self, owner: &str, video_id: &str) -> Result<VideoDetails> {
        let mut details = self.video_details_for_anyone(owner, video_id).await?;
        // The details are cached for every account; the progress in them is
        // this one's.
        self.overlay_viewer(owner, std::slice::from_mut(&mut details.video))
            .await?;
        self.overlay_viewer(owner, &mut details.related_videos)
            .await?;
        self.with_owner_follow_state(owner, details).await
    }

    /// The details any account would see. The follow state in them is not
    /// this account's - [`Self::video_details`] sets that.
    pub(crate) async fn video_details_for_anyone(
        &self,
        owner: &str,
        video_id: &str,
    ) -> Result<VideoDetails> {
        if let Some(cached) = self.read_video_details_cache(video_id, true).await? {
            return Ok(self.proxy_captions(cached).await);
        }
        let stale = self.read_video_details_cache(video_id, false).await?;
        let _ = owner;
        let local_video = self
            .videos_by_id("", &[video_id.to_string()])
            .await?
            .into_iter()
            .next();
        let (local_channel, local_related) = match &local_video {
            Some(video) => (
                self.catalog_channel(&video.channel_id).await?,
                // The channel's other uploads stand in for related videos.
                self.catalog_channel_videos(&video.channel_id, 9)
                    .await?
                    .into_iter()
                    .filter(|related| related.id != video_id)
                    .take(8)
                    .collect(),
            ),
            None => (None, Vec::new()),
        };

        if let Ok(mut details) = self
            .fetch_direct_video_details(video_id, local_video.clone(), local_channel.clone())
            .await
        {
            self.keep_finer_dates(std::slice::from_mut(&mut details.video))
                .await?;
            self.keep_finer_dates(&mut details.related_videos).await?;
            if let Some(channel) = &details.channel {
                self.upsert_discovered_channel(channel).await?;
            }
            self.upsert_feed_video(&details.video, video_sort_key(&details.video))
                .await?;
            for related in &details.related_videos {
                self.upsert_feed_video(related, video_sort_key(related))
                    .await?;
            }
            self.write_video_details_cache(&details).await?;
            return Ok(self.proxy_captions(details).await);
        }

        // Direct extraction is the only remote source. When it fails, serve the
        // most recent cached payload, then fall back to whatever the library
        // already knows about the video.
        if let Some(cached) = stale {
            return Ok(self.proxy_captions(cached).await);
        }
        let video = local_video.ok_or_else(|| anyhow!("video is not in the local cache"))?;
        Ok(VideoDetails {
            video,
            channel: local_channel,
            description: String::new(),
            like_count: 0,
            dislike_count: 0,
            captions: Vec::new(),
            chapters: Vec::new(),
            preview_frames: None,
            related_videos: local_related,
            comments: CommentsPage::default(),
            remote_available: false,
        })
    }

    pub(crate) async fn read_video_details_cache(
        &self,
        video_id: &str,
        fresh_only: bool,
    ) -> Result<Option<VideoDetails>> {
        let query = if fresh_only {
            "SELECT payload_json FROM video_detail_cache WHERE video_id = $video_id AND fetched_at > time::now() - 2h LIMIT 1"
        } else {
            "SELECT payload_json FROM video_detail_cache WHERE video_id = $video_id LIMIT 1"
        };
        let cached: Vec<DbVideoDetailCache> = self
            .db
            .query(query)
            .bind(("video_id", video_id.to_string()))
            .await?
            .take(0)?;
        Ok(cached
            .into_iter()
            .next()
            .and_then(|cached| serde_json::from_str(&cached.payload_json).ok()))
    }

    pub(crate) async fn write_video_details_cache(&self, details: &VideoDetails) -> Result<()> {
        self.db
            .query(
                r#"UPSERT type::record('video_detail_cache', $record_key) SET
                    video_id = $video_id,
                    payload_json = $payload_json,
                    fetched_at = time::now()"#,
            )
            .bind(("record_key", details.video.id.clone()))
            .bind(("video_id", details.video.id.clone()))
            .bind(("payload_json", serde_json::to_string(details)?))
            .await?
            .check()?;
        Ok(())
    }

    pub(crate) async fn channel_is_subscribed(&self, channel_id: &str) -> Result<bool> {
        let flags: Vec<DbSubscriptionFlag> = self
            .db
            .query("SELECT subscribed FROM channel WHERE channel_id = $channel_id LIMIT 1")
            .bind(("channel_id", channel_id.to_string()))
            .await?
            .take(0)?;
        Ok(flags
            .first()
            .map(|channel| channel.subscribed)
            .unwrap_or(false))
    }

    pub(crate) async fn search_direct(&self, query: &str, filter: &str) -> Result<SearchResults> {
        let result = youtube_call(
            self.youtube.query().search::<YouTubeItem, _>(query),
            "search YouTube directly",
        )
        .await?;
        let mut videos = Vec::new();
        let mut channels = Vec::new();
        for item in &result.items.items {
            match item {
                YouTubeItem::Video(item) if filter != "channels" => {
                    let video = rusty_video_item_to_video(item, None, item.is_short);
                    self.upsert_feed_video(&video, video_sort_key(&video))
                        .await?;
                    push_unique_video(&mut videos, video);
                }
                YouTubeItem::Channel(item) if filter != "videos" => {
                    let mut channel = rusty_channel_item_to_channel(item);
                    channel.subscribed = self.channel_is_subscribed(&channel.id).await?;
                    self.upsert_discovered_channel(&channel).await?;
                    push_unique_channel(&mut channels, channel);
                }
                _ => {}
            }
        }
        Ok(SearchResults {
            query: query.to_string(),
            videos,
            channels,
            suggestion: result.corrected_query,
            remote_available: true,
            next_page: encode_page("local-search", &result.items),
        })
    }

    pub(crate) async fn classify_rss_videos(
        &self,
        channel: &Channel,
        items: Vec<RustyChannelRssVideo>,
    ) -> Vec<(Video, String)> {
        use futures_util::{StreamExt, stream};

        stream::iter(items.into_iter().map(|item| async move {
            let video = Video {
                id: item.id,
                title: item.name,
                channel_id: channel.id.clone(),
                channel_name: channel.name.clone(),
                thumbnail_url: item.thumbnail.url,
                published_at: item.publish_date.to_string(),
                duration_seconds: 0,
                view_count: compact_count(item.view_count as i64, " views"),
                progress_seconds: 0,
                watched: false,
                is_live: false,
                is_short: false,
                audio_only: false,
                channel_avatar_url: None,
            };
            let video = self.enrich_video_player(video).await;
            let published = video.published_at.clone();
            (video, published)
        }))
        .buffer_unordered(4)
        .collect()
        .await
    }

    pub(crate) async fn enrich_video_player(&self, mut video: Video) -> Video {
        let Ok(player) = youtube_call(
            self.youtube
                .query()
                .player_from_clients(&video.id, PLAYER_CLIENTS),
            "enrich YouTube upload",
        )
        .await
        else {
            return video;
        };
        let vertical = player
            .video_streams
            .iter()
            .chain(player.video_only_streams.iter())
            .any(|stream| stream.height > stream.width);
        video.duration_seconds = player.details.duration as u64;
        video.is_live = player.details.is_live;
        video.is_short = !video.is_live
            && video.duration_seconds > 0
            && video.duration_seconds <= 180
            && vertical;
        if let Some(title) = player.details.name.filter(|title| !title.trim().is_empty()) {
            video.title = title;
        }
        if let Some(thumbnail) = best_thumbnail(&player.details.thumbnail) {
            video.thumbnail_url = thumbnail;
        }
        if let Some(views) = player.details.view_count {
            video.view_count = compact_count(views as i64, " views");
        }
        video
    }

    pub async fn channel_details(
        &self,
        owner: &str,
        channel_id: &str,
        order: &str,
    ) -> Result<ChannelDetails> {
        let mut details = self
            .channel_details_unviewed(owner, channel_id, order)
            .await?;
        for page in [&mut details.videos, &mut details.shorts, &mut details.live] {
            self.overlay_viewer(owner, &mut page.videos).await?;
        }
        Ok(details)
    }

    /// [`Self::channel_details`] before this viewer's progress is applied.
    pub(crate) async fn channel_details_unviewed(
        &self,
        owner: &str,
        channel_id: &str,
        order: &str,
    ) -> Result<ChannelDetails> {
        let query = self.youtube.query();
        let (rss_result, videos_result, shorts_result, live_result, info_result) = tokio::join!(
            youtube_call(query.channel_rss(channel_id), "extract channel RSS"),
            youtube_call(query.channel_videos(channel_id), "extract channel videos"),
            youtube_call(
                query.channel_videos_tab(channel_id, ChannelVideoTab::Shorts),
                "extract channel Shorts",
            ),
            youtube_call(
                query.channel_videos_tab(channel_id, ChannelVideoTab::Live),
                "extract channel livestreams",
            ),
            youtube_call(
                query.channel_info(channel_id),
                "extract channel subscriber metadata"
            ),
        );
        if let Ok(videos_channel) = videos_result {
            // This account's follow, not the instance's - see `user_follows`.
            let follow = self.user_follows(owner, channel_id).await?;
            let mut channel = rusty_channel_to_channel(
                &videos_channel,
                follow.is_some_and(|(subscribed, _)| subscribed),
            );
            // The header parser uses metadata positions and can mistake the
            // upload count for subscribers. About exposes a named field.
            if let Ok(info) = &info_result {
                channel.subscriber_count = info
                    .subscriber_count
                    .map(|count| compact_count(count as i64, ""))
                    .unwrap_or_default();
            } else if let Some(cached) = self.catalog_channel(channel_id).await? {
                channel.merge_metadata_from(&cached);
            }
            if let Some((_, content)) = follow {
                channel.subscription_content = content;
            }
            let mut videos = rusty_page(
                channel_id,
                ChannelMediaTab::Videos,
                &videos_channel.content,
                &channel.name,
            );
            let mut shorts = shorts_result
                .ok()
                .map(|result| {
                    rusty_page(
                        channel_id,
                        ChannelMediaTab::Shorts,
                        &result.content,
                        &channel.name,
                    )
                })
                .unwrap_or_else(|| {
                    empty_channel_page(channel_id, ChannelMediaTab::Shorts, "Direct YouTube")
                });
            let mixed_shorts = videos
                .videos
                .iter()
                .filter(|video| video.is_short)
                .cloned()
                .collect::<Vec<_>>();
            videos.videos.retain(|video| !video.is_short);
            for video in mixed_shorts {
                push_unique_video(&mut shorts.videos, video);
            }
            // The extractor cannot read the Shorts tab, so ask yt-dlp before
            // falling back to guessing from the RSS feed.
            if shorts.videos.is_empty() {
                for video in self.ytdlp_channel_shorts(channel_id, &channel.name).await {
                    push_unique_video(&mut shorts.videos, video);
                }
            }
            if shorts.videos.is_empty()
                && let Ok(rss) = rss_result
            {
                for (video, _) in self.classify_rss_videos(&channel, rss.videos).await {
                    if video.is_short {
                        push_unique_video(&mut shorts.videos, video);
                    } else if video.is_live {
                        // Added below once the live page exists.
                    }
                }
            }
            let mut live = live_result
                .ok()
                .map(|result| {
                    rusty_page(
                        channel_id,
                        ChannelMediaTab::Live,
                        &result.content,
                        &channel.name,
                    )
                })
                .unwrap_or_else(|| {
                    empty_channel_page(channel_id, ChannelMediaTab::Live, "Direct YouTube")
                });
            // rustypipe's current channel parser loses dates/views and its
            // synthesized sort tokens return newest for every order. Flat
            // yt-dlp listings retain those facts, including for later pages.
            for page in [&mut videos, &mut shorts, &mut live] {
                if let Ok(listed) = self
                    .channel_uploads_page(channel_id, &channel.name, page.tab, order, 0)
                    .await
                {
                    *page = listed;
                }
            }
            self.upsert_discovered_channel(&channel).await?;
            if let Ok(info) = info_result {
                self.store_subscriber_count(channel_id, info.subscriber_count)
                    .await?;
            }
            for video in videos
                .videos
                .iter()
                .chain(shorts.videos.iter())
                .chain(live.videos.iter())
            {
                let sort = video_sort_key(video);
                self.upsert_feed_video(video, sort).await?;
            }
            return Ok(ChannelDetails {
                channel,
                videos,
                shorts,
                live,
                remote_available: true,
            });
        }

        let mut channel = self
            .catalog_channel(channel_id)
            .await?
            .ok_or_else(|| anyhow!("channel is unavailable"))?;
        let (subscribed, content) = self
            .user_follows(owner, channel_id)
            .await?
            .unwrap_or((false, channel.subscription_content));
        channel.subscribed = subscribed;
        channel.subscription_content = content;
        let mut videos = Vec::new();
        let mut shorts = Vec::new();
        let mut live = Vec::new();
        for video in self.catalog_channel_videos(channel_id, 200).await? {
            if video.is_short {
                shorts.push(video);
            } else if video.is_live {
                live.push(video);
            } else {
                videos.push(video);
            }
        }
        Ok(ChannelDetails {
            channel,
            videos: ChannelMediaPage {
                channel_id: channel_id.into(),
                tab: ChannelMediaTab::Videos,
                videos,
                next_page: None,
                source: "Cache".into(),
            },
            shorts: ChannelMediaPage {
                channel_id: channel_id.into(),
                tab: ChannelMediaTab::Shorts,
                videos: shorts,
                next_page: None,
                source: "Cache".into(),
            },
            live: ChannelMediaPage {
                channel_id: channel_id.into(),
                tab: ChannelMediaTab::Live,
                videos: live,
                next_page: None,
                source: "Cache".into(),
            },
            remote_available: false,
        })
    }

    /// A page of a channel's uploads.
    ///
    /// Pure catalog data - the rows here are the same for everyone. The owner
    /// is taken so the endpoint still refuses an unauthenticated caller, and is
    /// otherwise unused: watched state reaches these cards from the client's
    /// own library, which is already scoped to the account.
    pub async fn channel_media_page(
        &self,
        _owner: &str,
        channel_id: &str,
        tab: ChannelMediaTab,
        next_page: &str,
    ) -> Result<ChannelMediaPage> {
        if let Some(token) = next_page.strip_prefix("ytdlp-channel:") {
            let (order, offset) = token
                .split_once(':')
                .ok_or_else(|| anyhow!("invalid channel continuation"))?;
            let offset = offset.parse::<usize>()?;
            let name = self
                .catalog_channel(channel_id)
                .await?
                .map(|channel| channel.name)
                .unwrap_or_default();
            let page = self
                .channel_uploads_page(channel_id, &name, tab, order, offset)
                .await?;
            for video in &page.videos {
                self.upsert_feed_video(video, video_sort_key(video)).await?;
            }
            return Ok(page);
        }
        if next_page.starts_with("local-channel:") {
            let paginator = decode_page::<RustyVideoItem>(next_page, "local-channel")?;
            let page = youtube_call(
                paginator.next(self.youtube.query()),
                "fetch next direct channel page",
            )
            .await?
            .ok_or_else(|| anyhow!("channel page is exhausted"))?;
            let channel_name = self
                .catalog_channel(channel_id)
                .await?
                .map(|channel| channel.name)
                .unwrap_or_else(|| "YouTube channel".into());
            let result = rusty_page(channel_id, tab, &page, &channel_name);
            for video in &result.videos {
                self.upsert_feed_video(video, video_sort_key(video)).await?;
            }
            return Ok(result);
        }
        Err(anyhow!("invalid channel continuation"))
    }

    async fn channel_uploads_page(
        &self,
        channel_id: &str,
        name: &str,
        tab: ChannelMediaTab,
        order: &str,
        offset: usize,
    ) -> Result<ChannelMediaPage> {
        static UPLOADS: ServerCache<(String, String), Vec<Video>> =
            ServerCache::new(Duration::from_secs(600), 32);
        let tab_key = match tab {
            ChannelMediaTab::Videos => "videos",
            ChannelMediaTab::Shorts => "shorts",
            ChannelMediaTab::Live => "live",
        };
        let mut uploads = UPLOADS
            .try_get_with(
                (channel_id.into(), tab_key.into()),
                self.ytdlp_channel_uploads(channel_id, name, tab),
            )
            .await
            .map_err(|error| anyhow!("{error}"))?;
        let order = match order {
            "oldest" | "popular" | "unpopular" => order,
            _ => "newest",
        };
        sort_channel_uploads(&mut uploads, order);
        let next_offset = offset.saturating_add(30);
        Ok(ChannelMediaPage {
            channel_id: channel_id.into(),
            tab,
            videos: uploads.iter().skip(offset).take(30).cloned().collect(),
            next_page: (next_offset < uploads.len())
                .then(|| format!("ytdlp-channel:{order}:{next_offset}")),
            source: "Direct YouTube · yt-dlp".into(),
        })
    }

    pub async fn search_catalog(
        &self,
        owner: &str,
        query: &str,
        filter: &str,
    ) -> Result<SearchResults> {
        let results = self.search_catalog_for_anyone(owner, query, filter).await?;
        self.with_owner_follows_in_search(owner, results).await
    }

    /// Search as any account would see it; the follow state in it is set by
    /// [`Self::search_catalog`].
    pub(crate) async fn search_catalog_for_anyone(
        &self,
        owner: &str,
        query: &str,
        filter: &str,
    ) -> Result<SearchResults> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(SearchResults {
                query: String::new(),
                videos: Vec::new(),
                channels: Vec::new(),
                suggestion: None,
                remote_available: true,
                next_page: None,
            });
        }
        let filter = match filter {
            "videos" | "channels" | "playlists" => filter,
            _ => "all",
        };
        let cache_key = (query.to_lowercase(), filter.to_string());
        // Identical searches share one upstream request, and searches take
        // turns: type-ahead bursts and several clients never reach YouTube at
        // once.
        let searched = SEARCHES
            .try_get_with(cache_key, async {
                let _turn = self.search_lock.lock().await;
                self.search_direct(query, filter).await
            })
            .await;
        match searched {
            Ok(result) => Ok(result),
            // Never into the shared cache: this searches the catalog for the
            // asking account, with its own progress.
            Err(_) => self.search_cached(owner, query, filter).await,
        }
    }

    pub async fn search_page(
        &self,
        owner: &str,
        query_text: &str,
        filter: &str,
        next_page: &str,
    ) -> Result<SearchResults> {
        let results = self
            .search_page_for_anyone(query_text, filter, next_page)
            .await?;
        self.with_owner_follows_in_search(owner, results).await
    }

    pub(crate) async fn search_page_for_anyone(
        &self,
        query_text: &str,
        filter: &str,
        next_page: &str,
    ) -> Result<SearchResults> {
        let paginator = decode_page::<YouTubeItem>(next_page, "local-search")?;
        let page = youtube_call(
            paginator.next(self.youtube.query()),
            "fetch next direct YouTube search page",
        )
        .await?
        .ok_or_else(|| anyhow!("search results are exhausted"))?;
        let mut videos = Vec::new();
        let mut channels = Vec::new();
        for item in &page.items {
            match item {
                YouTubeItem::Video(item) if filter != "channels" => {
                    let video = rusty_video_item_to_video(item, None, item.is_short);
                    self.upsert_feed_video(&video, video_sort_key(&video))
                        .await?;
                    push_unique_video(&mut videos, video);
                }
                YouTubeItem::Channel(item) if filter != "videos" => {
                    let mut channel = rusty_channel_item_to_channel(item);
                    channel.subscribed = self.channel_is_subscribed(&channel.id).await?;
                    self.upsert_discovered_channel(&channel).await?;
                    push_unique_channel(&mut channels, channel);
                }
                _ => {}
            }
        }
        Ok(SearchResults {
            query: query_text.to_string(),
            videos,
            channels,
            suggestion: None,
            remote_available: true,
            next_page: encode_page("local-search", &page),
        })
    }

    /// Search what the catalog already holds, for when YouTube cannot be
    /// reached. Matched in the database and capped, never by loading it all.
    pub(crate) async fn search_cached(
        &self,
        owner: &str,
        query: &str,
        filter: &str,
    ) -> Result<SearchResults> {
        const LIMIT: i64 = 60;
        let needle = query.to_lowercase();
        let mut videos = if filter == "channels" {
            Vec::new()
        } else {
            let rows: Vec<DbVideo> = self
                .db
                .query(format!(
                    "SELECT {} FROM video WHERE string::contains(string::lowercase(title), $needle) OR string::contains(string::lowercase(channel_name), $needle) ORDER BY published_sort DESC LIMIT $limit",
                    viewer::VIDEO_COLUMNS
                ))
                .bind(("needle", needle.clone()))
                .bind(("limit", LIMIT))
                .await?
                .check()?
                .take(0)?;
            rows.into_iter().map(Video::from).collect::<Vec<_>>()
        };
        self.overlay_viewer(owner, &mut videos).await?;
        let channels = if filter == "videos" {
            Vec::new()
        } else {
            let rows: Vec<DbChannel> = self
                .db
                .query(format!(
                    "SELECT {} FROM channel WHERE string::contains(string::lowercase(name), $needle) OR string::contains(string::lowercase(handle), $needle) LIMIT $limit",
                    viewer::CHANNEL_COLUMNS
                ))
                .bind(("needle", needle))
                .bind(("limit", LIMIT))
                .await?
                .check()?
                .take(0)?;
            rows.into_iter().map(Channel::from).collect()
        };
        Ok(SearchResults {
            query: query.to_string(),
            videos,
            channels,
            suggestion: None,
            remote_available: false,
            next_page: None,
        })
    }
}
