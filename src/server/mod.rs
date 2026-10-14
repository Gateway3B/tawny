use crate::models::{
    CaptionTrack, Channel, ChannelDetails, ChannelMediaPage, ChannelMediaTab, CommentsPage,
    FeedRefreshResult, HistoryEntry, PlaybackByteRange, PlaybackFailure, PlaybackFailureOrigin,
    PlaybackProtocol, PlaybackSession, PlaybackSource, PlaybackTrack, PlaybackTrackKind, Playlist,
    SearchResults, SponsorCategory, SponsorSegment, SubscriptionContent, SubscriptionGroup, Video,
    VideoChapter, VideoComment, VideoDetails, VideoPreviewFrames, VideoProgress,
};
use anyhow::{Context, Result, anyhow};
use axum::{
    Extension,
    body::{Body, Bytes},
    extract::{Path, Query},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::Response,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use dioxus::fullstack::FullstackContext;
use futures_util::FutureExt;
use g3_cache::ServerCache;
use hmac::{Hmac, Mac};
use quick_xml::{Reader, events::Event};
use rustypipe::{
    client::{ClientType, RustyPipe},
    model::{
        Channel as RustyChannel, ChannelItem as RustyChannelItem,
        ChannelRssVideo as RustyChannelRssVideo, Comment as RustyComment,
        VideoDetails as RustyVideoDetails, VideoItem as RustyVideoItem,
        VideoPlayer as RustyVideoPlayer, YouTubeItem, paginator::Paginator, richtext::ToPlaintext,
    },
    param::ChannelVideoTab,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha1::Sha1;
use sha2::{Digest, Sha256};
use std::future::Future;
use std::{
    collections::{HashMap, HashSet, hash_map::DefaultHasher},
    hash::{Hash, Hasher},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use surrealdb::{
    Surreal,
    engine::any::{Any, connect},
    opt::auth::Root,
};
use surrealdb_types::SurrealValue;

mod browse;
mod feed;
mod proxy;
mod resolve;
mod segments;
mod store;
mod viewer;
mod websub;
mod youtube;
mod ytdlp;
pub(crate) use feed::*;
pub(crate) use proxy::*;
pub(crate) use segments::*;
pub(crate) use websub::*;
pub(crate) use youtube::*;
pub(crate) use ytdlp::*;

#[derive(Clone)]
pub struct AppServerState {
    pub(crate) db: Arc<Surreal<Any>>,
    http: reqwest::Client,
    media_http: reqwest::Client,
    ytdlp_http: reqwest::Client,
    youtube: Arc<RustyPipe>,
    /// Optional containerized yt-dlp HTTP sidecar. When configured, Tawny does
    /// not discover or execute a host yt-dlp binary.
    ytdlp_service_url: Option<String>,
    /// Path to a `yt-dlp` binary used solely to obtain playback stream URLs.
    ytdlp_bin: Option<std::path::PathBuf>,
    /// Optional bgutil HTTP provider used by yt-dlp for video-bound PO tokens.
    ytdlp_po_provider_url: Option<String>,
    public_url: String,
    websub_callback_url: Option<String>,
    websub_secret: String,
    proxy_targets: Arc<tokio::sync::RwLock<HashMap<String, ProxyTarget>>>,
    proxy_counter: Arc<AtomicU64>,
    /// Resolved playback, keyed by video id. See [`AppServerState::playback_session`].
    playback_sessions: Arc<std::sync::Mutex<HashMap<String, CachedPlayback>>>,
    sync_lock: Arc<tokio::sync::Mutex<()>>,
    search_lock: Arc<tokio::sync::Mutex<()>>,
    /// A self-hosted SponsorBlock mirror, when one is configured.
    sponsor_api_url: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct ProxyTarget {
    url: String,
    request_headers: Vec<crate::models::PlaybackRequestHeader>,
    expires_at: std::time::Instant,
}

/// One video's playback resolve, shared by everyone who asks for it.
///
/// The session is held *before* it is proxied: proxy registrations are minted
/// per hand-out, so a session taken from here gets their full lifetime however
/// long it sat waiting.
#[derive(Clone)]
struct CachedPlayback {
    started_at: std::time::Instant,
    resolve: SharedPlaybackResolve,
}

type SharedPlaybackResolve = futures_util::future::Shared<
    futures_util::future::BoxFuture<'static, Result<ResolvedPlayback, PlaybackFailure>>,
>;

#[derive(Clone)]
pub(crate) struct ResolvedPlayback {
    session: PlaybackSession,
    /// Whether this is worth handing out again. Only a yt-dlp source is:
    /// without one the session is either the embed or extractor URLs that stop
    /// serving part way through, and a later ask deserves a fresh attempt.
    reusable: bool,
}

/// How long a resolved session is handed out again.
///
/// Long enough that the video queued behind an ordinary upload is still warm
/// when it comes up. The bound is the googlevideo URLs, which expire after
/// roughly six hours; proxy registrations are minted per hand-out and do not
/// count against it.
const PLAYBACK_SESSION_TTL: Duration = Duration::from_secs(60 * 60);
/// Plenty for every client's current and next video.
const PLAYBACK_SESSION_LIMIT: usize = 64;

const FEED_RECONCILE_BATCH_SIZE: usize = 48;
const FEED_RSS_CONCURRENCY: usize = 32;
/// How many channels one refresh will ask for video lengths, at most.
///
/// One call covers a channel's recent uploads, so this is the number of extra
/// Innertube requests a pull-to-refresh can cost. The subscription feed has no
/// runtimes, so this covers enough of the recent backlog for duration filters
/// to work before those videos are opened.
const FEED_DURATION_BACKFILL_CHANNELS: usize = 24;
/// How many rows to look at when deciding which channels those are.
const FEED_DURATION_SCAN_ROWS: usize = 600;
/// Exact player lookups are reserved for the cards a viewer can actually see.
/// One page is 24 cards; the API cap allows a loaded second page in one call
/// without turning the endpoint into an unbounded extractor proxy.
const FEED_DURATION_VISIBLE_BATCH: usize = 24;
/// Hard cap on one on-screen hydration request, from any page. Mirrored by the
/// client's look-ahead so a duration filter can ask about a page's worth of
/// cards plus the ones that will not land in the selected bucket.
const DURATION_HYDRATION_LIMIT: usize = 48;
const WEBSUB_RENEW_CONCURRENCY: usize = 12;
/// Channel pages requested at once while backfilling avatars and subscriber
/// counts. Each is a full channel extraction, so this stays well below the
/// WebSub figure, which is a single small POST each.
const CHANNEL_METADATA_CONCURRENCY: usize = 4;
/// Channel pages one backfill pass may request.
const CHANNEL_METADATA_BATCH: usize = 100;
const SEARCH_CACHE_TTL: Duration = Duration::from_secs(5 * 60);
/// Search results by query and filter. Shared by every account: the results
/// are YouTube's, and follow state is applied per viewer on the way out.
static SEARCHES: ServerCache<(String, String), SearchResults> =
    ServerCache::new(SEARCH_CACHE_TTL, 2_000);

fn reconciliation_limit(total: usize, has_accelerated_source: bool) -> usize {
    if has_accelerated_source {
        total.min(FEED_RECONCILE_BATCH_SIZE)
    } else {
        total
    }
}

impl dioxus::fullstack::axum_core::extract::FromRef<FullstackContext> for AppServerState {
    fn from_ref(state: &FullstackContext) -> Self {
        state
            .extension::<AppServerState>()
            .expect("Tawny server state")
    }
}

#[derive(Debug, Deserialize, SurrealValue)]
struct DbChannel {
    channel_id: String,
    name: String,
    handle: String,
    avatar_url: Option<String>,
    subscriber_count: String,
    subscribed: bool,
    /// Optional because rows written before this column existed read back as
    /// NONE, and `SurrealValue` does not honour `#[serde(default)]`. The
    /// startup backfill fills them in, but a row can still be read
    /// during the same startup that adds the column.
    subscription_content: Option<String>,
    description: String,
    banner_url: Option<String>,
}

/// One account's opinion of one channel.
#[derive(Debug, Deserialize, SurrealValue)]
struct DbUserSubscription {
    channel_id: String,
    subscribed: bool,
    /// Optional for the same reason as `DbChannel::subscription_content`.
    content: Option<String>,
}

/// One account's place in one video.
#[derive(Debug, Deserialize, SurrealValue)]
struct DbVideoProgress {
    video_id: String,
    watched: bool,
    progress_seconds: i64,
    audio_only: bool,
}

#[derive(Debug, Deserialize, SurrealValue)]
struct DbFollowerCount {
    count: i64,
}

#[derive(Debug, Deserialize, SurrealValue)]
struct DbSubscriptionFlag {
    subscribed: bool,
}

#[derive(Debug, Deserialize, SurrealValue)]
struct DbSubscriptionState {
    channel_id: String,
    subscribed: bool,
}

#[derive(Debug, Deserialize, SurrealValue)]
struct DbVideo {
    video_id: String,
    title: String,
    channel_id: String,
    channel_name: String,
    thumbnail_url: String,
    published_at: String,
    #[allow(dead_code)]
    published_sort: String,
    duration_seconds: i64,
    view_count: String,
    is_live: bool,
    is_short: bool,
    progress_seconds: i64,
    watched: bool,
    /// Optional for the same reason as `subscription_content`: rows written
    /// before the column existed read back as NONE, and `SurrealValue` does not
    /// honour `#[serde(default)]`.
    audio_only: Option<bool>,
}

#[derive(Debug, Deserialize, SurrealValue)]
struct DbPlaylist {
    playlist_id: String,
    name: String,
    video_ids: Vec<String>,
    /// Optional like the other later columns: a playlist never arranged has
    /// none, and `SurrealValue` does not honour `#[serde(default)]`.
    sort_order: Option<i64>,
}

/// The owner's own order first, then the playlists they never arranged by name.
fn sort_playlists(playlists: &mut [DbPlaylist]) {
    playlists.sort_by(|a, b| {
        let rank = |playlist: &DbPlaylist| playlist.sort_order.unwrap_or(i64::MAX);
        rank(a).cmp(&rank(b)).then_with(|| a.name.cmp(&b.name))
    });
}

#[derive(Debug, Deserialize, SurrealValue)]
struct DbSubscriptionGroup {
    group_id: String,
    name: String,
    channel_ids: Vec<String>,
}

#[derive(Debug, Deserialize, SurrealValue)]
struct DbVideoDetailCache {
    payload_json: String,
}

impl From<DbChannel> for Channel {
    fn from(value: DbChannel) -> Self {
        Self {
            id: value.channel_id,
            name: value.name,
            handle: value.handle,
            avatar_url: value.avatar_url,
            subscriber_count: value.subscriber_count,
            subscribed: value.subscribed,
            description: value.description,
            banner_url: value.banner_url,
            subscription_content: SubscriptionContent::from_storage(
                value.subscription_content.as_deref().unwrap_or_default(),
            ),
        }
    }
}

impl From<DbVideo> for Video {
    fn from(value: DbVideo) -> Self {
        Self {
            id: value.video_id,
            title: value.title,
            channel_id: value.channel_id,
            channel_name: value.channel_name,
            thumbnail_url: value.thumbnail_url,
            published_at: value.published_at,
            duration_seconds: value.duration_seconds.max(0) as u64,
            view_count: value.view_count,
            progress_seconds: value.progress_seconds.max(0) as u64,
            watched: value.watched,
            is_live: value.is_live,
            is_short: value.is_short,
            audio_only: value.audio_only.unwrap_or(false),
            channel_avatar_url: None,
        }
    }
}

impl From<DbPlaylist> for Playlist {
    fn from(value: DbPlaylist) -> Self {
        Self {
            id: value.playlist_id,
            name: value.name,
            video_ids: value.video_ids,
        }
    }
}

impl From<DbSubscriptionGroup> for SubscriptionGroup {
    fn from(value: DbSubscriptionGroup) -> Self {
        Self {
            id: value.group_id,
            name: value.name,
            channel_ids: value.channel_ids,
        }
    }
}

fn default_data_dir() -> std::path::PathBuf {
    #[cfg(test)]
    return std::env::temp_dir().join(format!("tawny-test-{}", std::process::id()));

    #[cfg(not(test))]
    if let Ok(path) = std::env::var("TAWNY_DATA_DIR") {
        return path.into();
    }
    #[cfg(all(not(test), target_os = "windows"))]
    if let Ok(path) = std::env::var("LOCALAPPDATA") {
        return std::path::PathBuf::from(path).join("Tawny");
    }
    #[cfg(all(not(test), target_os = "macos"))]
    if let Ok(path) = std::env::var("HOME") {
        return std::path::PathBuf::from(path)
            .join("Library")
            .join("Application Support")
            .join("Tawny");
    }
    #[cfg(not(test))]
    if let Ok(path) = std::env::var("XDG_DATA_HOME") {
        return std::path::PathBuf::from(path).join("tawny");
    }
    #[cfg(not(test))]
    if let Ok(path) = std::env::var("HOME") {
        return std::path::PathBuf::from(path)
            .join(".local")
            .join("share")
            .join("tawny");
    }
    #[cfg(not(test))]
    std::env::temp_dir().join("tawny")
}

impl AppServerState {
    pub async fn initialize() -> Result<Self> {
        let data_dir = default_data_dir();
        std::fs::create_dir_all(&data_dir).context("create Tawny data directory")?;
        let endpoint = if cfg!(test) {
            "mem://".into()
        } else {
            std::env::var("SURREALDB_HOST").context(
                "SURREALDB_HOST is required; start the Compose dependencies or provide a SurrealDB endpoint",
            )?
        };
        let db = connect(endpoint.clone())
            .await
            .context("connect to SurrealDB")?;
        if endpoint != "mem://"
            && let (Ok(username), Ok(password)) = (
                std::env::var("SURREALDB_USER"),
                std::env::var("SURREALDB_PASSWORD"),
            )
        {
            db.signin(Root { username, password })
                .await
                .context("sign in to SurrealDB")?;
        }
        db.use_ns(std::env::var("SURREALDB_NAMESPACE").unwrap_or_else(|_| "tawny".into()))
            .use_db(std::env::var("SURREALDB_NAME").unwrap_or_else(|_| "main".into()))
            .await
            .context("select SurrealDB namespace and database")?;
        crate::db::sync_schema(&db).await?;

        let extractor_cache = std::env::var("TAWNY_EXTRACTOR_CACHE_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| data_dir.join("extractor"));
        let mut youtube_builder = RustyPipe::builder()
            .storage_dir(extractor_cache)
            .po_token_cache();
        // Only honoured when the path actually exists. It is an optional
        // helper, so a stale or placeholder value should cost the feature, not
        // the whole server — the extractor refuses to build at all when handed
        // a binary it cannot find.
        if let Ok(botguard_bin) = std::env::var("TAWNY_BOTGUARD_BIN") {
            if std::path::Path::new(&botguard_bin).is_file() {
                youtube_builder = youtube_builder.botguard_bin(botguard_bin);
            } else {
                eprintln!("ignoring TAWNY_BOTGUARD_BIN: no binary at {botguard_bin}");
            }
        }
        let youtube = youtube_builder
            .build()
            .context("initialize direct YouTube extractor")?;
        let public_url = std::env::var("TAWNY_PUBLIC_URL")
            // The Android bundle's generated network-security policy permits
            // cleartext loopback by literal IP. `localhost` is a different
            // policy entry even though it resolves to the same interface.
            .unwrap_or_else(|_| "http://127.0.0.1:8080".into())
            .trim_end_matches('/')
            .to_string();
        let websub_callback_url = std::env::var("TAWNY_WEBSUB_CALLBACK_URL")
            .ok()
            .filter(|url| !url.trim().is_empty())
            .or_else(|| {
                (!public_url.contains("localhost") && !public_url.contains("127.0.0.1"))
                    .then(|| format!("{public_url}/api/v1/websub/youtube"))
            });
        let websub_secret_path = data_dir.join("websub-secret");
        let websub_secret = std::env::var("TAWNY_WEBSUB_SECRET")
            .ok()
            .map(|secret| secret.trim().to_string())
            .filter(|secret| !secret.is_empty())
            .unwrap_or_else(|| {
                std::fs::read_to_string(&websub_secret_path)
                    .ok()
                    .map(|secret| secret.trim().to_string())
                    .filter(|secret| !secret.is_empty())
                    .unwrap_or_else(|| {
                        let secret = hex::encode(rand::random::<[u8; 32]>());
                        let _ = std::fs::write(&websub_secret_path, &secret);
                        secret
                    })
            });

        let ytdlp_service_url = configured_ytdlp_service_url();
        let ytdlp_bin = ytdlp_service_url.is_none().then(locate_ytdlp).flatten();
        let state = Self {
            db: Arc::new(db),
            http: reqwest::Client::builder()
                .user_agent("Tawny/0.1 subscription updater")
                .timeout(Duration::from_secs(15))
                .build()?,
            media_http: reqwest::Client::builder()
                .user_agent("Tawny/0.1 media proxy")
                // A media fetch that never returns is worse than one that
                // fails: the browser request hangs open, MediaSource simply
                // starves, and playback freezes with no error for the player
                // to recover from. Bound connect and per-chunk read time
                // instead of the whole request, so long streaming bodies stay
                // legal while a wedged upstream surfaces quickly.
                .connect_timeout(Duration::from_secs(10))
                .read_timeout(Duration::from_secs(20))
                .build()?,
            ytdlp_http: reqwest::Client::builder()
                .user_agent("Tawny/0.1 yt-dlp sidecar client")
                .connect_timeout(Duration::from_secs(3))
                // Longer than the extractor's queue for a free slot plus its own
                // extraction limit, so a video that waited its turn is not given
                // up on from this side.
                .timeout(Duration::from_secs(60))
                .build()?,
            youtube: Arc::new(youtube),
            ytdlp_service_url,
            ytdlp_bin,
            ytdlp_po_provider_url: configured_ytdlp_po_provider_url(),
            public_url,
            websub_callback_url,
            websub_secret,
            proxy_targets: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            proxy_counter: Arc::new(AtomicU64::new(1)),
            playback_sessions: Arc::new(std::sync::Mutex::new(HashMap::new())),
            sync_lock: Arc::new(tokio::sync::Mutex::new(())),
            search_lock: Arc::new(tokio::sync::Mutex::new(())),
            sponsor_api_url: std::env::var("TAWNY_SPONSORBLOCK_URL")
                .ok()
                .map(|url| url.trim_end_matches('/').to_string())
                .filter(|url| !url.is_empty()),
        };
        state.seed_demo_if_empty().await?;
        Ok(state)
    }

    /// Account operations against this instance's database.
    ///
    /// Borrowed per call rather than stored: it holds nothing but the handle,
    /// and keeping the account code behind one entry point is what stops it
    /// spreading through the catalog queries below.
    pub fn accounts(&self) -> crate::auth::Accounts<'_> {
        crate::auth::Accounts::new(&self.db)
    }

    /// Per-user rows are keyed by a record id built from the owner, so two
    /// accounts can hold the same client-chosen id - every library starts with
    /// a `watch-later`, and without this the second account to save one would
    /// overwrite the first.
    fn owner_key(owner: &str, id: &str) -> String {
        // `|` cannot appear in an app_user id, so this cannot be made ambiguous
        // by a playlist name that happens to contain the separator.
        format!("{}|{}", owner.trim_start_matches("app_user:"), id)
    }

    /// What this account follows, by channel id.
    async fn user_subscriptions(
        &self,
        owner: &str,
    ) -> Result<HashMap<String, (bool, SubscriptionContent)>> {
        if owner.is_empty() {
            return Ok(HashMap::new());
        }
        let rows: Vec<DbUserSubscription> = self
            .db
            .query(
                "SELECT channel_id, subscribed, content FROM user_subscription WHERE owner = type::record($owner)",
            )
            .bind(("owner", owner.to_string()))
            .await?
            .check()?
            .take(0)?;
        Ok(rows
            .into_iter()
            .map(|row| {
                let content =
                    SubscriptionContent::from_storage(row.content.as_deref().unwrap_or("all"));
                (row.channel_id, (row.subscribed, content))
            })
            .collect())
    }

    /// Whether this account follows one channel, and which uploads it wants.
    ///
    /// Anything shown as a Subscribe button asks this, not `channel.subscribed`:
    /// that column means someone on the instance follows the channel - see
    /// `refresh_channel_interest` - so it reads as subscribed to an account
    /// that is not, and the other way round for details cached by another one.
    async fn user_follows(
        &self,
        owner: &str,
        channel_id: &str,
    ) -> Result<Option<(bool, SubscriptionContent)>> {
        if owner.is_empty() {
            return Ok(None);
        }
        let rows: Vec<DbUserSubscription> = self
            .db
            .query(
                "SELECT channel_id, subscribed, content FROM user_subscription WHERE owner = type::record($owner) AND channel_id = $channel_id LIMIT 1",
            )
            .bind(("owner", owner.to_string()))
            .bind(("channel_id", channel_id.to_string()))
            .await?
            .check()?
            .take(0)?;
        Ok(rows.into_iter().next().map(|row| {
            let content =
                SubscriptionContent::from_storage(row.content.as_deref().unwrap_or("all"));
            (row.subscribed, content)
        }))
    }

    /// Details with the channel's follow state set for `owner`.
    ///
    /// The details cache is shared by every account and keyed by video alone,
    /// so whatever follow state it holds belongs to whoever fetched it first -
    /// and until now it was served as-is for two hours, and indefinitely as the
    /// stale fallback. Applied to every copy on the way out, cached or fresh.
    async fn with_owner_follow_state(
        &self,
        owner: &str,
        mut details: VideoDetails,
    ) -> Result<VideoDetails> {
        if let Some(channel) = details.channel.as_mut() {
            let (subscribed, content) = self
                .user_follows(owner, &channel.id)
                .await?
                .unwrap_or((false, channel.subscription_content));
            channel.subscribed = subscribed;
            channel.subscription_content = content;
        }
        Ok(details)
    }

    /// Search results with every channel's follow state set for `owner`.
    ///
    /// Search is cached by query alone, and the channels in it carried the
    /// instance-wide flag, which is not anyone's own. That mattered beyond the
    /// button: the client adds a channel it has not seen to the library as it
    /// arrives, flag included, so a wrong "subscribed" here became a follow.
    async fn with_owner_follows_in_search(
        &self,
        owner: &str,
        mut results: SearchResults,
    ) -> Result<SearchResults> {
        if results.channels.is_empty() {
            return Ok(results);
        }
        let follows = self.user_subscriptions(owner).await?;
        for channel in &mut results.channels {
            match follows.get(&channel.id) {
                Some((subscribed, content)) => {
                    channel.subscribed = *subscribed;
                    channel.subscription_content = *content;
                }
                None => channel.subscribed = false,
            }
        }
        Ok(results)
    }

    /// Recompute `channel.subscribed` from the per-user table.
    ///
    /// That column is no longer anyone's opinion - it means "someone on this
    /// instance follows this channel", which is the question the poller and the
    /// WebSub renewals ask. It must never be written directly: one account
    /// unsubscribing would otherwise stop the feed for everyone else who still
    /// follows it.
    async fn refresh_channel_interest(&self, channel_ids: &[String]) -> Result<()> {
        for channel_id in channel_ids {
            let counts: Vec<DbFollowerCount> = self
                .db
                .query(
                    "SELECT count() FROM user_subscription WHERE channel_id = $channel_id AND subscribed = true GROUP ALL",
                )
                .bind(("channel_id", channel_id.clone()))
                .await?
                .check()?
                .take(0)?;
            let followed = counts.first().map(|row| row.count > 0).unwrap_or(false);
            self.db
                .query("UPDATE channel SET subscribed = $followed WHERE channel_id = $channel_id")
                .bind(("channel_id", channel_id.clone()))
                .bind(("followed", followed))
                .await?
                .check()?;
        }
        Ok(())
    }

    async fn reachable_ytdlp_po_provider_url(&self) -> Option<&str> {
        let provider_url = self.ytdlp_po_provider_url.as_deref()?;
        let ping_url = format!("{provider_url}/ping");
        match tokio::time::timeout(Duration::from_secs(2), self.http.get(&ping_url).send()).await {
            Ok(Ok(response)) if response.status().is_success() => Some(provider_url),
            Ok(Ok(response)) => {
                eprintln!(
                    "PO-token provider at {provider_url} returned {} from /ping; using yt-dlp's default client",
                    response.status()
                );
                None
            }
            Ok(Err(error)) => {
                eprintln!(
                    "PO-token provider at {provider_url} is unavailable ({error}); using yt-dlp's default client"
                );
                None
            }
            Err(_) => {
                eprintln!(
                    "PO-token provider at {provider_url} did not answer /ping; using yt-dlp's default client"
                );
                None
            }
        }
    }

    /// Merge this account's watch progress for the videos it sent.
    ///
    /// A merge, not a replacement: the client sends only videos it has touched,
    /// so rows it did not mention are still this account's history and have to
    /// survive.
    async fn write_user_progress(&self, owner: &str, progress: &[VideoProgress]) -> Result<()> {
        for entry in progress {
            self.db
                .query(
                    r#"UPSERT type::record('video_progress', $record_key) SET
                        owner = type::record($owner),
                        video_id = $video_id,
                        watched = $watched,
                        progress_seconds = $progress_seconds,
                        audio_only = $audio_only,
                        updated_at = time::now()"#,
                )
                .bind(("record_key", Self::owner_key(owner, &entry.video_id)))
                .bind(("owner", owner.to_string()))
                .bind(("video_id", entry.video_id.clone()))
                .bind(("watched", entry.watched))
                .bind(("progress_seconds", entry.progress_seconds as i64))
                .bind(("audio_only", entry.audio_only))
                .await?
                .check()?;
        }
        Ok(())
    }

    /// Hand newly changed channels to the WebSub and feed machinery.
    ///
    /// The channels carry *this* account's subscribed flag, but the work they
    /// trigger is instance-wide, so the decision has to be re-read from the
    /// derived column rather than taken from the caller: one viewer
    /// unsubscribing must not cancel a WebSub lease that other viewers still
    /// depend on.
    fn spawn_subscription_changes(&self, changed: Vec<Channel>) {
        if changed.is_empty() {
            return;
        }
        let state = self.clone();
        tokio::spawn(async move {
            let ids = changed
                .iter()
                .map(|channel| channel.id.clone())
                .collect::<Vec<_>>();
            let interest: Vec<DbChannel> = match state
                .db
                .query("SELECT channel_id, name, handle, avatar_url, subscriber_count, subscribed, subscription_content, description, banner_url FROM channel WHERE channel_id IN $ids")
                .bind(("ids", ids))
                .await
                .and_then(|mut response| response.take(0))
            {
                Ok(rows) => rows,
                Err(error) => {
                    eprintln!("could not re-read channel interest: {error}");
                    return;
                }
            };
            let channels = interest.into_iter().map(Channel::from).collect::<Vec<_>>();
            if !channels.is_empty() {
                state.process_subscription_changes(channels).await;
            }
        });
    }

    async fn process_subscription_changes(&self, changed_channels: Vec<Channel>) {
        let newly_subscribed = changed_channels
            .iter()
            .filter(|channel| channel.subscribed)
            .cloned()
            .collect::<Vec<_>>();

        use futures_util::{StreamExt, stream};
        let websub_job = async {
            stream::iter(changed_channels.iter().cloned().map(|channel| async move {
                self.request_websub_subscription(&channel.id, channel.subscribed)
                    .await
            }))
            .buffer_unordered(WEBSUB_RENEW_CONCURRENCY)
            .collect::<Vec<_>>()
            .await
        };
        let feed_job =
            async {
                if newly_subscribed.len() <= 3 {
                    let _ =
                        stream::iter(newly_subscribed.iter().cloned().map(|channel| async move {
                            self.refresh_channel_local(channel).await
                        }))
                        .buffer_unordered(3)
                        .collect::<Vec<_>>()
                        .await;
                } else {
                    let ids = newly_subscribed
                        .iter()
                        .map(|channel| channel.id.clone())
                        .collect::<Vec<_>>();
                    let limit = reconciliation_limit(
                        newly_subscribed.len(),
                        self.websub_callback_url.is_some(),
                    );
                    let _ = self
                        .reconcile_channels_rss(newly_subscribed.into_iter().take(limit).collect())
                        .await;
                    // RSS carries no avatar or subscriber count, and a bulk
                    // import is the one path that never reads a channel page.
                    self.backfill_channel_metadata(Some(&ids), CHANNEL_METADATA_BATCH)
                        .await;
                }
            };
        let _ = tokio::join!(websub_job, feed_job);
    }
}

#[cfg(test)]
mod tests {
    use crate::models::{playlist_queue_entry, queued_playlist_id};

    use super::DbVideo;

    #[test]
    fn flat_channel_listings_sort_the_whole_catalog_with_real_counts() {
        let dump =
            serde_json::from_value::<super::YtdlpFlatPlaylist>(serde_json::json!({"entries": [
                {"id": "new", "title": "Newest", "timestamp": 1791820800, "view_count": 27000},
                {"id": "hit", "title": "Popular", "timestamp": 1789430400, "view_count": 1100000},
                {"id": "unknown", "title": "Unknown"},
                {"id": "old", "title": "Oldest", "view_count": 88}
            ]}))
            .unwrap();
        let videos = super::ytdlp_flat_playlist_videos(dump, "channel", "Channel");
        assert_eq!(videos[0].published_at, "2026-10-12");
        let sorted = |order| {
            let mut copy = videos.clone();
            super::sort_channel_uploads(&mut copy, order);
            copy.into_iter().map(|video| video.id).collect::<Vec<_>>()
        };
        assert_eq!(sorted("newest"), ["new", "hit", "unknown", "old"]);
        assert_eq!(sorted("oldest"), ["old", "unknown", "hit", "new"]);
        assert_eq!(sorted("popular"), ["hit", "new", "old", "unknown"]);
        assert_eq!(sorted("unpopular"), ["old", "new", "hit", "unknown"]);
    }

    #[tokio::test]
    async fn verified_subscribers_replace_header_counts_and_hidden_counts() {
        let state = super::AppServerState::initialize().await.unwrap();
        let mut channel = crate::models::DemoLibrary::demo().channels[0].clone();
        channel.subscriber_count = "88".into();
        state.upsert_channel(&channel).await.unwrap();
        state
            .store_subscriber_count(&channel.id, Some(482_000))
            .await
            .unwrap();
        assert_eq!(
            state
                .catalog_channel(&channel.id)
                .await
                .unwrap()
                .unwrap()
                .subscriber_count,
            "482.0K"
        );
        // A verified smaller or hidden count must also replace a cached count.
        state
            .store_subscriber_count(&channel.id, Some(80))
            .await
            .unwrap();
        assert_eq!(
            state
                .catalog_channel(&channel.id)
                .await
                .unwrap()
                .unwrap()
                .subscriber_count,
            "80"
        );
        state
            .store_subscriber_count(&channel.id, None)
            .await
            .unwrap();
        assert_eq!(
            state
                .catalog_channel(&channel.id)
                .await
                .unwrap()
                .unwrap()
                .subscriber_count,
            ""
        );
    }
    use super::{
        AppServerState, SegmentRangeKey, SegmentRanges, cached_segment_ranges, canonical_sort_key,
        center_vtt_cues, classify_ytdlp_error, ebml_vint, extract_chapters,
        extractor_status_failure, mp4_segment_ranges, no_stream_failure, parse_youtube_feed,
        playback_proxy, playback_proxy_options, reconciliation_limit, requested_byte_range,
        segment_ranges, sniff_media_type, store_segment_ranges, unambiguous_segment_ranges,
        url_path_ends_with, video_published_epoch, webm_segment_ranges, websub_channel_from_topic,
        ytdlp_hls_source, ytdlp_po_provider_args, ytdlp_service_channel_shorts_url,
        ytdlp_service_video_url,
    };
    use crate::models::PlaybackProtocol;
    use crate::models::Video;
    use crate::models::{PlaybackFailure, PlaybackFailureOrigin};
    use axum::extract::Path;
    use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
    use std::time::Duration;

    #[test]
    fn parses_youtube_atom_entries() {
        let xml = r#"<feed xmlns:yt="http://www.youtube.com/xml/schemas/2015"><entry><yt:videoId>abc123</yt:videoId><yt:channelId>UC-real-channel</yt:channelId><title>Fresh upload</title><published>2026-08-10T12:00:00Z</published></entry></feed>"#;
        let entries = parse_youtube_feed(xml).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].video_id, "abc123");
        assert_eq!(entries[0].channel_id, "UC-real-channel");
        assert_eq!(entries[0].title, "Fresh upload");
    }

    #[test]
    fn validates_youtube_websub_topics() {
        let valid = "https://www.youtube.com/xml/feeds/videos.xml?channel_id=UC-real-channel";
        assert_eq!(
            websub_channel_from_topic(valid).as_deref(),
            Some("UC-real-channel")
        );
        assert!(websub_channel_from_topic("https://example.com/?channel_id=UC-bad").is_none());
    }

    /// A fresh account to own whatever the test writes.
    ///
    /// Library calls now refuse an unauthenticated caller outright, so every
    /// test needs one - and a test getting its own means one test's
    /// subscriptions can no longer surface in another's snapshot.
    async fn test_owner(state: &AppServerState) -> String {
        state
            .accounts()
            .create_guest()
            .await
            .expect("mint a guest for the test")
            .id
    }

    /// The seeded demo channels, as the catalog holds them.
    fn demo_channels() -> Vec<crate::models::Channel> {
        crate::models::DemoLibrary::demo().channels
    }

    /// Set whether `owner` follows `channel`.
    async fn set_follow(
        state: &AppServerState,
        owner: &str,
        channel: &crate::models::Channel,
        subscribed: bool,
    ) {
        state
            .set_subscriptions(
                owner,
                vec![crate::models::SubscriptionChange {
                    channel: channel.clone(),
                    subscribed,
                    content: crate::models::SubscriptionContent::All,
                }],
            )
            .await
            .expect("set a follow for the test");
    }

    #[tokio::test]
    async fn permits_android_webview_media_requests() {
        let response = playback_proxy_options().await;
        assert_eq!(response.status(), axum::http::StatusCode::NO_CONTENT);
        assert_eq!(response.headers()["access-control-allow-origin"], "*");
        assert_eq!(
            response.headers()["access-control-allow-private-network"],
            "true"
        );
        assert!(
            response.headers()["access-control-allow-headers"]
                .to_str()
                .unwrap()
                .contains("Range")
        );
    }

    #[tokio::test]
    async fn dubbed_audio_tracks_do_not_share_segment_ranges() {
        // One itag, two languages, two files with different headers.
        let original = SegmentRanges {
            init_end: 258,
            index_start: 259,
            index_end: 4037,
        };
        let dub = SegmentRanges {
            init_end: 300,
            index_start: 301,
            index_end: 4100,
        };
        let key = |language: &str, size: u64| SegmentRangeKey {
            video_id: "dubbed-cache-test".into(),
            itag: 140,
            language: Some(language.into()),
            size,
        };
        // XKSjCOKDtpk's Japanese and Turkish itag 140 are the same length.
        store_segment_ranges(key("ja", 34_831_565), original).await;
        store_segment_ranges(key("tr", 34_831_565), dub).await;
        assert_eq!(
            cached_segment_ranges(&key("ja", 34_831_565)).await,
            Some(original)
        );
        assert_eq!(
            cached_segment_ranges(&key("tr", 34_831_565)).await,
            Some(dub)
        );
        assert_eq!(cached_segment_ranges(&key("en", 34_830_834)).await, None);
    }

    #[test]
    fn reads_the_requested_byte_range() {
        let mut headers = axum::http::HeaderMap::new();
        assert_eq!(requested_byte_range(&headers), None);
        headers.insert(header::RANGE, HeaderValue::from_static("bytes=100-199"));
        assert_eq!(requested_byte_range(&headers), Some((100, Some(199))));
        headers.insert(header::RANGE, HeaderValue::from_static("bytes=100-"));
        assert_eq!(requested_byte_range(&headers), Some((100, None)));
        headers.insert(header::RANGE, HeaderValue::from_static("bytes=0-1,5-9"));
        assert_eq!(requested_byte_range(&headers), None);
    }

    /// An upstream that drops the first ranged response halfway through, then
    /// serves whatever range it is asked for next. Returns the ranges it saw.
    async fn flaky_range_upstream(
        media: Vec<u8>,
    ) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = seen.clone();
        tokio::spawn(async move {
            let mut served = 0;
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                let mut request = Vec::new();
                let mut buffer = [0u8; 1024];
                while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                    let read = socket.read(&mut buffer).await.unwrap();
                    if read == 0 {
                        break;
                    }
                    request.extend_from_slice(&buffer[..read]);
                }
                let request = String::from_utf8_lossy(&request).to_ascii_lowercase();
                let range = request
                    .lines()
                    .find_map(|line| line.strip_prefix("range: bytes="))
                    .unwrap()
                    .trim()
                    .to_string();
                log.lock().unwrap().push(range.clone());
                let (start, end) = range.split_once('-').unwrap();
                let (start, end): (usize, usize) = (start.parse().unwrap(), end.parse().unwrap());
                let body = &media[start..=end];
                let head = format!(
                    "HTTP/1.1 206 Partial Content\r\ncontent-type: application/octet-stream\r\ncontent-length: {}\r\ncontent-range: bytes {start}-{end}/{}\r\nconnection: close\r\n\r\n",
                    body.len(),
                    media.len()
                );
                socket.write_all(head.as_bytes()).await.unwrap();
                let cut = if served == 0 {
                    body.len() / 2
                } else {
                    body.len()
                };
                socket.write_all(&body[..cut]).await.unwrap();
                socket.flush().await.unwrap();
                served += 1;
            }
        });
        (format!("http://{address}/media"), seen)
    }

    #[tokio::test]
    async fn resumes_a_range_the_upstream_drops_midway() {
        let state = AppServerState::initialize().await.unwrap();
        let media = (0..200_000u32)
            .map(|index| (index % 251) as u8)
            .collect::<Vec<_>>();
        let (upstream, seen) = flaky_range_upstream(media.clone()).await;
        let proxied = state
            .register_proxy_target(upstream, Vec::new(), Duration::from_secs(60))
            .await;
        let token = proxied.rsplit('/').next().unwrap().to_string();
        let mut headers = HeaderMap::new();
        headers.insert(header::RANGE, HeaderValue::from_static("bytes=1000-150999"));

        let response = playback_proxy(axum::Extension(state), Path(token), headers).await;
        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(response.headers()[header::CONTENT_LENGTH], "150000");
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("the resumed body completes");
        assert_eq!(&body[..], &media[1000..=150_999]);
        let seen = seen.lock().unwrap().clone();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0], "1000-150999");
        let resumed_from: usize = seen[1].split_once('-').unwrap().0.parse().unwrap();
        assert!(resumed_from > 1000 && resumed_from <= 76_000, "{seen:?}");
    }
    #[test]
    fn bounds_local_reconciliation_when_push_is_active() {
        assert_eq!(reconciliation_limit(700, true), 48);
        assert_eq!(reconciliation_limit(700, false), 700);
        assert_eq!(reconciliation_limit(12, true), 12);
    }

    #[test]
    fn configures_ytdlp_for_video_bound_po_tokens() {
        assert!(ytdlp_po_provider_args(None).is_empty());
        assert_eq!(
            ytdlp_po_provider_args(Some("http://127.0.0.1:4416")),
            vec![
                "--extractor-args",
                "youtubepot-bgutilhttp:base_url=http://127.0.0.1:4416",
                "--extractor-args",
                "youtube:player_client=mweb",
            ]
        );
    }

    #[test]
    fn builds_encoded_ytdlp_sidecar_video_urls() {
        let url = ytdlp_service_video_url("http://yt-dlp:8080/base", "aNXB-8Aqt88").unwrap();
        assert_eq!(
            url.as_str(),
            "http://yt-dlp:8080/base/v1/videos/aNXB-8Aqt88"
        );
        assert!(ytdlp_service_video_url("not a URL", "aNXB-8Aqt88").is_none());
    }

    #[test]
    fn falls_back_to_ytdlp_hls_master_playlist() {
        let dump = serde_json::json!({
            "duration": 60.0,
            "formats": [
                {"format_id": "sb0", "protocol": "mhtml", "url": "https://i.ytimg.com/sb"},
                {
                    "format_id": "232",
                    "protocol": "m3u8_native",
                    "url": "https://manifest.googlevideo.com/api/manifest/hls_playlist/itag/232/index.m3u8",
                    "manifest_url": "https://manifest.googlevideo.com/api/manifest/hls_variant/file/index.m3u8"
                }
            ]
        });
        let formats = super::formats_from_ytdlp_dump(serde_json::from_value(dump).unwrap());
        let source = ytdlp_hls_source(&formats).expect("an HLS source");
        assert_eq!(source.protocol, PlaybackProtocol::Hls);
        assert_eq!(
            source.url,
            "https://manifest.googlevideo.com/api/manifest/hls_variant/file/index.m3u8"
        );

        let dash_only = serde_json::json!({
            "formats": [{"format_id": "140", "protocol": "https", "url": "https://rr1.googlevideo.com/videoplayback"}]
        });
        let formats = super::formats_from_ytdlp_dump(serde_json::from_value(dash_only).unwrap());
        assert!(ytdlp_hls_source(&formats).is_none());
    }

    #[test]
    fn builds_ytdlp_sidecar_channel_shorts_urls() {
        let url =
            ytdlp_service_channel_shorts_url("http://yt-dlp:8080/base", "UCsXVk37bltHxD1rDPwtNM8Q")
                .unwrap();
        assert_eq!(
            url.as_str(),
            "http://yt-dlp:8080/base/v1/channels/UCsXVk37bltHxD1rDPwtNM8Q/shorts"
        );
    }

    #[test]
    fn orders_human_and_atom_dates_by_actual_recency() {
        assert!(video_published_epoch("2 hours ago") > video_published_epoch("1 year ago"));
        assert!(
            canonical_sort_key("2026-08-10T12:00:00Z") > canonical_sort_key("2025-08-10T12:00:00Z")
        );
        assert_eq!(canonical_sort_key("unknown"), "00000000000000000000");
    }

    #[test]
    fn extracts_ordered_description_chapters() {
        let chapters =
            extract_chapters("0:00 Opening\n1:05 First idea\n12:34 Second idea\n1:02:03 Closing");
        assert_eq!(chapters.len(), 4);
        assert_eq!(chapters[1].title, "First idea");
        assert_eq!(chapters[1].start_seconds, 65);
        assert_eq!(chapters[3].start_seconds, 3723);
        let bulleted = extract_chapters("• 0:00 Opening\n2. (1:05) First idea\n- 2:10 Closing");
        assert_eq!(bulleted.len(), 3);
        assert_eq!(bulleted[1].title, "First idea");
        assert!(extract_chapters("1:22 One stray timestamp").is_empty());
    }

    #[test]
    fn treats_only_real_playlists_as_playlists() {
        // A YouTube HLS segment carries the playlist path plus /sq/N/, so it
        // contains ".m3u8" without being one. Rewriting it as a playlist
        // replaces the media with proxy URLs and playback never buffers.
        assert!(url_path_ends_with(
            "https://manifest.googlevideo.com/api/manifest/hls_playlist/file/index.m3u8",
            ".m3u8"
        ));
        assert!(!url_path_ends_with(
            "https://rr3---sn-x.googlevideo.com/videoplayback/file/index.m3u8/sq/3/goap/clen/123",
            ".m3u8"
        ));
        assert!(url_path_ends_with(
            "https://example.test/stream/index.m3u8?itag=0",
            ".m3u8"
        ));
        assert!(!url_path_ends_with(
            "https://example.test/seg/index.m3u8/sq/0?itag=0",
            ".m3u8"
        ));
    }

    #[test]
    fn identifies_hls_segment_containers_youtube_serves_as_octet_stream() {
        // Packed AAC, which Shaka otherwise mistakes for fMP4 and never decodes.
        assert_eq!(sniff_media_type(b"ID3\x03\x00\x00"), Some("audio/aac"));
        assert_eq!(
            sniff_media_type(&[0xFF, 0xF1, 0x50, 0x80]),
            Some("audio/aac")
        );
        // MPEG-TS sync byte.
        assert_eq!(
            sniff_media_type(&[0x47, 0x40, 0x00, 0x30]),
            Some("video/mp2t")
        );
        assert_eq!(
            sniff_media_type(b"\x00\x00\x00\x18ftypmp42"),
            Some("video/mp4")
        );
        assert_eq!(
            sniff_media_type(&[0x1A, 0x45, 0xDF, 0xA3]),
            Some("video/webm")
        );
        assert_eq!(sniff_media_type(b"not media at all"), None);
        assert_eq!(sniff_media_type(&[]), None);
    }

    #[test]
    fn centers_caption_cues_without_discarding_other_settings() {
        let vtt = "WEBVTT\n\n00:00:01.000 --> 00:00:03.000 align:start position:0% line:90%\nHello";
        let centered = center_vtt_cues(vtt);
        assert!(
            centered.contains("00:00:01.000 --> 00:00:03.000 line:90% align:center position:50%")
        );
        assert!(!centered.contains("align:start"));
    }

    #[tokio::test]
    async fn serves_video_details_for_a_seeded_video() {
        let state = AppServerState::initialize().await.unwrap();
        let owner = test_owner(&state).await;
        // A demo video whose channel has others, so the offline fallback has
        // related videos to offer.
        let demo = crate::models::DemoLibrary::demo();
        let video_id = demo
            .videos
            .iter()
            .find(|video| {
                demo.videos
                    .iter()
                    .filter(|other| other.channel_id == video.channel_id)
                    .count()
                    > 1
            })
            .expect("a demo channel with two videos")
            .id
            .clone();
        let details = state.video_details(&owner, &video_id).await.unwrap();
        assert_eq!(details.video.id, video_id);
        // Seeded videos use real YouTube ids, so direct extraction may or may
        // not reach YouTube from the machine running the tests. Either way the
        // call has to resolve to that video with something to show next, which
        // is what the offline library fallback guarantees.
        assert!(!details.related_videos.is_empty());
    }

    /// The guard that keeps a listing from overwriting a known length lives in
    /// the statement, so the statement is what the test exercises. A rounded
    /// duration from a channel page replacing the exact one the player recorded
    /// would be a silent regression in every timeline and filter.
    #[tokio::test]
    async fn a_listing_fills_an_unknown_length_but_never_replaces_a_known_one() {
        let state = AppServerState::initialize().await.unwrap();
        let _owner = test_owner(&state).await;
        let video = Video {
            id: "duration-test".into(),
            title: "From the subscription feed".into(),
            channel_id: "UC-real-channel".into(),
            channel_name: "Channel".into(),
            thumbnail_url: String::new(),
            published_at: "2026-08-10T12:00:00Z".into(),
            duration_seconds: 0,
            view_count: String::new(),
            progress_seconds: 0,
            watched: false,
            is_live: false,
            is_short: false,
            audio_only: false,
            channel_avatar_url: None,
        };
        state
            .upsert_feed_hint(&video, video.published_at.clone())
            .await
            .unwrap();

        let filled = state
            .apply_video_durations(&[("duration-test".to_string(), 754, "12K views".to_string())])
            .await
            .unwrap();
        assert_eq!(filled, 1, "an RSS row starts at zero and takes the length");

        let again = state
            .apply_video_durations(&[("duration-test".to_string(), 12, String::new())])
            .await
            .unwrap();
        assert_eq!(again, 0, "a known length is never overwritten");

        let stored: Vec<DbVideo> = state
            .db
            .query("SELECT * FROM video WHERE video_id = $video_id")
            .bind(("video_id", "duration-test"))
            .await
            .unwrap()
            .take(0)
            .unwrap();
        assert_eq!(stored[0].duration_seconds, 754);
        assert_eq!(stored[0].view_count, "12K views");
    }

    /// Channels are chosen by where the gaps actually are. Picking them any
    /// other way spends the refresh budget on channels that are already filled.
    #[tokio::test]
    async fn only_channels_with_unknown_lengths_are_asked_for_them() {
        let state = AppServerState::initialize().await.unwrap();
        let _owner = test_owner(&state).await;
        let known = Video {
            id: "known-length".into(),
            title: "Already measured".into(),
            channel_id: "UC-measured-channel".into(),
            channel_name: "Measured".into(),
            thumbnail_url: String::new(),
            published_at: "2026-08-11T12:00:00Z".into(),
            duration_seconds: 600,
            view_count: String::new(),
            progress_seconds: 0,
            watched: false,
            is_live: false,
            is_short: false,
            audio_only: false,
            channel_avatar_url: None,
        };
        let unknown = Video {
            id: "unknown-length".into(),
            channel_id: "UC-unmeasured-channel".into(),
            channel_name: "Unmeasured".into(),
            ..known.clone()
        };
        state
            .upsert_feed_video(&known, known.published_at.clone())
            .await
            .unwrap();
        state
            .upsert_feed_hint(&unknown, unknown.published_at.clone())
            .await
            .unwrap();

        let channels = state.channels_missing_durations(6).await.unwrap();
        assert!(channels.contains(&"UC-unmeasured-channel".to_string()));
        assert!(!channels.contains(&"UC-measured-channel".to_string()));
    }

    #[tokio::test]
    async fn visible_duration_hydration_is_scoped_to_the_viewers_feed() {
        let state = AppServerState::initialize().await.unwrap();
        let owner = test_owner(&state).await;
        let video = crate::models::DemoLibrary::demo()
            .videos
            .into_iter()
            .find(|video| video.duration_seconds > 0)
            .expect("seeded catalog has a measured video");
        let channel = demo_channels()
            .into_iter()
            .find(|channel| channel.id == video.channel_id)
            .expect("video channel is cached");
        set_follow(&state, &owner, &channel, true).await;

        let resolved = state
            .hydrate_video_durations(
                &owner,
                vec![video.id.clone(), video.id.clone(), "not-cached".into()],
            )
            .await
            .unwrap();
        assert_eq!(resolved.len(), 1, "duplicates and unknown ids are ignored");
        assert_eq!(resolved[0].id, video.id);
        assert!(resolved[0].duration_seconds > 0);

        let other_owner = test_owner(&state).await;
        assert!(
            state
                .hydrate_video_durations(&other_owner, vec![video.id])
                .await
                .unwrap()
                .is_empty(),
            "a video outside the viewer's subscriptions is not hydrated"
        );
    }

    /// A playlist is where a reader collects videos from channels they do not
    /// follow, so a subscription-only allowlist left exactly those rows without
    /// a length - and a duration filter silently drops an unknown length.
    #[tokio::test]
    async fn a_saved_video_is_hydrated_without_following_its_channel() {
        let state = AppServerState::initialize().await.unwrap();
        let owner = test_owner(&state).await;
        let video = crate::models::DemoLibrary::demo()
            .videos
            .into_iter()
            .find(|video| video.duration_seconds > 0)
            .expect("seeded catalog has a measured video");
        // Saved to a playlist, without following the channel.
        state
            .save_playlist(&owner, "saved-from-elsewhere", "Saved")
            .await
            .unwrap();
        state
            .set_in_playlist(&owner, "saved-from-elsewhere", &video.id, true)
            .await
            .unwrap();

        let resolved = state
            .hydrate_video_durations(&owner, vec![video.id.clone()])
            .await
            .unwrap();
        assert_eq!(resolved.len(), 1, "a saved video is hydrated on its own");
        assert_eq!(resolved[0].id, video.id);

        assert!(
            state
                .hydrate_video_durations(&test_owner(&state).await, vec![video.id])
                .await
                .unwrap()
                .is_empty(),
            "another account's playlist grants nothing"
        );
    }

    /// The queue carries one marker entry standing in for a playlist run, so the
    /// server has to store an entry that is not a video id. Nothing here resolves
    /// queue entries against the catalog, and this is what keeps it that way.
    #[tokio::test]
    async fn a_queued_playlist_run_survives_a_round_trip() {
        let state = AppServerState::initialize().await.unwrap();
        let owner = test_owner(&state).await;
        let video_id = crate::models::DemoLibrary::demo().videos[0].id.clone();
        state
            .save_playlist(&owner, "watch-later", "Watch later")
            .await
            .unwrap();
        state
            .set_queue(
                &owner,
                &[playlist_queue_entry("watch-later"), video_id.clone()],
            )
            .await
            .unwrap();

        let reread = state.viewer(&owner).await.unwrap();
        assert_eq!(
            reread.queue,
            vec![playlist_queue_entry("watch-later"), video_id],
            "the run marker keeps its place among the queued videos"
        );
        assert_eq!(
            queued_playlist_id(&reread.queue[0]),
            Some("watch-later"),
            "and reads back as the run it was"
        );
    }

    #[tokio::test]
    async fn websub_renewal_is_a_noop_without_a_public_callback() {
        let state = AppServerState::initialize().await.unwrap();
        let _owner = test_owner(&state).await;
        assert_eq!(state.renew_websub_subscriptions().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn playback_session_is_ungated_or_says_why_not() {
        let state = AppServerState::initialize().await.unwrap();
        let _owner = test_owner(&state).await;
        match state.playback_session("aqz-KE-bpKQ", true, true).await {
            Ok(session) => {
                assert!(session.fallback_url.contains("youtube-nocookie.com"));
                assert!(session.primary.url.contains("/api/v1/playback/proxy/"));
            }
            Err(failure) => assert!(!failure.summary.is_empty(), "{failure}"),
        }
    }

    #[test]
    fn names_why_no_stream_could_be_played() {
        let unreachable = PlaybackFailure::new(PlaybackFailureOrigin::Setup, "unreachable");
        assert_eq!(no_stream_failure(Some(unreachable.clone()), 0), unreachable);
        assert_eq!(
            no_stream_failure(None, 0).origin,
            PlaybackFailureOrigin::Unknown
        );
        let unplayable = no_stream_failure(None, 12);
        assert_eq!(unplayable.origin, PlaybackFailureOrigin::Bug);
        assert!(unplayable.summary.contains("12 streams"));
    }

    #[test]
    fn sorts_ytdlp_errors_by_whose_problem_they_are() {
        use PlaybackFailureOrigin::{Setup, Unknown, Youtube};
        let cases = [
            // Verbatim from the extractor log on 2026-09-25 (ysz5S6PUM-U).
            ("ERROR: [youtube] ysz5S6PUM-U: Video unavailable", Youtube),
            (
                "ERROR: [youtube] abcdefghijk: The uploader has not made this video available in your country",
                Youtube,
            ),
            (
                "ERROR: [youtube] abcdefghijk: Sign in to confirm you\u{2019}re not a bot. Use --cookies",
                Youtube,
            ),
            (
                "ERROR: [youtube] abcdefghijk: Private video. Sign in if you've been granted access",
                Youtube,
            ),
            (
                "ERROR: [youtube] abcdefghijk: Sign in to confirm your age. This video may be inappropriate for some users.",
                Youtube,
            ),
            (
                "ERROR: [youtube] abcdefghijk: Premieres in 3 hours",
                Youtube,
            ),
            (
                "ERROR: [youtube] abcdefghijk: Unable to extract yt initial data; please report this issue",
                Setup,
            ),
            (
                "ERROR: [youtube] abcdefghijk: Unable to download API page: timed out",
                Setup,
            ),
            ("ERROR: something new entirely", Unknown),
        ];
        for (stderr, origin) in cases {
            assert_eq!(classify_ytdlp_error(stderr).origin, origin, "{stderr}");
        }
        let unavailable = classify_ytdlp_error(
            "WARNING: noise\nERROR: [youtube] ysz5S6PUM-U: Video unavailable\n",
        );
        assert_eq!(unavailable.detail.as_deref(), Some("Video unavailable"));
        assert!(unavailable.remedy.is_some());
    }

    #[test]
    fn reads_the_extractors_status_answers() {
        let failed = extractor_status_failure(
            reqwest::StatusCode::BAD_GATEWAY,
            br#"{"error": "yt-dlp extraction failed", "ytdlp_error": "ERROR: [youtube] ysz5S6PUM-U: Video unavailable"}"#,
        );
        assert_eq!(failed.origin, PlaybackFailureOrigin::Youtube);
        let old_extractor = extractor_status_failure(
            reqwest::StatusCode::BAD_GATEWAY,
            br#"{"error": "yt-dlp extraction failed"}"#,
        );
        assert_eq!(old_extractor.origin, PlaybackFailureOrigin::Unknown);
        assert!(old_extractor.remedy.unwrap().contains("rebuild"));
        let no_provider = extractor_status_failure(reqwest::StatusCode::SERVICE_UNAVAILABLE, b"");
        assert_eq!(no_provider.origin, PlaybackFailureOrigin::Setup);
    }

    /// The property the whole per-user rewrite exists for.
    ///
    /// Two accounts on one instance share the catalog and nothing else. Before
    /// this, subscriptions and watch progress were columns on the shared
    /// `channel` and `video` rows, so a second account inherited the first
    /// account's entire library the moment it connected.
    #[tokio::test]
    async fn one_account_cannot_see_another_account_library() {
        let state = AppServerState::initialize().await.unwrap();
        let first = test_owner(&state).await;
        let second = test_owner(&state).await;

        let video_id = crate::models::DemoLibrary::demo().videos[0].id.clone();
        let channel = demo_channels()[0].clone();
        set_follow(&state, &first, &channel, true).await;
        state
            .save_progress(
                &first,
                &[crate::models::VideoProgress {
                    video_id: video_id.clone(),
                    watched: true,
                    progress_seconds: 42,
                    audio_only: false,
                }],
            )
            .await
            .unwrap();
        state
            .save_playlist(&first, "watch-later", "Mine")
            .await
            .unwrap();
        state
            .set_in_playlist(&first, "watch-later", &video_id, true)
            .await
            .unwrap();
        state.record_history(&first, &video_id).await.unwrap();
        state
            .set_queue(&first, std::slice::from_ref(&video_id))
            .await
            .unwrap();

        let theirs = state.viewer(&second).await.unwrap();
        assert!(
            theirs.subscriptions.is_empty(),
            "subscriptions leaked between accounts"
        );
        assert!(
            theirs.playlists.is_empty(),
            "playlists leaked between accounts"
        );
        assert!(
            theirs.queue.is_empty() && theirs.history.is_empty(),
            "queue or history leaked between accounts"
        );
        // The catalog is deliberately shared: the same upload must not be
        // fetched and stored twice because two people follow the channel.
        let seen = state
            .videos_by_id(&second, std::slice::from_ref(&video_id))
            .await
            .unwrap();
        assert_eq!(seen.len(), 1);
        assert!(!seen[0].watched, "watch progress leaked between accounts");
        assert_eq!(
            seen[0].progress_seconds, 0,
            "playback position leaked between accounts"
        );
        // The fresh read a player makes when a video starts: its own place
        // only.
        let mine = state.video_progress(&first, &video_id).await.unwrap();
        assert_eq!(mine.map(|progress| progress.progress_seconds), Some(42));
        assert!(
            state
                .video_progress(&second, &video_id)
                .await
                .unwrap()
                .is_none(),
            "a single video's position leaked between accounts"
        );

        // The second account can hold the same client-chosen playlist id...
        state
            .save_playlist(&second, "watch-later", "Theirs")
            .await
            .unwrap();
        let ours = state.viewer(&second).await.unwrap();
        assert_eq!(ours.playlists.len(), 1);
        assert_eq!(ours.playlists[0].name, "Theirs");

        // ...without disturbing the first account's playlist of the same id.
        let mine_again = state.viewer(&first).await.unwrap();
        assert_eq!(mine_again.playlists.len(), 1);
        assert_eq!(mine_again.playlists[0].name, "Mine");
        assert_eq!(mine_again.playlists[0].video_ids, [video_id]);
    }

    /// Unsubscribing must not silence a channel other accounts still follow.
    #[tokio::test]
    async fn a_channel_stays_polled_while_anyone_still_follows_it() {
        let state = AppServerState::initialize().await.unwrap();
        let first = test_owner(&state).await;
        let second = test_owner(&state).await;

        let channel = demo_channels()[0].clone();
        let channel_id = channel.id.clone();

        set_follow(&state, &first, &channel, true).await;
        set_follow(&state, &second, &channel, true).await;
        assert!(state.channel_is_subscribed(&channel_id).await.unwrap());

        // One leaves; the instance still has a reason to poll.
        set_follow(&state, &first, &channel, false).await;
        assert!(
            state.channel_is_subscribed(&channel_id).await.unwrap(),
            "the last remaining follower lost their feed"
        );

        // Both gone, and only then does the instance stop caring.
        set_follow(&state, &second, &channel, false).await;
        assert!(!state.channel_is_subscribed(&channel_id).await.unwrap());
    }

    /// Video details are cached for every account at once, so the follow
    /// state inside them is whoever fetched them first. Each account has to
    /// see its own.
    #[tokio::test]
    async fn video_details_carry_each_accounts_own_follow_state() {
        let state = AppServerState::initialize().await.unwrap();
        let follower = test_owner(&state).await;
        let other = test_owner(&state).await;

        let channel = demo_channels()[0].clone();
        set_follow(&state, &follower, &channel, true).await;
        let demo = crate::models::DemoLibrary::demo();
        let video = demo
            .videos
            .iter()
            .find(|video| video.channel_id == channel.id)
            .cloned()
            .unwrap_or_else(|| demo.videos[0].clone());
        use crate::models::{Channel, CommentsPage, VideoDetails};
        let cached_with = |subscribed: bool| VideoDetails {
            video: video.clone(),
            channel: Some(Channel {
                subscribed,
                ..channel.clone()
            }),
            description: String::new(),
            like_count: 0,
            dislike_count: 0,
            captions: Vec::new(),
            chapters: Vec::new(),
            preview_frames: None,
            related_videos: Vec::new(),
            comments: CommentsPage::default(),
            remote_available: true,
        };
        let followed = |details: VideoDetails| details.channel.unwrap().subscribed;

        // Cached by the follower, read by someone who does not follow.
        let seen = state
            .with_owner_follow_state(&other, cached_with(true))
            .await
            .unwrap();
        assert!(!followed(seen), "another account's follow leaked through");

        // Cached before following - or by the other account - read by the
        // follower: the reported bug.
        let seen = state
            .with_owner_follow_state(&follower, cached_with(false))
            .await
            .unwrap();
        assert!(followed(seen), "a follower was shown Subscribe");
    }

    /// Search is cached by query for every account, and a channel in it that
    /// the client has not seen is added to the library flag and all - so a
    /// wrong "subscribed" here is a follow nobody asked for.
    #[tokio::test]
    async fn search_results_carry_each_accounts_own_follow_state() {
        use crate::models::{Channel, SearchResults};
        let state = AppServerState::initialize().await.unwrap();
        let follower = test_owner(&state).await;
        let other = test_owner(&state).await;

        let followed_channel = demo_channels()[0].clone();
        let unfollowed_channel = demo_channels()[1].clone();
        set_follow(&state, &follower, &followed_channel, true).await;
        // As the shared cache might hold them: flags from the instance, or
        // from whichever account searched first.
        let cached = SearchResults {
            query: "anything".into(),
            videos: Vec::new(),
            channels: vec![
                Channel {
                    subscribed: false,
                    ..followed_channel.clone()
                },
                Channel {
                    subscribed: true,
                    ..unfollowed_channel.clone()
                },
            ],
            suggestion: None,
            remote_available: true,
            next_page: None,
        };
        let flags = |results: SearchResults| {
            results
                .channels
                .into_iter()
                .map(|channel| channel.subscribed)
                .collect::<Vec<_>>()
        };

        let mine = state
            .with_owner_follows_in_search(&follower, cached.clone())
            .await
            .unwrap();
        assert_eq!(flags(mine), vec![true, false]);

        let theirs = state
            .with_owner_follows_in_search(&other, cached)
            .await
            .unwrap();
        assert_eq!(flags(theirs), vec![false, false]);
    }

    #[tokio::test]
    #[ignore = "live YouTube connectivity check"]
    async fn live_youtube_search_subscribe_channel_and_feed_round_trip() {
        let state = AppServerState::initialize().await.unwrap();
        let owner = test_owner(&state).await;
        let results = state
            .search_catalog(&owner, "Alan Chikin Chow", "all")
            .await
            .unwrap();
        assert!(results.remote_available);
        assert!(!results.videos.is_empty());
        let channel = results
            .channels
            .iter()
            .find(|channel| {
                channel.id.starts_with("UC")
                    && channel
                        .name
                        .to_ascii_lowercase()
                        .contains("alan chikin chow")
            })
            .or_else(|| {
                results
                    .channels
                    .iter()
                    .find(|channel| channel.id.starts_with("UC"))
            })
            .cloned()
            .expect("real YouTube channel search result");

        let details = state
            .channel_details(&owner, &channel.id, "newest")
            .await
            .unwrap();
        eprintln!(
            "live channel: {} ({}) videos={} shorts={} live={}",
            channel.name,
            channel.id,
            details.videos.videos.len(),
            details.shorts.videos.len(),
            details.live.videos.len()
        );
        assert!(details.remote_available);
        assert!(
            !details.videos.videos.is_empty()
                || !details.shorts.videos.is_empty()
                || !details.live.videos.is_empty()
        );
        assert!(!details.shorts.videos.is_empty(), "Shorts tab is populated");
        let playable_video = details
            .videos
            .videos
            .first()
            .or_else(|| details.shorts.videos.first())
            .expect("channel has a playable upload");
        let video_details = state
            .video_details(&owner, &playable_video.id)
            .await
            .unwrap();
        assert!(video_details.remote_available);
        let playback = state
            .playback_session(&playable_video.id, true, true)
            .await
            .unwrap();
        eprintln!("live playback protocol: {:?}", playback.primary.protocol);
        assert!(playback.primary.url.contains("/api/v1/playback/proxy/"));

        state
            .set_subscriptions(
                &owner,
                vec![crate::models::SubscriptionChange {
                    channel: channel.clone(),
                    subscribed: true,
                    content: crate::models::SubscriptionContent::All,
                }],
            )
            .await
            .unwrap();
        let viewer = state.viewer(&owner).await.unwrap();
        assert!(viewer.follows(&channel.id).is_some());

        let refresh = state.refresh_feed(&owner).await.unwrap();
        assert_eq!(refresh.failed_channels, 0);
        let query = crate::models::FeedQuery {
            group: crate::models::FeedGroup::All,
            kind: crate::models::FeedFilter::All,
            duration: None,
            hide_watched: false,
            thresholds: (&crate::models::AppSettings::default()).into(),
        };
        let feed = state.feed_page(&owner, &query, 0).await.unwrap();
        let channel_feed = feed
            .videos
            .iter()
            .filter(|video| video.channel_id == channel.id)
            .collect::<Vec<_>>();
        assert!(!channel_feed.is_empty());
        assert!(
            channel_feed
                .windows(2)
                .all(|pair| { pair[0].published_epoch() >= pair[1].published_epoch() })
        );
    }
    /// Header layout measured from a real YouTube itag 160 stream:
    /// ftyp@0+28, moov@28+710, sidx@738+3812.
    fn mp4_head(boxes: &[(&[u8; 4], u32)]) -> Vec<u8> {
        let mut out = Vec::new();
        for (kind, size) in boxes {
            out.extend_from_slice(&size.to_be_bytes());
            out.extend_from_slice(*kind);
            out.resize(out.len() + (*size as usize - 8), 0);
        }
        out
    }

    #[test]
    fn mp4_index_is_the_sidx_box_and_init_is_everything_before_it() {
        let head = mp4_head(&[(b"ftyp", 28), (b"moov", 710), (b"sidx", 3812)]);
        let ranges = mp4_segment_ranges(&head).expect("sidx found");
        assert_eq!(ranges.init_end, 737);
        assert_eq!(ranges.index_start, 738);
        assert_eq!(ranges.index_end, 4549);
    }

    #[test]
    fn known_ranges_are_only_kept_for_a_file_named_once() {
        use crate::models::PlaybackByteRange;
        let range = |start, end| PlaybackByteRange { start, end };
        let known = unambiguous_segment_ranges([
            (137, 5_000, range(0, 740), range(741, 1_900)),
            // Two dubbed languages of one itag, the same length: either
            // could be meant, so neither is trusted.
            (140, 9_000, range(0, 630), range(631, 1_200)),
            (140, 9_000, range(0, 640), range(641, 1_210)),
            // The same itag at another length is another file.
            (140, 9_100, range(0, 650), range(651, 1_220)),
            // Not the init-then-index layout: left to the probe.
            (251, 7_000, range(0, 300), range(400, 900)),
        ]);
        assert_eq!(
            known.get(&(137, 5_000)),
            Some(&SegmentRanges {
                init_end: 740,
                index_start: 741,
                index_end: 1_900,
            })
        );
        assert_eq!(known.get(&(140, 9_000)), None);
        assert_eq!(
            known.get(&(140, 9_100)).map(|ranges| ranges.index_start),
            Some(651)
        );
        assert_eq!(known.get(&(251, 7_000)), None);
        assert_eq!(known.len(), 2);
    }

    #[test]
    fn mp4_walk_stops_rather_than_looping_on_a_zero_sized_box() {
        let mut head = Vec::new();
        head.extend_from_slice(&0u32.to_be_bytes());
        head.extend_from_slice(b"free");
        head.resize(64, 0);
        assert_eq!(mp4_segment_ranges(&head), None);
    }

    #[test]
    fn ebml_vint_widths_and_marker_handling() {
        // 0x81 is a one-byte value of 1 once the marker is stripped.
        assert_eq!(ebml_vint(&[0x81], 0, false), Some((1, 1)));
        assert_eq!(ebml_vint(&[0x81], 0, true), Some((0x81, 1)));
        // 0x1A45DFA3 is the four-byte EBML header id, kept whole.
        assert_eq!(
            ebml_vint(&[0x1A, 0x45, 0xDF, 0xA3], 0, true),
            Some((0x1A45_DFA3, 4))
        );
        assert_eq!(ebml_vint(&[0x00], 0, false), None);
    }

    /// Emit one EBML element: id bytes, then a size vint of the given width,
    /// then a zeroed payload.
    fn ebml_element(out: &mut Vec<u8>, id: &[u8], payload: usize, size_width: usize) {
        out.extend_from_slice(id);
        let marker = 1u64 << (7 * size_width);
        let size = marker | payload as u64;
        let bytes = size.to_be_bytes();
        out.extend_from_slice(&bytes[8 - size_width..]);
        out.resize(out.len() + payload, 0);
    }

    /// Layout measured from a real YouTube itag 278 stream: an EBML header of
    /// 36 bytes, a Segment whose children run 44..219, then Cues at 219
    /// spanning 5345 bytes.
    #[test]
    fn webm_index_is_the_cues_element_inside_the_segment() {
        let mut head = Vec::new();
        ebml_element(&mut head, &[0x1A, 0x45, 0xDF, 0xA3], 31, 1);
        assert_eq!(head.len(), 36);
        // Segment is a master element: header only, with a four-byte declared
        // size covering the children that follow rather than a payload of its
        // own.
        head.extend_from_slice(&[0x18, 0x53, 0x80, 0x67]);
        let segment_payload = 5564u64 - 44;
        head.extend_from_slice(&((1u64 << 28) | segment_payload).to_be_bytes()[4..]);
        assert_eq!(head.len(), 44);
        for payload in [42usize, 54, 64] {
            ebml_element(&mut head, &[0x11, 0x4D, 0x9B, 0x74], payload, 1);
        }
        assert_eq!(head.len(), 219);
        ebml_element(&mut head, &[0x1C, 0x53, 0xBB, 0x6B], 5339, 2);
        assert_eq!(head.len(), 5564);

        let ranges = webm_segment_ranges(&head).expect("cues found");
        assert_eq!(ranges.init_end, 218);
        assert_eq!(ranges.index_start, 219);
        assert_eq!(ranges.index_end, 5563);
    }

    #[test]
    fn ebml_vint_of_full_width_does_not_overflow_its_mask() {
        // An eight-byte size carries no value bits in its first byte.
        assert_eq!(
            ebml_vint(&[0x01, 0, 0, 0, 0, 0, 0, 7], 0, false),
            Some((7, 8))
        );
    }

    #[test]
    fn container_is_chosen_by_magic_not_by_extension() {
        let mp4 = mp4_head(&[(b"ftyp", 28), (b"sidx", 100)]);
        assert!(segment_ranges(&mp4).is_some());
        assert_eq!(segment_ranges(b"not a media container at all"), None);
    }
}

// ---------------------------------------------------------------------------
// SponsorBlock
// ---------------------------------------------------------------------------

/// How long a fetched segment list is trusted.
///
/// Segments change when someone submits or votes, which is slow enough that an
/// hour is generous and short enough that a correction lands the same evening.
const SPONSOR_CACHE_TTL: Duration = Duration::from_secs(60 * 60);
/// Segments by video and the category set asked for, because those are
/// different answers for the same video.
static SPONSOR_SEGMENTS: ServerCache<(String, String), Vec<SponsorSegment>> =
    ServerCache::new(SPONSOR_CACHE_TTL, 10_000);

/// The public instance. Self-hosters who run their own can point at it.
const SPONSOR_API_DEFAULT: &str = "https://sponsor.ajay.app";

#[derive(Debug, Deserialize)]
struct SponsorApiVideo {
    #[serde(rename = "videoID")]
    video_id: String,
    segments: Vec<SponsorApiSegment>,
}

#[derive(Debug, Deserialize)]
struct SponsorApiSegment {
    #[serde(rename = "UUID")]
    uuid: String,
    category: String,
    /// `[start, end]` in seconds. A point segment has both the same.
    segment: Vec<f64>,
    #[serde(rename = "actionType", default)]
    action_type: String,
    #[serde(default)]
    locked: i64,
    #[serde(default)]
    votes: i64,
}

impl AppServerState {
    /// Segments for one video, from SponsorBlock.
    ///
    /// Fetched by the server rather than the client on purpose. The client would
    /// otherwise talk to sponsor.ajay.app directly from every viewer's address,
    /// which is exactly the correlation a self-hosted instance exists to avoid -
    /// and it lets one fetch serve every account here.
    ///
    /// The lookup is by hash prefix, which is SponsorBlock's own privacy
    /// mechanism: the first four hex characters of the video id's SHA-256 go in
    /// the path, the API answers with every video sharing that prefix, and the
    /// caller picks its own out. The server therefore never tells SponsorBlock
    /// which video was actually watched.
    pub async fn sponsor_segments(
        &self,
        video_id: &str,
        categories: &[SponsorCategory],
    ) -> Result<Vec<SponsorSegment>> {
        if categories.is_empty() {
            return Ok(Vec::new());
        }
        let cache_key = (
            video_id.to_string(),
            sponsor_cache_discriminator(categories),
        );
        if let Some(cached) = SPONSOR_SEGMENTS.get(&cache_key).await {
            return Ok(cached);
        }

        let digest = hex::encode(Sha256::digest(video_id.as_bytes()));
        let prefix = &digest[..4];
        let wanted = categories
            .iter()
            .map(|category| format!("\"{}\"", category.api_name()))
            .collect::<Vec<_>>()
            .join(",");
        let url = format!(
            "{}/api/skipSegments/{prefix}?categories=[{wanted}]&actionTypes=[\"skip\",\"poi\"]",
            self.sponsor_api_url()
        );

        let response = self.http.get(&url).send().await?;
        // 404 is the ordinary answer for "nobody has submitted anything with
        // this prefix", not a failure worth surfacing.
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            self.remember_sponsor_segments(cache_key, Vec::new()).await;
            return Ok(Vec::new());
        }
        let response = response.error_for_status()?;
        let videos: Vec<SponsorApiVideo> = response.json().await?;

        let segments = videos
            .into_iter()
            .find(|video| video.video_id == video_id)
            .map(|video| convert_sponsor_segments(video.segments))
            .unwrap_or_default();
        self.remember_sponsor_segments(cache_key, segments.clone())
            .await;
        Ok(segments)
    }

    fn sponsor_api_url(&self) -> &str {
        self.sponsor_api_url
            .as_deref()
            .unwrap_or(SPONSOR_API_DEFAULT)
    }

    async fn remember_sponsor_segments(
        &self,
        key: (String, String),
        segments: Vec<SponsorSegment>,
    ) {
        SPONSOR_SEGMENTS.insert(key, segments).await;
    }
}

