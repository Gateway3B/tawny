use super::*;

/// A community-submitted category of segment.
///
/// The names on the wire are SponsorBlock's own, and the colours are the ones
/// its extension uses - a viewer who already knows what a green bar means
/// should not have to relearn it here.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SponsorCategory {
    Sponsor,
    #[serde(rename = "selfpromo")]
    SelfPromo,
    Interaction,
    Intro,
    Outro,
    Preview,
    #[serde(rename = "music_offtopic")]
    MusicOfftopic,
    Filler,
    #[serde(rename = "poi_highlight")]
    Highlight,
}

#[cfg_attr(
    not(feature = "server"),
    allow(
        dead_code,
        reason = "Shared definitions are used by selected platform, server or test builds."
    )
)]
impl SponsorCategory {
    /// Every category Tawny asks the API for, in the order the settings list
    /// them.
    pub const ALL: [Self; 9] = [
        Self::Sponsor,
        Self::SelfPromo,
        Self::Interaction,
        Self::Intro,
        Self::Outro,
        Self::Preview,
        Self::MusicOfftopic,
        Self::Filler,
        Self::Highlight,
    ];

    pub fn api_name(self) -> &'static str {
        match self {
            Self::Sponsor => "sponsor",
            Self::SelfPromo => "selfpromo",
            Self::Interaction => "interaction",
            Self::Intro => "intro",
            Self::Outro => "outro",
            Self::Preview => "preview",
            Self::MusicOfftopic => "music_offtopic",
            Self::Filler => "filler",
            Self::Highlight => "poi_highlight",
        }
    }

    pub fn from_api_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|category| category.api_name() == name)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Sponsor => "Sponsor",
            Self::SelfPromo => "Unpaid self-promotion",
            Self::Interaction => "Interaction reminder",
            Self::Intro => "Intermission / intro",
            Self::Outro => "Endcards / credits",
            Self::Preview => "Preview / recap",
            Self::MusicOfftopic => "Non-music section",
            Self::Filler => "Filler tangent",
            Self::Highlight => "Highlight",
        }
    }

    /// What the category actually covers.
    ///
    /// The names alone do not settle it - "Preview" and "Filler" in particular
    /// are guessable in several directions, and choosing an action for one you
    /// have misread is how a viewer ends up skipping the video itself.
    pub fn description(self) -> &'static str {
        match self {
            Self::Sponsor => "Paid promotion, referral codes, and direct advertising.",
            Self::SelfPromo => {
                "Unpaid plugs for the creator's own merch, Patreon, or other channels."
            }
            Self::Interaction => "Brief reminders to like, subscribe, or comment.",
            Self::Intro => "Title cards and animated openers with no content in them.",
            Self::Outro => "Endcards and credits, where the video is effectively over.",
            Self::Preview => {
                "A recap of this video, or a run-through of what is coming later in it."
            }
            Self::MusicOfftopic => "The parts of a music video that are not the music.",
            Self::Filler => "Tangents and jokes the creator added that are not the subject.",
            Self::Highlight => "The moment the video is actually about. Jumped to, never over.",
        }
    }

    /// What a skip toast says once it has happened.
    pub fn skip_message(self) -> &'static str {
        match self {
            Self::Sponsor => "Skipped sponsor",
            Self::SelfPromo => "Skipped self-promotion",
            Self::Interaction => "Skipped interaction reminder",
            Self::Intro => "Skipped intro",
            Self::Outro => "Skipped endcards",
            Self::Preview => "Skipped recap",
            Self::MusicOfftopic => "Skipped non-music section",
            Self::Filler => "Skipped filler",
            Self::Highlight => "Jumped to the highlight",
        }
    }

    /// SponsorBlock's own palette, so the timeline reads the same as the
    /// extension's does.
    pub fn color(self) -> &'static str {
        match self {
            Self::Sponsor => "#00d400",
            Self::SelfPromo => "#ffff00",
            Self::Interaction => "#cc00ff",
            Self::Intro => "#00ffff",
            Self::Outro => "#0202ed",
            Self::Preview => "#008fd6",
            Self::MusicOfftopic => "#ff9900",
            Self::Filler => "#7300ff",
            Self::Highlight => "#ff1684",
        }
    }

    /// A highlight is a point, not a stretch: skipping *to* it is the whole
    /// action, so it is never skipped over and never auto-applied.
    pub fn is_point(self) -> bool {
        matches!(self, Self::Highlight)
    }

    /// What a fresh install does with each category.
    ///
    /// Matches the extension's defaults: the categories nobody wants are
    /// skipped, the ones that are sometimes content are offered, and the rest
    /// stay off until asked for.
    pub fn default_action(self) -> SponsorAction {
        match self {
            Self::Sponsor | Self::SelfPromo | Self::Interaction => SponsorAction::Skip,
            Self::Intro | Self::Outro | Self::Preview | Self::MusicOfftopic => SponsorAction::Show,
            Self::Filler | Self::Highlight => SponsorAction::Off,
        }
    }
}

/// What to do when playback reaches a segment.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SponsorAction {
    /// Jump past it, and say so.
    Skip,
    /// Mark it on the timeline and offer a button, but do not move the
    /// playhead. The right default for anything that is sometimes the video.
    #[default]
    Show,
    /// Not fetched, not drawn, not skipped.
    Off,
}

impl SponsorAction {
    pub const ALL: [Self; 3] = [Self::Skip, Self::Show, Self::Off];

