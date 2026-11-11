use super::*;

/// Where this viewer has reached in one video, sent by `save_progress`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoProgress {
    pub video_id: String,
    pub watched: bool,
    pub progress_seconds: u64,
    #[serde(default)]
    pub audio_only: bool,
}

/// One account's own state. Kilobytes: ids and names, never the videos they
/// point at.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Viewer {
    /// The channels this account follows, each with the uploads it wants.
    pub subscriptions: Vec<Channel>,
    pub playlists: Vec<Playlist>,
    pub subscription_groups: Vec<SubscriptionGroup>,
    /// Video ids, and `playlist:` markers for a queued run.
    pub queue: Vec<String>,
    /// Most recent first, at most [`HISTORY_LIMIT`].
    pub history: Vec<HistoryEntry>,
}

impl Viewer {
    pub fn follows(&self, channel_id: &str) -> Option<&Channel> {
        self.subscriptions
            .iter()
            .find(|channel| channel.id == channel_id)
    }

    pub fn playlist(&self, playlist_id: &str) -> Option<&Playlist> {
        self.playlists
            .iter()
            .find(|playlist| playlist.id == playlist_id)
    }
}

/// One subscription as a device last saw it, sent back as the answer to set.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SubscriptionChange {
    pub channel: Channel,
    pub subscribed: bool,
    pub content: SubscriptionContent,
}

/// How many history entries an account keeps.
pub const HISTORY_LIMIT: usize = 250;

/// One account's whole library, to move it between this device and a server
/// (`export_library`, then `import_library` on the other side).
///
/// It carries the videos its lists point at, so the other side can show them
/// before it has met them itself.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LibraryExport {
    /// Followed channels, each with the uploads it wants.
    pub subscriptions: Vec<Channel>,
    pub groups: Vec<SubscriptionGroup>,
    pub playlists: Vec<Playlist>,
    pub queue: Vec<String>,
    pub history: Vec<HistoryEntry>,
    pub progress: Vec<VideoProgress>,
    pub videos: Vec<Video>,
}

impl LibraryExport {
    pub fn is_empty(&self) -> bool {
        self.subscriptions.is_empty()
            && self.groups.is_empty()
            && self
                .playlists
                .iter()
                .all(|playlist| playlist.video_ids.is_empty())
            && self.queue.is_empty()
            && self.history.is_empty()
            && self.progress.is_empty()
    }
}

/// What an import added that the account did not have.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryImport {
    pub channels: usize,
    pub playlists: usize,
    pub playlist_videos: usize,
    pub history: usize,
}

#[cfg_attr(
    not(feature = "server"),
    allow(
        dead_code,
        reason = "Shared definitions are used by selected platform, server or test builds."
    )
)]
/// How many recommendations one page of the search page's list holds.
pub const RECOMMENDATION_PAGE_SIZE: usize = 24;

#[cfg_attr(
    not(feature = "server"),
    allow(
        dead_code,
        reason = "Shared definitions are used by selected platform, server or test builds."
    )
)]
/// How many videos one feed page holds.
pub const FEED_PAGE_SIZE: usize = 24;

/// Which subscriptions a feed draws from.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum FeedGroup {
    #[default]
    All,
    /// Channels in no group.
    Ungrouped,
    Group(String),
}

/// The length buckets' edges, from the viewer's settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DurationThresholds {
    pub video_short_max_seconds: u64,
    pub video_medium_max_seconds: u64,
    pub shorts_short_max_seconds: u64,
    pub shorts_medium_max_seconds: u64,
}

impl From<&AppSettings> for DurationThresholds {
    fn from(settings: &AppSettings) -> Self {
        Self {
            video_short_max_seconds: settings.video_short_max_seconds,
            video_medium_max_seconds: settings.video_medium_max_seconds,
            shorts_short_max_seconds: settings.shorts_short_max_seconds,
            shorts_medium_max_seconds: settings.shorts_medium_max_seconds,
        }
    }
}

/// Everything that decides which videos the subscription feed shows. Part of
/// the cache key, so each combination is its own cached answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeedQuery {
    pub group: FeedGroup,
    pub kind: FeedFilter,
    pub duration: Option<DurationFilter>,
    pub hide_watched: bool,
    pub thresholds: DurationThresholds,
}

/// One page of the subscription feed, newest first.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FeedPage {
    pub videos: Vec<Video>,
    pub has_more: bool,
    /// With a length filter on: how many videos on this page's stretch of the
    /// feed have no length yet, so the filter cannot speak for them.
    pub without_duration: usize,
    /// Those videos' ids, for the screen to ask their lengths for.
    pub unknown_durations: Vec<String>,
}

/// Anything that holds videos, so a change to one video (its progress, its
/// length) can be applied to every cached list showing it at once.
pub trait VideoList {
    fn videos_mut(&mut self) -> Box<dyn Iterator<Item = &mut Video> + '_>;

    fn update_video(&mut self, video_id: &str, mut update: impl FnMut(&mut Video)) {
        for video in self.videos_mut().filter(|video| video.id == video_id) {
            update(video);
        }
    }