/// Two requests for the same video with different categories are different
/// answers, so the cache has to tell them apart.
fn sponsor_cache_discriminator(categories: &[SponsorCategory]) -> String {
    let mut names = categories
        .iter()
        .map(|category| category.api_name())
        .collect::<Vec<_>>();
    names.sort_unstable();
    names.join(",")
}

/// Turn the API's shape into Tawny's, dropping what cannot be used.
///
/// Overlaps are real: several people submit the same sponsor with slightly
/// different bounds. Keeping all of them would draw a muddle on the timeline
/// and skip the same break twice, so within a category the best-supported
/// submission wins any overlap - locked first, then votes, which is the order
/// SponsorBlock itself trusts them in.
fn convert_sponsor_segments(raw: Vec<SponsorApiSegment>) -> Vec<SponsorSegment> {
    let mut segments = raw
        .into_iter()
        .filter_map(|item| {
            let category = SponsorCategory::from_api_name(&item.category)?;
            let start = *item.segment.first()?;
            let end = *item.segment.get(1)?;
            if !start.is_finite() || !end.is_finite() || end < start {
                return None;
            }
            // A poi is a point; anything else with no length would be an
            // invisible segment that still triggers a skip.
            let is_point = item.action_type == "poi" || category.is_point();
            if !is_point && end <= start {
                return None;
            }
            Some(SponsorSegment {
                uuid: item.uuid,
                category,
                start_seconds: start.max(0.0),
                end_seconds: end.max(0.0),
                locked: item.locked > 0,
                votes: item.votes,
            })
        })
        .collect::<Vec<_>>();

    // Best first, so the retain below keeps the winner of each overlap.
    segments.sort_by(|a, b| {
        b.locked
            .cmp(&a.locked)
            .then(b.votes.cmp(&a.votes))
            .then(a.start_seconds.total_cmp(&b.start_seconds))
    });

    let mut kept: Vec<SponsorSegment> = Vec::new();
    for segment in segments {
        let overlaps = kept.iter().any(|existing| {
            existing.category == segment.category
                && segment.start_seconds < existing.end_seconds
                && existing.start_seconds < segment.end_seconds
        });
        if !overlaps {
            kept.push(segment);
        }
    }

    kept.sort_by(|a, b| a.start_seconds.total_cmp(&b.start_seconds));
    kept
}

