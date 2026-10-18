use super::*;

/// Which native design language the component library renders with.
///
/// g3-ui detects this from the platform at startup, which is right for a
/// packaged build but arbitrary on the web, where the same browser serves both
/// looks. Storing an explicit override lets the choice be made once and kept.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlatformStyle {
    /// Follow the running platform, which is what an installed build wants.
    #[default]
    Auto,
    Ios,
    Material,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Appearance {
    /// The system's light or dark setting, followed as it changes.
    #[default]
    Auto,
    Dark,
    Light,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AppSettings {
    pub appearance: Appearance,
    #[serde(default)]
    pub platform_style: PlatformStyle,
    pub playback_speed: f64,
    /// Shorts are watched at a different pace than long-form video, so the two
    /// speeds are remembered independently rather than sharing one setting.
    #[serde(default = "default_speed")]
    pub shorts_playback_speed: f64,
    pub swipe_right_playlist_id: String,
    pub swipe_left_playlist_id: String,
    /// What each swipe direction actually does. The playlist ids above are only
    /// consulted when the action is `AddToPlaylist`.
    #[serde(default)]
    pub swipe_right_action: SwipeActionKind,
    #[serde(default)]
    pub swipe_left_action: SwipeActionKind,
    pub hide_watched: bool,
    /// Long-form autoplay. Shorts get their own below, for the same reason
    /// speed does: a feed of ninety-second clips and a forty-minute video are
    /// not the same decision.
    pub autoplay: bool,
    #[serde(default = "default_true")]
    pub shorts_autoplay: bool,
    #[serde(default = "default_true")]
    pub auto_landscape_fullscreen: bool,
    /// Whether a desktop player opens in theater mode. A viewer who chose the
    /// wider stage once meant it for watching, not for this one video.
    #[serde(default)]
    pub theater_mode: bool,
    #[serde(default = "default_video_short_max_seconds")]
    pub video_short_max_seconds: u64,
    #[serde(default = "default_video_medium_max_seconds")]
    pub video_medium_max_seconds: u64,
    #[serde(default = "default_shorts_short_max_seconds")]
    pub shorts_short_max_seconds: u64,
    #[serde(default = "default_shorts_medium_max_seconds")]
    pub shorts_medium_max_seconds: u64,
    #[serde(default)]
    pub sponsor_block: SponsorBlockSettings,
    /// BCP-47 language tag the playback transport should prefer for dubbed
    /// uploads. Stored separately from the browser locale so a device's UI
    /// language cannot unexpectedly change what a viewer hears.
    #[serde(default = "default_audio_language")]
    pub preferred_audio_language: String,
    /// BCP-47 language tag of the captions a video opens with. Without it the
    /// first track was chosen, and YouTube lists them alphabetically, so most
    /// videos with translated captions came up in Arabic.
    #[serde(default = "default_caption_language")]
    pub preferred_caption_language: String,
    pub prefer_sabr: bool,
    pub po_token_provider_url: Option<String>,
    /// How each playlist is arranged, keyed by playlist id.
    ///
    /// Kept here rather than on [`Playlist`] because it is a per-device view of
    /// shared data - a phone and a desktop can reasonably sit on different
    /// filters - and because it has to be readable while resolving a queued run,
    /// which happens nowhere near the playlist page. Entries that hold nothing
    /// but defaults are dropped, so this stays empty until a filter is used.
    #[serde(default)]
    pub playlist_views: BTreeMap<String, PlaylistView>,
}

pub(crate) fn default_speed() -> f64 {
    1.0
}

pub(crate) fn default_true() -> bool {
    true
}

pub(crate) fn default_audio_language() -> String {
    "en".to_string()
}

pub(crate) fn default_caption_language() -> String {
    "en".to_string()
}

pub(crate) fn default_video_short_max_seconds() -> u64 {
    10 * 60
}

pub(crate) fn default_video_medium_max_seconds() -> u64 {
    35 * 60
}

pub(crate) fn default_shorts_short_max_seconds() -> u64 {
    30
}

pub(crate) fn default_shorts_medium_max_seconds() -> u64 {
    60
}

/// What a swipe on a video card does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SwipeActionKind {
    #[default]
    AddToPlaylist,
    AddToQueue,
    PlayNext,
    Share,
    MarkWatched,
}

impl SwipeActionKind {
    pub const ALL: [Self; 5] = [
        Self::AddToPlaylist,
        Self::AddToQueue,
        Self::PlayNext,
        Self::Share,
        Self::MarkWatched,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::AddToPlaylist => "Add to playlist",
            Self::AddToQueue => "Add to queue",
            Self::PlayNext => "Play next",
            Self::Share => "Share",
            Self::MarkWatched => "Mark watched",
        }
    }

    /// Compact text for the closed selector; the full label remains in the
    /// menu where it has room to explain the action.
    pub fn trigger_label(self) -> &'static str {
        match self {
            Self::AddToPlaylist => "Playlist",
            Self::AddToQueue => "Queue",
            Self::PlayNext => "Play next",
            Self::Share => "Share",
            Self::MarkWatched => "Watched",
        }
    }
}

impl AppSettings {
    /// The remembered speed for this kind of video.
    pub fn autoplay_for(&self, is_short: bool) -> bool {
        if is_short {
            self.shorts_autoplay
        } else {
            self.autoplay
        }
    }

    pub fn set_autoplay_for(&mut self, is_short: bool, autoplay: bool) {
        if is_short {
            self.shorts_autoplay = autoplay;
        } else {
            self.autoplay = autoplay;
        }
    }

    pub fn speed_for(&self, is_short: bool) -> f64 {
        if is_short {
            self.shorts_playback_speed
        } else {
            self.playback_speed
        }
    }

    pub fn set_speed_for(&mut self, is_short: bool, speed: f64) {
        if is_short {
            self.shorts_playback_speed = speed;
        } else {
            self.playback_speed = speed;
        }
    }
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            appearance: Appearance::Auto,
            platform_style: PlatformStyle::Auto,
            playback_speed: 1.0,
            shorts_playback_speed: 1.0,
            // Right plays next; left saves to Watch Later. The right
            // playlist only matters if that swipe is changed to "Add to
            // playlist", and Watch Later is the one every account has.
            swipe_right_action: SwipeActionKind::PlayNext,
            swipe_left_action: SwipeActionKind::AddToPlaylist,
            swipe_right_playlist_id: WATCH_LATER_PLAYLIST_ID.to_string(),
            swipe_left_playlist_id: WATCH_LATER_PLAYLIST_ID.to_string(),
            hide_watched: false,
            autoplay: true,
            shorts_autoplay: true,
            auto_landscape_fullscreen: true,
            theater_mode: false,
            video_short_max_seconds: default_video_short_max_seconds(),
            video_medium_max_seconds: default_video_medium_max_seconds(),
            shorts_short_max_seconds: default_shorts_short_max_seconds(),
            shorts_medium_max_seconds: default_shorts_medium_max_seconds(),
            sponsor_block: SponsorBlockSettings::default(),
            preferred_audio_language: default_audio_language(),
            preferred_caption_language: default_caption_language(),
            prefer_sabr: true,
            po_token_provider_url: None,
            playlist_views: BTreeMap::new(),
        }
    }
}

#[cfg(test)]
mod settings_tests {
    use super::AppSettings;

    #[test]
    fn audio_preference_starts_with_english() {
        assert_eq!(AppSettings::default().preferred_audio_language, "en");
    }
}
