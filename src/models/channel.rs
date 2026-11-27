use super::*;

/// Which of a channel's uploads a subscription wants in the feed: any mix of
/// its long-form videos, its Shorts and its livestreams.
///
/// A channel's Shorts and its long-form uploads are often two different shows,
/// and a channel that streams can bury both. This narrows a subscription
/// without unsubscribing. It is never empty: following nothing is an
/// unsubscribe, which is a different choice.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SubscriptionContent {
    pub videos: bool,
    pub shorts: bool,
    pub live: bool,
}

impl Default for SubscriptionContent {
    fn default() -> Self {
        Self::ALL
    }
}

impl SubscriptionContent {
    pub const ALL: Self = Self {
        videos: true,
        shorts: true,
        live: true,
    };
    /// What "Videos" used to mean, before each kind had its own toggle: not
    /// Shorts, so livestreams came too.
    pub const VIDEOS: Self = Self {
        videos: true,
        shorts: false,
        live: true,
    };
    pub const SHORTS: Self = Self {
        videos: false,
        shorts: true,
        live: false,
    };

    /// Stored, and sent, as a plain string so the column stays readable and an
    /// unknown value degrades to everything instead of failing the row. The
    /// three values from before the toggles keep their strings, so a server or
    /// an export from then reads them, and every other mix is spelled out.
    pub fn from_storage(value: &str) -> Self {
        let value = value.trim().to_ascii_lowercase();
        if let Some(kinds) = value.strip_prefix("only:") {
            let has = |kind: &str| kinds.split(',').any(|part| part.trim() == kind);
            let content = Self {
                videos: has("videos"),
                shorts: has("shorts"),
                live: has("live"),
            };
            return if content.is_empty() {
                Self::ALL
            } else {
                content
            };
        }
        match value.as_str() {
            "videos" => Self::VIDEOS,
            "shorts" => Self::SHORTS,
            _ => Self::ALL,
        }
    }

    pub fn as_storage(self) -> String {
        match self {
            Self::ALL => "all".into(),
            Self::VIDEOS => "videos".into(),
            Self::SHORTS => "shorts".into(),
            _ => {
                let kinds = [
                    (self.videos, "videos"),
                    (self.shorts, "shorts"),
                    (self.live, "live"),
                ]
                .into_iter()
                .filter_map(|(on, kind)| on.then_some(kind))
                .collect::<Vec<_>>();
                format!("only:{}", kinds.join(","))
            }
        }
    }

    pub fn is_empty(self) -> bool {
        !(self.videos || self.shorts || self.live)
    }

    /// What the feed shows, for a toast: "videos, Shorts and live".
    pub fn label(self) -> String {
        if self == Self::ALL {
            return "everything".into();
        }
        let kinds = [
            (self.videos, "videos"),
            (self.shorts, "Shorts"),
            (self.live, "live"),
        ]
        .into_iter()
        .filter_map(|(on, kind)| on.then_some(kind))
        .collect::<Vec<_>>();
        match kinds.as_slice() {
            [] => "nothing".into(),
            [only] => format!("{only} only"),
            [first, second] => format!("{first} and {second}"),
            _ => kinds.join(", "),
        }
    }
}

impl Serialize for SubscriptionContent {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.as_storage())
    }
}

impl<'de> Deserialize<'de> for SubscriptionContent {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Self::from_storage(&String::deserialize(deserializer)?))
    }
}

impl SurrealValue for SubscriptionContent {
    fn kind_of() -> surrealdb_types::Kind {
        String::kind_of()
    }

    fn is_value(value: &surrealdb_types::Value) -> bool {
        String::is_value(value)
    }

    fn into_value(self) -> surrealdb_types::Value {
        self.as_storage().into_value()
    }

    fn from_value(value: surrealdb_types::Value) -> Result<Self, surrealdb_types::Error> {
        String::from_value(value).map(|value| Self::from_storage(&value))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, SurrealValue)]
pub struct Channel {
    pub id: String,
    pub name: String,
    pub handle: String,
    pub avatar_url: Option<String>,
    pub subscriber_count: String,
    pub subscribed: bool,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub banner_url: Option<String>,
    #[serde(default)]
    pub subscription_content: SubscriptionContent,
}

impl Channel {
    #[cfg_attr(
        not(feature = "server"),
        allow(
            dead_code,
            reason = "Shared definitions are used by selected platform, server or test builds."
        )
    )]
    /// Merge metadata discovered by another YouTube surface without letting a
    /// sparse search result erase the richer channel record already cached.
    /// Subscription state is local state and is therefore never replaced.
    pub fn merge_metadata_from(&mut self, discovered: &Channel) -> bool {
        let before = self.clone();
        if !discovered.name.trim().is_empty() {
            self.name = discovered.name.clone();
        }
        if !discovered.handle.trim().is_empty() {
            self.handle = discovered.handle.clone();
        }
        if discovered.avatar_url.is_some() {
            self.avatar_url = discovered.avatar_url.clone();
        }
        if discovered.banner_url.is_some() {
            self.banner_url = discovered.banner_url.clone();
        }
        if !discovered.description.trim().is_empty() {
            self.description = discovered.description.clone();
        }

        let existing_count = compact_count_value(&self.subscriber_count);
        let discovered_count = compact_count_value(&discovered.subscriber_count);
        if !discovered.subscriber_count.trim().is_empty()
            && (self.subscriber_count.trim().is_empty()
                || discovered_count
                    .zip(existing_count)
                    .is_some_and(|(new, old)| new > old))
        {
            self.subscriber_count = discovered.subscriber_count.clone();
        }
        *self != before
    }
}