#[cfg(test)]
mod sponsor_server_tests {
    use super::{SponsorApiSegment, convert_sponsor_segments, sponsor_cache_discriminator};
    use crate::models::SponsorCategory;

    fn raw(category: &str, start: f64, end: f64, locked: i64, votes: i64) -> SponsorApiSegment {
        SponsorApiSegment {
            uuid: format!("{category}-{start}-{end}"),
            category: category.into(),
            segment: vec![start, end],
            action_type: "skip".into(),
            locked,
            votes,
        }
    }

    /// The exact shape sponsor.ajay.app returns, checked against a live
    /// response: `locked` and `votes` are integers, not booleans.
    #[test]
    fn the_api_payload_deserializes() {
        let payload = r#"{
            "category": "intro",
            "actionType": "skip",
            "segment": [0, 14.827],
            "UUID": "5d683910",
            "videoDuration": 3238.835,
            "locked": 0,
            "votes": 3,
            "description": ""
        }"#;
        let parsed: SponsorApiSegment = serde_json::from_str(payload).expect("parse");
        assert_eq!(parsed.uuid, "5d683910");
        assert_eq!(parsed.segment, vec![0.0, 14.827]);
        assert_eq!(parsed.votes, 3);
    }

    #[test]
    fn segments_come_back_in_playback_order() {
        let converted = convert_sponsor_segments(vec![
            raw("outro", 500.0, 540.0, 0, 0),
            raw("sponsor", 30.0, 60.0, 0, 0),
            raw("intro", 0.0, 10.0, 0, 0),
        ]);
        let starts = converted
            .iter()
            .map(|segment| segment.start_seconds)
            .collect::<Vec<_>>();
        assert_eq!(starts, vec![0.0, 30.0, 500.0]);
    }

    #[test]
    fn a_locked_submission_wins_an_overlap() {
        // Two people submitted the same sponsor break. Drawing both would
        // muddle the timeline and skip it twice.
        let converted = convert_sponsor_segments(vec![
            raw("sponsor", 30.0, 60.0, 0, 40),
            raw("sponsor", 28.0, 62.0, 1, 2),
        ]);
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0].start_seconds, 28.0);
        assert!(converted[0].locked);
    }

    #[test]
    fn votes_break_a_tie_when_neither_is_locked() {
        let converted = convert_sponsor_segments(vec![
            raw("sponsor", 30.0, 60.0, 0, 2),
            raw("sponsor", 31.0, 59.0, 0, 99),
        ]);
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0].start_seconds, 31.0);
    }

    #[test]
    fn overlapping_segments_of_different_categories_both_survive() {
        // An intro that overlaps a sponsor is two separate things to show.
        let converted = convert_sponsor_segments(vec![
            raw("sponsor", 0.0, 30.0, 0, 0),
            raw("intro", 0.0, 10.0, 0, 0),
        ]);
        assert_eq!(converted.len(), 2);
    }

    #[test]
    fn unusable_segments_are_dropped_rather_than_skipped_over() {
        let converted = convert_sponsor_segments(vec![
            // Unknown category from a newer API.
            raw("chapter", 0.0, 10.0, 0, 0),
            // Backwards.
            raw("sponsor", 60.0, 30.0, 0, 0),
            // Zero length, which would be an invisible segment that still fires.
            raw("outro", 45.0, 45.0, 0, 0),
        ]);
        assert!(converted.is_empty());
    }

    #[test]
    fn a_point_segment_survives_having_no_length() {
        let mut point = raw("poi_highlight", 90.0, 90.0, 0, 0);
        point.action_type = "poi".into();
        let converted = convert_sponsor_segments(vec![point]);
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0].category, SponsorCategory::Highlight);
    }

    #[test]
    fn the_cache_key_ignores_the_order_categories_were_asked_in() {
        let one = sponsor_cache_discriminator(&[SponsorCategory::Sponsor, SponsorCategory::Intro]);
        let other =
            sponsor_cache_discriminator(&[SponsorCategory::Intro, SponsorCategory::Sponsor]);
        assert_eq!(one, other);
        // ...but not which categories they were.
        assert_ne!(
            one,
            sponsor_cache_discriminator(&[SponsorCategory::Sponsor])
        );
    }
}
