use super::*;

/// The playlist every account starts with, and where the default left swipe
/// saves a video.
pub const WATCH_LATER_PLAYLIST_ID: &str = "watch-later";

/// How a playlist's videos are ordered on its page.
///
/// `Added` is the playlist's own order and is the default, because a hand-built
/// watchlist already carries the order its owner chose. Anything else would make
/// the control destructive by default rather than helpful.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlaylistSort {
    #[default]
    Added,
    Published,
    Duration,
    Title,
}

impl PlaylistSort {
    pub const ALL: [Self; 4] = [Self::Added, Self::Published, Self::Duration, Self::Title];

    /// The label names the resulting order rather than the field it sorts on,
    /// because the chip is the only place the direction is visible: tapping an
    /// active chip flips it, and the text has to be what says so.
    pub fn label(self, descending: bool) -> &'static str {
        match (self, descending) {
            (Self::Added, false) => "First added",
            (Self::Added, true) => "Last added",
            (Self::Published, true) => "Newest",
            (Self::Published, false) => "Oldest",
            (Self::Duration, true) => "Longest",
            (Self::Duration, false) => "Shortest",
            (Self::Title, false) => "A-Z",
            (Self::Title, true) => "Z-A",
        }
    }

    /// Which direction a sort lands on when it is first picked. Newest and
    /// longest are what someone reaches for; oldest and shortest are the second
    /// tap.
    pub fn default_descending(self) -> bool {
        matches!(self, Self::Published | Self::Duration)
    }

    /// Order `videos` in place. They arrive in the playlist's own order, which
    /// is why `Added` sorts on nothing.
    ///
    /// Reversing after a stable sort rather than comparing backwards also
    /// inverts ties. For `Added` that is the whole point, and elsewhere a tie
    /// means equal keys, so the two orders are equally correct.
    pub fn apply(self, videos: &mut [Video], descending: bool) {
        match self {
            Self::Added => {}
            // Cached, because parsing a published date is not free and a
            // playlist can hold hundreds of them.
            Self::Published => videos.sort_by_cached_key(|video| video.published_epoch()),
            Self::Duration => videos.sort_by_key(|video| video.duration_seconds),
            Self::Title => videos.sort_by_cached_key(|video| video.title.to_lowercase()),
        }
        if descending {
            videos.reverse();
        }
    }
}

/// Which kinds of upload a playlist page is showing.
///
/// Deliberately not [`FeedFilter`], which also carries `Live`. A playlist holds
/// whatever was saved to it, and a persisted filter the segmented control cannot
/// display is a filter nobody can turn off.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlaylistKind {
    #[default]
    All,
    Videos,
    Shorts,
}

impl PlaylistKind {
    pub const ALL: [Self; 3] = [Self::All, Self::Videos, Self::Shorts];

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Videos => "Videos",
            Self::Shorts => "Shorts",
        }
    }

    /// A live upload counts as a video here. The feed splits Live out because a
    /// stream is a different thing to sit down to; a playlist entry is something
    /// its owner filed by hand, and hiding it under a chip it never named would
    /// just lose it.
    pub fn matches(self, video: &Video) -> bool {
        match self {
            Self::All => true,
            Self::Videos => !video.is_short,
            Self::Shorts => video.is_short,
        }
    }
}

/// How one playlist is arranged: what it shows and in what order.
///
/// Persisted per playlist, because a run is resolved lazily out of the queue -
/// see [`PLAYLIST_QUEUE_PREFIX`] - so the arrangement has to outlive the page
/// that set it. Without that, autoplay would walk a different playlist than the
/// one the viewer was looking at when they pressed play.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PlaylistView {
    #[serde(default)]
    pub kind: PlaylistKind,
    #[serde(default)]
    pub only_unwatched: bool,
    #[serde(default)]
    pub duration: Option<DurationFilter>,
    #[serde(default)]
    pub sort: PlaylistSort,
    #[serde(default)]
    pub descending: bool,
}

