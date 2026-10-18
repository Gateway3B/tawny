use super::*;

pub(crate) async fn youtube_call<T, E, F>(future: F, operation: &str) -> Result<T>
where
    E: std::fmt::Display,
    F: Future<Output = std::result::Result<T, E>>,
{
    match std::panic::AssertUnwindSafe(future).catch_unwind().await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => Err(anyhow!("{operation}: {error}")),
        Err(_) => Err(anyhow!("{operation}: the extractor aborted its request")),
    }
}

pub(crate) fn center_vtt_cues(input: &str) -> String {
    input
        .lines()
        .map(|line| {
            if !line.contains("-->") {
                return line.to_string();
            }
            let mut settings = line
                .split_whitespace()
                .filter(|part| !part.starts_with("align:") && !part.starts_with("position:"))
                .collect::<Vec<_>>()
                .join(" ");
            settings.push_str(" align:center position:50%");
            settings
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn best_thumbnail(thumbnails: &[rustypipe::model::Thumbnail]) -> Option<String> {
    thumbnails
        .iter()
        .max_by_key(|thumbnail| u64::from(thumbnail.width) * u64::from(thumbnail.height))
        .map(|thumbnail| thumbnail.url.clone())
}

pub(crate) fn rusty_channel_to_channel<T>(channel: &RustyChannel<T>, subscribed: bool) -> Channel {
    Channel {
        id: channel.id.clone(),
        name: channel.name.clone(),
        handle: channel.handle.clone().unwrap_or_else(|| {
            handle_for(
                &channel.name,
                &format!("https://youtube.com/channel/{}", channel.id),
            )
        }),
        avatar_url: best_thumbnail(&channel.avatar),
        subscriber_count: channel
            .subscriber_count
            .map(|count| compact_count(count as i64, ""))
            .unwrap_or_default(),
        subscribed,
        description: channel.description.clone(),
        banner_url: best_thumbnail(&channel.banner),
        subscription_content: SubscriptionContent::default(),
    }
}

pub(crate) fn rusty_channel_item_to_channel(item: &RustyChannelItem) -> Channel {
    Channel {
        id: item.id.clone(),
        name: item.name.clone(),
        handle: item.handle.clone().unwrap_or_else(|| {
            handle_for(
                &item.name,
                &format!("https://youtube.com/channel/{}", item.id),
            )
        }),
        avatar_url: best_thumbnail(&item.avatar),
        subscriber_count: item
            .subscriber_count
            .map(|count| compact_count(count as i64, ""))
            .unwrap_or_default(),
        subscribed: false,
        description: item.short_description.clone(),
        banner_url: None,
        subscription_content: SubscriptionContent::default(),
    }
}

pub(crate) fn rusty_video_item_to_video(
    item: &RustyVideoItem,
    fallback_channel: Option<(&str, &str)>,
    force_short: bool,
) -> Video {
    let channel_id = item
        .channel
        .as_ref()
        .map(|channel| channel.id.clone())
        .or_else(|| fallback_channel.map(|(id, _)| id.to_string()))
        .unwrap_or_else(|| fallback_channel_id("Unknown channel"));
    // Recommendation entries sometimes omit the uploader. An empty name reads
    // as "no channel to show" and the card hides the line, which is better than
    // captioning every such video "Unknown channel".
    let channel_name = item
        .channel
        .as_ref()
        .map(|channel| channel.name.clone())
        .or_else(|| fallback_channel.map(|(_, name)| name.to_string()))
        .unwrap_or_default();
    Video {
        id: item.id.clone(),
        title: item.name.clone(),
        channel_id,
        channel_name,
        thumbnail_url: best_thumbnail(&item.thumbnail)
            .unwrap_or_else(|| format!("https://i.ytimg.com/vi/{}/hqdefault.jpg", item.id)),
        published_at: item
            .publish_date_txt
            .clone()
            .or_else(|| item.publish_date.map(|date| date.to_string()))
            .unwrap_or_else(|| {
                if item.is_live {
                    "Streaming now".into()
                } else {
                    "From YouTube".into()
                }
            }),
        duration_seconds: item.duration.unwrap_or_default() as u64,
        view_count: item
            .view_count
            .map(|count| {
                compact_count(
                    count as i64,
                    if item.is_live { " watching" } else { " views" },
                )
            })
            .unwrap_or_default(),
        progress_seconds: 0,
        watched: false,
        is_live: item.is_live,
        is_short: force_short || item.is_short,
        audio_only: false,
        channel_avatar_url: None,
    }
}

pub(crate) fn encode_page<T: Serialize>(prefix: &str, paginator: &Paginator<T>) -> Option<String> {
    paginator.ctoken.as_ref()?;
    // The token travels in a GET query string. The items already shown are
    // no use to the next fetch and, for comments, made it long enough to be
    // refused, so only what continues the list is kept.
    let mut value = serde_json::to_value(paginator).ok()?;
    value["items"] = serde_json::Value::Array(Vec::new());
    serde_json::to_vec(&value)
        .ok()
        .map(|json| format!("{prefix}:{}", URL_SAFE_NO_PAD.encode(json)))
}

pub(crate) fn decode_page<T: DeserializeOwned>(token: &str, prefix: &str) -> Result<Paginator<T>> {
    let encoded = token
        .strip_prefix(&format!("{prefix}:"))
        .ok_or_else(|| anyhow!("unsupported continuation token"))?;
    let bytes = URL_SAFE_NO_PAD
        .decode(encoded)
        .context("decode continuation token")?;
    serde_json::from_slice(&bytes).context("deserialize continuation token")
}

pub(crate) fn rusty_page(
    channel_id: &str,
    tab: ChannelMediaTab,
    paginator: &Paginator<RustyVideoItem>,
    fallback_name: &str,
) -> ChannelMediaPage {
    ChannelMediaPage {
        channel_id: channel_id.to_string(),
        tab,
        videos: paginator
            .items
            .iter()
            .map(|item| {
                rusty_video_item_to_video(
                    item,
                    Some((channel_id, fallback_name)),
                    tab == ChannelMediaTab::Shorts,
                )
            })
            .collect(),
        next_page: encode_page("local-channel", paginator),
        source: "Direct YouTube".into(),
    }
}

pub(crate) fn empty_channel_page(
    channel_id: &str,
    tab: ChannelMediaTab,
    source: &str,
) -> ChannelMediaPage {
    ChannelMediaPage {
        channel_id: channel_id.to_string(),
        tab,
        videos: Vec::new(),
        next_page: None,
        source: source.into(),
    }
}

pub(crate) fn embed_url(video_id: &str) -> String {
    format!(
        "https://www.youtube-nocookie.com/embed/{video_id}?enablejsapi=1&autoplay=1&playsinline=1"
    )
}

pub(crate) fn rusty_comment(comment: &RustyComment) -> VideoComment {
    VideoComment {
        id: comment.id.clone(),
        author: comment
            .author
            .as_ref()
            .map(|author| author.name.clone())
            .unwrap_or_else(|| "Deleted user".into()),
        author_avatar_url: comment
            .author
            .as_ref()
            .and_then(|author| best_thumbnail(&author.avatar)),
        author_channel_id: comment.author.as_ref().map(|author| author.id.clone()),
        text: comment.text.to_plaintext(),
        published_at: comment.publish_date_txt.clone(),
        like_count: comment.like_count.unwrap_or_default() as u64,
        reply_count: comment.reply_count as u64,
        pinned: comment.pinned,
        hearted: comment.hearted,
        creator: comment.by_owner,
    }
}

pub(crate) fn rusty_comments_page(paginator: &Paginator<RustyComment>) -> CommentsPage {
    CommentsPage {
        comments: paginator.items.iter().map(rusty_comment).collect(),
        next_page: encode_page("local-comments", paginator),
        disabled: paginator.items.is_empty() && paginator.ctoken.is_none(),
        remote_available: true,
    }
}

pub(crate) fn normalize_rusty_video_details(
    video_id: &str,
    details: RustyVideoDetails,
    player: Option<RustyVideoPlayer>,
    comments: CommentsPage,
    fallback_video: Option<Video>,
    fallback_channel: Option<Channel>,
) -> VideoDetails {
    let duration = player
        .as_ref()
        .map(|player| player.details.duration as u64)
        .or_else(|| fallback_video.as_ref().map(|video| video.duration_seconds))
        .unwrap_or_default();
    let thumbnail_url = player
        .as_ref()
        .and_then(|player| best_thumbnail(&player.details.thumbnail))
        .or_else(|| {
            fallback_video
                .as_ref()
                .map(|video| video.thumbnail_url.clone())
        })
        .unwrap_or_else(|| format!("https://i.ytimg.com/vi/{video_id}/hqdefault.jpg"));
    let channel = Channel {
        id: details.channel.id.clone(),
        name: details.channel.name.clone(),
        handle: fallback_channel
            .as_ref()
            .map(|channel| channel.handle.clone())
            .unwrap_or_else(|| {
                handle_for(
                    &details.channel.name,
                    &format!("https://youtube.com/channel/{}", details.channel.id),
                )
            }),
        avatar_url: best_thumbnail(&details.channel.avatar).or_else(|| {
            fallback_channel
                .as_ref()
                .and_then(|channel| channel.avatar_url.clone())
        }),
        subscriber_count: details
            .channel
            .subscriber_count
            .map(|count| compact_count(count as i64, ""))
            .or_else(|| {
                fallback_channel
                    .as_ref()
                    .map(|channel| channel.subscriber_count.clone())
            })
            .unwrap_or_default(),
        subscribed: fallback_channel
            .as_ref()
            .map(|channel| channel.subscribed)
            .unwrap_or(false),
        description: fallback_channel
            .as_ref()
            .map(|channel| channel.description.clone())
            .unwrap_or_default(),
        subscription_content: fallback_channel
            .as_ref()
            .map(|channel| channel.subscription_content)
            .unwrap_or_default(),
        banner_url: fallback_channel.and_then(|channel| channel.banner_url),
    };
    let video = Video {
        id: video_id.into(),
        title: details.name.clone(),
        channel_id: channel.id.clone(),
        channel_name: channel.name.clone(),
        thumbnail_url,
        // The machine date first. The text is localised ("Aug 2, 2013",
        // "Premiered ..."), and a date that cannot be sorted on sank the video
        // to the bottom of every newest-first list the moment it was opened.
        // `published_label` renders this as the same "Aug 2, 2013".
        published_at: details
            .publish_date
            .map(|date| date.to_string())
            .or_else(|| details.publish_date_txt.clone())
            .unwrap_or_else(|| "From YouTube".into()),
        duration_seconds: duration,
        view_count: compact_count(details.view_count as i64, " views"),
        progress_seconds: fallback_video
            .as_ref()
            .map(|video| video.progress_seconds)
            .unwrap_or_default(),
        watched: fallback_video
            .as_ref()
            .map(|video| video.watched)
            .unwrap_or(false),
        is_live: details.is_live,
        is_short: fallback_video
            .as_ref()
            .map(|video| video.is_short)
            .unwrap_or_else(|| {
                player.as_ref().is_some_and(|player| {
                    player.details.duration <= 180
                        && player
                            .video_streams
                            .iter()
                            .any(|stream| stream.height > stream.width)
                })
            }),
        audio_only: false,
        channel_avatar_url: None,
    };
    let captions = player
        .as_ref()
        .map(|player| {
            player
                .subtitles
                .iter()
                .map(|subtitle| CaptionTrack {
                    label: subtitle.lang_name.clone(),
                    language_code: subtitle.lang.clone(),
                    mime_type: "text/vtt".into(),
                    url: if subtitle.url.contains("fmt=") {
                        subtitle.url.clone()
                    } else if subtitle.url.contains('?') {
                        format!("{}&fmt=vtt", subtitle.url)
                    } else {
                        format!("{}?fmt=vtt", subtitle.url)
                    },
                    auto_generated: subtitle.auto_generated,
                })
                .collect()
        })
        .unwrap_or_default();
    let preview_frames = player.as_ref().and_then(|player| {
        player
            .preview_frames
            .iter()
            .filter(|frames| frames.page_count > 0 && frames.total_count > 0)
            .max_by_key(|frames| frames.frame_width)
            .map(|frames| VideoPreviewFrames {
                page_urls: frames.urls().collect(),
                frame_width: frames.frame_width,
                frame_height: frames.frame_height,
                total_count: frames.total_count,
                duration_per_frame_ms: frames.duration_per_frame,
                frames_per_page_x: frames.frames_per_page_x,
                frames_per_page_y: frames.frames_per_page_y,
            })
    });
    let description = details.description.to_plaintext();
    // YouTube only reports structured chapters when the uploader's markers were
    // recognised. Plenty of videos just list timestamps in the description, so
    // fall back to parsing those rather than showing no chapters at all.
    let mut chapters = details
        .chapters
        .iter()
        .map(|chapter| VideoChapter {
            title: chapter.name.clone(),
            start_seconds: chapter.position as u64,
        })
        .collect::<Vec<_>>();
    if chapters.is_empty() {
        chapters = extract_chapters(&description);
    }
    VideoDetails {
        video,
        channel: Some(channel),
        description,
        like_count: details.like_count.unwrap_or_default() as u64,
        dislike_count: 0,
        captions,
        chapters,
        preview_frames,
        related_videos: details
            .recommended
            .items
            .iter()
            .map(|item| rusty_video_item_to_video(item, None, item.is_short))
            .collect(),
        comments,
        remote_available: true,
    }
}

pub(crate) fn parse_timestamp(value: &str) -> Option<u64> {
    let parts = value
        .trim_matches(|character: char| !character.is_ascii_digit() && character != ':')
        .split(':')
        .map(str::parse::<u64>)
        .collect::<std::result::Result<Vec<_>, _>>()
        .ok()?;
    match parts.as_slice() {
        [minutes, seconds] if *seconds < 60 => Some(minutes * 60 + seconds),
        [hours, minutes, seconds] if *minutes < 60 && *seconds < 60 => {
            Some(hours * 3600 + minutes * 60 + seconds)
        }
        _ => None,
    }
}

pub(crate) fn extract_chapters(description: &str) -> Vec<VideoChapter> {
    let mut chapters = description
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            // Chapter lists are often bulleted or numbered, so the timestamp
            // is not guaranteed to be the very first token.
            let (timestamp, start_seconds) = line
                .split_whitespace()
                .take(3)
                .find_map(|token| parse_timestamp(token).map(|seconds| (token, seconds)))?;
            let timestamp_end = line.find(timestamp)? + timestamp.len();
            let title = line[timestamp_end..]
                .trim()
                .trim_start_matches(['-', '–', '—', ':', ')', ']'])
                .trim();
            (!title.is_empty()).then(|| VideoChapter {
                title: title.to_string(),
                start_seconds,
            })
        })
        .collect::<Vec<_>>();
    chapters.sort_by_key(|chapter| chapter.start_seconds);
    chapters.dedup_by_key(|chapter| chapter.start_seconds);
    if chapters.len() >= 2
        && chapters
            .first()
            .is_some_and(|chapter| chapter.start_seconds <= 1)
    {
        chapters
    } else {
        Vec::new()
    }
}

pub(crate) fn fallback_channel_id(name: &str) -> String {
    let mut hasher = DefaultHasher::new();
    name.hash(&mut hasher);
    format!("discovered-{:x}", hasher.finish())
}

pub(crate) fn compact_count(value: i64, suffix: &str) -> String {
    let value = value.max(0) as f64;
    let count = if value >= 1_000_000_000.0 {
        format!("{:.1}B", value / 1_000_000_000.0)
    } else if value >= 1_000_000.0 {
        format!("{:.1}M", value / 1_000_000.0)
    } else if value >= 1_000.0 {
        format!("{:.1}K", value / 1_000.0)
    } else {
        format!("{}", value as i64)
    };
    format!("{}{}", count.trim_end_matches(".0"), suffix)
}

pub(crate) fn sort_channel_uploads(videos: &mut [Video], order: &str) {
    match order {
        // The channel listing is chronological even where YouTube omits dates.
        "oldest" => videos.reverse(),
        "popular" | "unpopular" => videos.sort_by(|a, b| {
            match (
                crate::models::compact_count_value(&a.view_count),
                crate::models::compact_count_value(&b.view_count),
            ) {
                (Some(a), Some(b)) if order == "popular" => b.total_cmp(&a),
                (Some(a), Some(b)) => a.total_cmp(&b),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            }
        }),
        _ => {}
    }
}

pub(crate) fn handle_for(name: &str, url: &str) -> String {
    if let Some(handle) = url
        .split('/')
        .find(|segment| segment.starts_with('@') && segment.len() > 1)
    {
        return handle.to_string();
    }
    format!(
        "@{}",
        name.to_lowercase()
            .chars()
            .filter(|character| character.is_ascii_alphanumeric())
            .collect::<String>()
    )
}

pub(crate) fn push_unique_video(videos: &mut Vec<Video>, video: Video) {
    if !videos.iter().any(|existing| existing.id == video.id) {
        videos.push(video);
    }
}

pub(crate) fn push_unique_channel(channels: &mut Vec<Channel>, channel: Channel) {
    if !channels.iter().any(|existing| existing.id == channel.id) {
        channels.push(channel);
    }
}

pub(crate) fn search_sort_key() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("{millis:020}")
}

