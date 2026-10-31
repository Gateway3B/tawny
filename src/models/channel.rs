use super::*;

/// Which of a channel's uploads a subscription actually wants in the feed.
///
/// A channel's Shorts and its long-form uploads are often two different shows,
/// and subscribing is currently all-or-nothing. This narrows a subscription
/// without unsubscribing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, SurrealValue)]
#[serde(rename_all = "kebab-case")]
pub enum SubscriptionContent {
    /// Everything the channel publishes, including livestreams.
    #[default]
    All,
    /// Long-form uploads only; Shorts are dropped from the feed.
    Videos,
    /// Shorts only.
    Shorts,
}

impl SubscriptionContent {
    /// SurrealDB stores this as a plain string so the column stays readable and
    /// an unknown value degrades to "everything" instead of failing the row.
    /// Only the server reads and writes it.
    #[cfg_attr(
        not(feature = "server"),
        allow(
            dead_code,
            reason = "Shared definitions are used by selected platform, server or test builds."
        )
    )]
    pub fn from_storage(value: &str) -> Self {
        match value {
            "videos" => Self::Videos,
            "shorts" => Self::Shorts,
            _ => Self::All,
        }
    }

    #[cfg_attr(
        not(feature = "server"),
        allow(
            dead_code,
            reason = "Shared definitions are used by selected platform, server or test builds."
        )
    )]
    pub fn as_storage(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Videos => "videos",
            Self::Shorts => "shorts",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "Videos & Shorts",
            Self::Videos => "Videos only",
            Self::Shorts => "Shorts only",
        }
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
            subscription_content: super::SubscriptionContent::All,
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
}