#[cfg_attr(
    not(feature = "server"),
    allow(
        dead_code,
        reason = "Shared definitions are used by selected platform, server or test builds."
    )
)]
pub(crate) fn compact_count_value(value: &str) -> Option<f64> {
    let trimmed = value.trim();
    let numeric = trimmed
        .chars()
        .take_while(|character| character.is_ascii_digit() || matches!(character, '.' | ','))
        .collect::<String>()
        .replace(',', "");
    let suffix = trimmed
        .chars()
        .skip_while(|character| character.is_ascii_digit() || matches!(character, '.' | ','))
        .find(|character| character.is_ascii_alphabetic());
    let multiplier = match suffix.map(|character| character.to_ascii_uppercase()) {
        Some('K') => 1_000.0,
        Some('M') => 1_000_000.0,
        Some('B') => 1_000_000_000.0,
        _ => 1.0,
    };
    numeric.parse::<f64>().ok().map(|count| count * multiplier)
}

#[cfg(test)]
mod channel_tests {
    use super::Channel;

    fn channel(count: &str) -> Channel {
        Channel {
            id: "UC-test".into(),
            name: "Veritasium".into(),
            handle: "@veritasium".into(),
            avatar_url: Some("rich-avatar".into()),
            subscriber_count: count.into(),
            subscribed: true,
            description: "Rich description".into(),
            banner_url: Some("rich-banner".into()),
            subscription_content: super::SubscriptionContent::ALL,
        }
    }

    #[test]
    fn sparse_channel_metadata_does_not_erase_richer_cache() {
        let mut cached = channel("21.1M");
        let mut sparse = channel("528");
        sparse.avatar_url = None;
        sparse.banner_url = None;
        sparse.description.clear();
        sparse.subscribed = false;

        cached.merge_metadata_from(&sparse);

        assert_eq!(cached.subscriber_count, "21.1M");
        assert_eq!(cached.avatar_url.as_deref(), Some("rich-avatar"));
        assert_eq!(cached.banner_url.as_deref(), Some("rich-banner"));
        assert_eq!(cached.description, "Rich description");
        assert!(cached.subscribed);
    }

    #[test]
    fn richer_channel_count_wins_even_with_a_label() {
        let mut cached = channel("528 subscribers");
        let discovered = channel("21.1M subscribers");
        cached.merge_metadata_from(&discovered);
        assert_eq!(cached.subscriber_count, "21.1M subscribers");
    }

    #[test]
    fn subscription_content_keeps_the_strings_older_servers_know() {
        use super::SubscriptionContent;
        // A 0.1.9 server stores and sends these three, with these meanings.
        assert_eq!(
            SubscriptionContent::from_storage("all"),
            SubscriptionContent::ALL
        );
        assert_eq!(
            SubscriptionContent::from_storage("videos"),
            SubscriptionContent::VIDEOS
        );
        assert_eq!(
            SubscriptionContent::from_storage("shorts"),
            SubscriptionContent::SHORTS
        );
        assert_eq!(SubscriptionContent::ALL.as_storage(), "all");
        assert_eq!(SubscriptionContent::VIDEOS.as_storage(), "videos");
        assert_eq!(SubscriptionContent::SHORTS.as_storage(), "shorts");
        // Anything unreadable still means everything rather than nothing.
        assert_eq!(
            SubscriptionContent::from_storage("only:"),
            SubscriptionContent::ALL
        );
        assert_eq!(
            SubscriptionContent::from_storage("???"),
            SubscriptionContent::ALL
        );
    }

    #[test]
    fn every_mix_of_kinds_round_trips() {
        use super::SubscriptionContent;
        for bits in 1..8u8 {
            let content = SubscriptionContent {
                videos: bits & 1 != 0,
                shorts: bits & 2 != 0,
                live: bits & 4 != 0,
            };
            assert_eq!(
                SubscriptionContent::from_storage(&content.as_storage()),
                content
            );
            let json = serde_json::to_string(&content).unwrap();
            assert_eq!(
                serde_json::from_str::<SubscriptionContent>(&json).unwrap(),
                content
            );
        }
        assert_eq!(
            SubscriptionContent {
                videos: true,
                shorts: false,
                live: false
            }
            .as_storage(),
            "only:videos"
        );
    }
}