    /// Take the catalog facts of `fresh` (length, live and Shorts state)
    /// without touching this viewer's progress.
    fn merge_metadata(&mut self, fresh: &[Video]) {
        for video in self.videos_mut() {
            if let Some(fresh) = fresh.iter().find(|fresh| fresh.id == video.id) {
                video.duration_seconds = fresh.duration_seconds;
                video.is_live = fresh.is_live;
                video.is_short = fresh.is_short;
            }
        }
    }
}

impl VideoList for Vec<Video> {
    fn videos_mut(&mut self) -> Box<dyn Iterator<Item = &mut Video> + '_> {
        Box::new(self.iter_mut())
    }
}

impl VideoList for FeedPage {
    fn videos_mut(&mut self) -> Box<dyn Iterator<Item = &mut Video> + '_> {
        Box::new(self.videos.iter_mut())
    }
}

/// One playlist on the playlists page: its cover and its counts, without the
/// videos themselves.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaylistPreview {
    pub id: String,
    pub name: String,
    pub count: usize,
    pub unwatched: usize,
    /// The first few videos' thumbnails, in playlist order.
    pub thumbnails: Vec<String>,
}

/// A playlist with its videos, in the playlist's own order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlaylistContents {
    pub playlist: Playlist,
    pub videos: Vec<Video>,
}

impl VideoList for PlaylistContents {
    fn videos_mut(&mut self) -> Box<dyn Iterator<Item = &mut Video> + '_> {
        Box::new(self.videos.iter_mut())
    }
}

// ---------------------------------------------------------------------------
// Accounts
// ---------------------------------------------------------------------------

/// One kind of change to an account's own data, as counted in
/// [`LibraryChanges`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LibraryChange {
    Subscriptions,
    Groups,
    Playlists,
    Queue,
    History,
    /// A video marked watched or unwatched, which lists show and filter on.
    Watched,
    /// Any saved position, including the ticks of a video playing elsewhere.
    Progress,
}

impl LibraryChange {
    pub const ALL: [Self; 7] = [
        Self::Subscriptions,
        Self::Groups,
        Self::Playlists,
        Self::Queue,
        Self::History,
        Self::Watched,
        Self::Progress,
    ];
}

/// Counters that move whenever this account's data changes, from any device.
///
/// A device asks for these often and cheaply, and compares them with the last
/// it saw, to learn what another device changed while its own screens sat on
/// cached answers. They are not data: only "something of this kind changed".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryChanges {
    /// Which run of the server counted these. The counts live in memory and
    /// start again on a restart, so a new epoch means anything may have
    /// changed.
    pub epoch: u64,
    pub subscriptions: u64,
    pub groups: u64,
    pub playlists: u64,
    pub queue: u64,
    pub history: u64,
    pub watched: u64,
    pub progress: u64,
}

impl LibraryChanges {
    fn count_mut(&mut self, change: LibraryChange) -> &mut u64 {
        match change {
            LibraryChange::Subscriptions => &mut self.subscriptions,
            LibraryChange::Groups => &mut self.groups,
            LibraryChange::Playlists => &mut self.playlists,
            LibraryChange::Queue => &mut self.queue,
            LibraryChange::History => &mut self.history,
            LibraryChange::Watched => &mut self.watched,
            LibraryChange::Progress => &mut self.progress,
        }
    }

    pub fn count(mut self, change: LibraryChange) -> u64 {
        *self.count_mut(change)
    }

    /// Counted on the server, as each write lands.
    #[cfg_attr(
        not(any(feature = "server", test)),
        allow(
            dead_code,
            reason = "Shared definitions are used by selected platform, server or test builds."
        )
    )]
    pub fn note(&mut self, change: LibraryChange) {
        let count = self.count_mut(change);
        *count = count.wrapping_add(1);
    }

    /// What changed between `before` and these. Everything, when the server
    /// restarted in between.
    pub fn changed_since(self, before: Self) -> Vec<LibraryChange> {
        LibraryChange::ALL
            .into_iter()
            .filter(|&change| {
                self.epoch != before.epoch || self.count(change) != before.count(change)
            })
            .collect()
    }
}

#[cfg(test)]
mod library_changes_tests {
    use super::*;

    #[test]
    fn only_the_kinds_that_moved_are_reported() {
        let before = LibraryChanges {
            epoch: 7,
            ..LibraryChanges::default()
        };
        let mut after = before;
        after.note(LibraryChange::Playlists);
        after.note(LibraryChange::Progress);
        after.note(LibraryChange::Progress);
        assert_eq!(
            after.changed_since(before),
            vec![LibraryChange::Playlists, LibraryChange::Progress]
        );
        assert!(after.changed_since(after).is_empty());
    }

    #[test]
    fn a_restarted_server_reports_everything() {
        let before = LibraryChanges {
            epoch: 1,
            ..LibraryChanges::default()
        };
        let after = LibraryChanges {
            epoch: 2,
            ..LibraryChanges::default()
        };
        assert_eq!(after.changed_since(before), LibraryChange::ALL.to_vec());
    }
}