pub(crate) fn video_sort_key(video: &Video) -> String {
    canonical_sort_key(&video.published_at)
}

pub(crate) fn canonical_sort_key(value: &str) -> String {
    if value.len() == 20 && value.bytes().all(|byte| byte.is_ascii_digit()) {
        return value.to_string();
    }
    format!("{:020}", video_published_epoch(value).max(0))
}

pub(crate) fn video_published_epoch(value: &str) -> i64 {
    use time::{Date, OffsetDateTime, format_description::well_known::Rfc3339};

    let value = value.trim();
    // An approximate moment (`~` and a timestamp; see `Video::settle_relative_date`).
    let value = value.strip_prefix('~').unwrap_or(value);
    if let Ok(timestamp) = OffsetDateTime::parse(value, &Rfc3339) {
        return timestamp.unix_timestamp();
    }
    if value.len() >= 10
        && let Ok(format) = time::format_description::parse_borrowed::<2>("[year]-[month]-[day]")
        && let Ok(date) = Date::parse(&value[..10], &format)
    {
        return date.midnight().assume_utc().unix_timestamp();
    }
    if let Some(date) = crate::models::text_date(value) {
        return date.midnight().assume_utc().unix_timestamp();
    }

    let lower = value.to_ascii_lowercase();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    if lower.contains("streaming now") || lower == "live" || lower.contains("just now") {
        return now;
    }
    if lower.contains("yesterday") {
        return now - 86_400;
    }

    let words = lower.split_whitespace().collect::<Vec<_>>();
    for (index, word) in words.iter().enumerate() {
        let count = word
            .trim_matches(|character: char| !character.is_ascii_digit())
            .parse::<i64>()
            .ok()
            .or_else(|| matches!(*word, "a" | "an").then_some(1));
        let Some(count) = count else { continue };
        let Some(unit) = words.get(index + 1) else {
            continue;
        };
        let seconds = if unit.starts_with("second") {
            1
        } else if unit.starts_with("minute") {
            60
        } else if unit.starts_with("hour") {
            3_600
        } else if unit.starts_with("day") {
            86_400
        } else if unit.starts_with("week") {
            7 * 86_400
        } else if unit.starts_with("month") {
            30 * 86_400
        } else if unit.starts_with("year") {
            365 * 86_400
        } else {
            continue;
        };
        return now.saturating_sub(count.saturating_mul(seconds));
    }
    0
}