impl PlaylistView {
    /// Kind and duration filters plus the sort - everything except the watched
    /// filter.
    ///
    /// The watched filter is left out on purpose. A run finds its place by
    /// locating the video that just finished, and finishing a video is exactly
    /// what marks it watched: dropping it here would lose the position and send
    /// the run back to the top of the playlist. Apply [`Self::shows`] on top of
    /// this order instead.
    pub fn arrange(&self, videos: &mut Vec<Video>, settings: &AppSettings) {
        videos.retain(|video| self.kind.matches(video));
        if let Some(duration) = self.duration {
            videos.retain(|video| duration.matches(video, settings));
        }
        self.sort.apply(videos, self.descending);
    }

    /// Whether the watched filter keeps this video on screen.
    pub fn shows(&self, video: &Video) -> bool {
        !self.only_unwatched || !video.watched
    }

    /// What a run plays after `current_video_id`, given an [`Self::arrange`]d
    /// order.
    ///
    /// A current video that is not in `order` means the run has not started yet -
    /// it is sitting behind something else in the queue - so it begins at the
    /// top.
    pub fn next_after(&self, order: &[Video], current_video_id: &str) -> Option<String> {
        let start = order
            .iter()
            .position(|video| video.id == current_video_id)
            .map_or(0, |index| index + 1);
        order
            .get(start..)?
            .iter()
            .find(|video| self.shows(video))
            .map(|video| video.id.clone())
    }

    /// What a run plays before `current_video_id`. `None` once there is nothing
    /// shown ahead of it, and for a video the playlist does not hold: going back
    /// into a run you were never in has no meaning.
    pub fn previous_before(&self, order: &[Video], current_video_id: &str) -> Option<String> {
        let index = order
            .iter()
            .position(|video| video.id == current_video_id)?;
        order
            .get(..index)?
            .iter()
            .rev()
            .find(|video| self.shows(video))
            .map(|video| video.id.clone())
    }

    /// Whether anything is being hidden, which is what an empty page has to
    /// explain: "nothing matches" and "nothing saved" are different problems.
    pub fn is_filtered(&self) -> bool {
        self.only_unwatched || self.duration.is_some() || self.kind != PlaylistKind::All
    }

    /// The filters in force, in words, for somewhere other than the playlist's
    /// own page to say what a run is walking. Empty when nothing is hidden.
    pub fn filter_labels(&self) -> Vec<String> {
        let mut labels = Vec::new();
        match self.kind {
            PlaylistKind::All => {}
            PlaylistKind::Videos => labels.push("Videos only".to_string()),
            PlaylistKind::Shorts => labels.push("Shorts only".to_string()),
        }
        if let Some(duration) = self.duration {
            labels.push(format!("{} length", duration.label()));
        }
        if self.only_unwatched {
            labels.push("Unwatched only".to_string());
        }
        labels
    }
}

/// Marks the queue entry that stands in for "the rest of a playlist".
///
/// A run used to be spilled into the queue as a list of ids. That froze the
/// order and the filters at the moment Play was pressed, could not be told apart
/// from videos queued by hand, and buried a fifty-video playlist in the one
/// place meant for what comes next. A marker is resolved against the playlist's
/// current [`PlaylistView`] each time the run advances, so the queue stays one
/// flat list and a playlist occupies one entry in it.
///
/// A YouTube video id is eleven characters of `[A-Za-z0-9_-]`, so the `:` here
/// can never collide with one.
pub const PLAYLIST_QUEUE_PREFIX: &str = "playlist:";

pub fn playlist_queue_entry(playlist_id: &str) -> String {
    format!("{PLAYLIST_QUEUE_PREFIX}{playlist_id}")
}

/// The playlist a queue entry stands for, or `None` when it is a plain video id.
pub fn queued_playlist_id(entry: &str) -> Option<&str> {
    entry.strip_prefix(PLAYLIST_QUEUE_PREFIX)
}