    pub fn label(self) -> &'static str {
        match self {
            Self::Skip => "Skip automatically",
            Self::Show => "Show on the timeline",
            Self::Off => "Ignore",
        }
    }

    pub fn trigger_label(self) -> &'static str {
        match self {
            Self::Skip => "Skip",
            Self::Show => "Show",
            Self::Off => "Off",
        }
    }
}

/// One segment of one video.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SponsorSegment {
    pub uuid: String,
    pub category: SponsorCategory,
    pub start_seconds: f64,
    pub end_seconds: f64,
    /// SponsorBlock's `locked` flag: a segment vetted by a moderator. Kept
    /// because it is the tie-breaker when two submissions overlap.
    #[serde(default)]
    pub locked: bool,
    #[serde(default)]
    pub votes: i64,
}

/// Per-category behaviour, stored as a list rather than a map so it serialises
/// stably and an unknown category from a future build round-trips untouched.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SponsorCategorySetting {
    pub category: SponsorCategory,
    pub action: SponsorAction,
}

/// Everything the viewer controls about SponsorBlock.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SponsorBlockSettings {
    pub enabled: bool,
    /// Whether a skip announces itself. On by default: a video that silently
    /// jumps looks broken until you know why.
    pub notify_on_skip: bool,
    pub categories: Vec<SponsorCategorySetting>,
}

impl Default for SponsorBlockSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            notify_on_skip: true,
            categories: SponsorCategory::ALL
                .into_iter()
                .map(|category| SponsorCategorySetting {
                    category,
                    action: category.default_action(),
                })
                .collect(),
        }
    }
}

impl SponsorBlockSettings {
    pub fn action_for(&self, category: SponsorCategory) -> SponsorAction {
        if !self.enabled {
            return SponsorAction::Off;
        }
        self.categories
            .iter()
            .find(|setting| setting.category == category)
            .map(|setting| setting.action)
            // A category this build knows but the stored settings predate.
            .unwrap_or_else(|| category.default_action())
    }

    pub fn set_action(&mut self, category: SponsorCategory, action: SponsorAction) {
        match self
            .categories
            .iter_mut()
            .find(|setting| setting.category == category)
        {
            Some(setting) => setting.action = action,
            None => self
                .categories
                .push(SponsorCategorySetting { category, action }),
        }
    }

    /// The categories worth asking the API for. Requesting the ignored ones
    /// would mean fetching data only to discard it.
    pub fn requested_categories(&self) -> Vec<SponsorCategory> {
        SponsorCategory::ALL
            .into_iter()
            .filter(|category| self.action_for(*category) != SponsorAction::Off)
            .collect()
    }
}

#[cfg(test)]
mod sponsor_tests {
    use super::*;

    #[test]
    fn api_names_round_trip() {
        for category in SponsorCategory::ALL {
            assert_eq!(
                SponsorCategory::from_api_name(category.api_name()),
                Some(category),
                "{category:?}"
            );
        }
        assert_eq!(SponsorCategory::from_api_name("not_a_category"), None);
    }

    #[test]
    fn the_wire_format_matches_sponsorblocks_own_names() {
        // These strings go to sponsor.ajay.app and come back from it, so they
        // are a contract with someone else's API rather than an internal name.
        let encoded = serde_json::to_string(&SponsorCategory::MusicOfftopic).unwrap();
        assert_eq!(encoded, "\"music_offtopic\"");
        assert_eq!(
            serde_json::from_str::<SponsorCategory>("\"poi_highlight\"").unwrap(),
            SponsorCategory::Highlight
        );
        assert_eq!(
            serde_json::from_str::<SponsorCategory>("\"selfpromo\"").unwrap(),
            SponsorCategory::SelfPromo
        );
    }

    #[test]
    fn disabling_sponsorblock_turns_every_category_off() {
        let mut settings = SponsorBlockSettings::default();
        assert_eq!(
            settings.action_for(SponsorCategory::Sponsor),
            SponsorAction::Skip
        );
        settings.enabled = false;
        for category in SponsorCategory::ALL {
            assert_eq!(settings.action_for(category), SponsorAction::Off);
        }
        assert!(settings.requested_categories().is_empty());
    }

    #[test]
    fn only_the_categories_in_use_are_requested() {
        let mut settings = SponsorBlockSettings::default();
        settings.set_action(SponsorCategory::Sponsor, SponsorAction::Off);
        let requested = settings.requested_categories();
        assert!(!requested.contains(&SponsorCategory::Sponsor));
        assert!(requested.contains(&SponsorCategory::Intro));
    }

    #[test]
    fn a_category_missing_from_stored_settings_falls_back_to_its_default() {
        // Settings written by a build that predates a category must not make it
        // silently Off - that would look like SponsorBlock ignoring it.
        let settings = SponsorBlockSettings {
            enabled: true,
            notify_on_skip: true,
            categories: Vec::new(),
        };
        assert_eq!(
            settings.action_for(SponsorCategory::Sponsor),
            SponsorAction::Skip
        );
        assert_eq!(
            settings.action_for(SponsorCategory::Filler),
            SponsorAction::Off
        );
    }

    #[test]
    fn a_highlight_is_a_point_and_nothing_else_is() {
        for category in SponsorCategory::ALL {
            assert_eq!(
                category.is_point(),
                category == SponsorCategory::Highlight,
                "{category:?}"
            );
        }
    }
}
