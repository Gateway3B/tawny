use super::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum FeedFilter {
    #[default]
    All,
    Videos,
    Shorts,
    Live,
}

impl FeedFilter {
    pub const ALL: [Self; 4] = [Self::All, Self::Videos, Self::Shorts, Self::Live];

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Videos => "Videos",
            Self::Shorts => "Shorts",
            Self::Live => "Live",
        }
    }
}

/// Which list the search page shows with nothing typed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DiscoverList {
    /// What YouTube recommended beside recent watches.
    #[default]
    Recommended,
    /// The newest unwatched uploads from followed channels.
    Following,
}

impl DiscoverList {
    pub const ALL: [Self; 2] = [Self::Recommended, Self::Following];

    pub fn label(self) -> &'static str {
        match self {
            Self::Recommended => "Recommended",
            Self::Following => "Following",
        }
    }
}

/// What a search looks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExploreFilter {
    All,
    Videos,
    Channels,
}

impl ExploreFilter {
    pub const ALL: [Self; 3] = [Self::All, Self::Videos, Self::Channels];

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Videos => "Videos",
            Self::Channels => "Channels",
        }
    }

    /// The server's name for the filter.
    pub fn query(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Videos => "videos",
            Self::Channels => "channels",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DurationFilter {
    Short,
    Medium,
    Long,
}

impl DurationFilter {
    pub const ALL: [Self; 3] = [Self::Short, Self::Medium, Self::Long];

    pub fn label(self) -> &'static str {
        match self {
            Self::Short => "Short",
            Self::Medium => "Medium",
            Self::Long => "Long",
        }
    }

    pub fn matches(self, video: &Video, settings: &AppSettings) -> bool {
        if video.is_live || video.duration_seconds == 0 {
            return false;
        }
        let (short_max, medium_max) = if video.is_short {
            (
                settings.shorts_short_max_seconds,
                settings.shorts_medium_max_seconds,
            )
        } else {
            (
                settings.video_short_max_seconds,
                settings.video_medium_max_seconds,
            )
        };
        match self {
            Self::Short => video.duration_seconds <= short_max,
            Self::Medium => {
                video.duration_seconds > short_max && video.duration_seconds <= medium_max
            }
            Self::Long => video.duration_seconds > medium_max,
        }
    }
}
