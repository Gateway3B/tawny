//! Reading and writing subscription lists in the formats other YouTube clients
//! actually hand out.
//!
//! Everything here is pure string work so it can run on the client, where the
//! file is picked, without dragging the server's XML and HTTP dependencies into
//! the WASM bundle. OPML and CSV are scanned rather than fully parsed: these are
//! machine-generated exports with a narrow shape, and a tolerant scanner beats
//! rejecting a whole file over one unexpected attribute.

use crate::models::{Channel, SubscriptionContent, SubscriptionGroup};
use serde::{Deserialize, Serialize};

/// One channel recovered from an export, before it is matched against the
/// library.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImportedSubscription {
    /// The `UC…` id. Empty when the export only identified the channel by
    /// handle, which Takeout and OPML never do but Piped occasionally does.
    pub channel_id: String,
    pub name: String,
    pub handle: String,
    pub content: SubscriptionContent,
}

impl ImportedSubscription {
    /// A record with neither an id nor a handle cannot be matched or fetched,
    /// so it is dropped rather than written as an unusable row.
    fn is_usable(&self) -> bool {
        !self.channel_id.is_empty() || !self.handle.is_empty()
    }
}

/// The formats this module can read. Detection is by content, not file
/// extension, because browsers and archive tools rename these freely.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubscriptionFormat {
    /// Tawny's own export: the only one carrying per-channel content
    /// preferences and subscription groups.
    Tawny,
    /// The `subscriptions.json` Piped, LibreTube, and NewPipe all read.
    Piped,
    /// The `subscriptions.csv` inside a Google Takeout archive.
    TakeoutCsv,
    /// Feed-reader OPML, as exported by NewPipe and YouTube's own RSS lists.
    Opml,
}

impl SubscriptionFormat {
    pub fn label(self) -> &'static str {
        match self {
            Self::Tawny => "Tawny",
            Self::Piped => "Piped / LibreTube",
            Self::TakeoutCsv => "Google Takeout",
            Self::Opml => "OPML",
        }
    }

    pub fn file_name(self) -> &'static str {
        match self {
            Self::Tawny => "tawny-subscriptions.json",
            Self::Piped => "subscriptions.json",
            Self::TakeoutCsv => "subscriptions.csv",
            Self::Opml => "subscriptions.opml",
        }
    }

    pub fn mime_type(self) -> &'static str {
        match self {
            Self::Tawny | Self::Piped => "application/json",
            Self::TakeoutCsv => "text/csv",
            Self::Opml => "text/x-opml",
        }
    }
}

/// Pull a channel id out of any of the URL shapes these exports use.
///
/// Handles `/channel/UC…`, the `channel_id` query parameter on an RSS feed URL,
/// and bare ids. Returns `None` for `/@handle` and `/c/vanity` URLs, which name
/// a channel without identifying it.
pub fn channel_id_from_url(url: &str) -> Option<String> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return None;
    }
    if is_channel_id(trimmed) {
        return Some(trimmed.to_string());
    }
    if let Some(index) = trimmed.find("channel_id=") {
        let value = &trimmed[index + "channel_id=".len()..];
        let value = value.split(['&', '#']).next().unwrap_or_default();
        if is_channel_id(value) {
            return Some(value.to_string());
        }
    }
    if let Some(index) = trimmed.find("/channel/") {
        let value = &trimmed[index + "/channel/".len()..];
        let value = value.split(['/', '?', '#']).next().unwrap_or_default();
        if is_channel_id(value) {
            return Some(value.to_string());
        }
    }
    None
}

/// Pull an `@handle` out of a channel URL, for exports that only offer one.
pub fn handle_from_url(url: &str) -> Option<String> {
    let trimmed = url.trim();
    let index = trimmed.find("/@")?;
    let value = &trimmed[index + 2..];
    let value = value.split(['/', '?', '#']).next().unwrap_or_default();
    if value.is_empty() {
        return None;
    }
    Some(format!("@{value}"))
}

/// YouTube channel ids are always `UC` plus 22 URL-safe base64 characters.
/// Checking the shape keeps a vanity path from being stored as an id.
fn is_channel_id(value: &str) -> bool {
    value.len() == 24
        && value.starts_with("UC")
        && value[2..]
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
}

/// The canonical watch URL for a channel, used by every export format.
fn channel_url(channel_id: &str, handle: &str) -> String {
    if channel_id.is_empty() {
        return format!("https://www.youtube.com/{handle}");
    }
    format!("https://www.youtube.com/channel/{channel_id}")
}

/// Piped, LibreTube, and NewPipe all emit this envelope. Only `subscriptions`
/// matters; `app` and `version` vary between them and are ignored.
#[derive(Debug, Deserialize)]
struct PipedExport {
    #[serde(default)]
    subscriptions: Vec<PipedSubscription>,
}

#[derive(Debug, Deserialize)]
struct PipedSubscription {
    #[serde(default)]
    url: String,
    #[serde(default)]
    name: String,
}

/// Tawny's own export. Round-trips the two things no third-party format can
/// carry: the per-channel Videos/Shorts preference, and subscription groups.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct TawnyExport {
    #[serde(default)]
    pub app: String,
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub subscriptions: Vec<TawnySubscription>,
    #[serde(default)]
    pub groups: Vec<SubscriptionGroup>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct TawnySubscription {
    #[serde(default)]
    pub channel_id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub handle: String,
    #[serde(default)]
    pub content: SubscriptionContent,
}

pub fn parse_piped_json(contents: &str) -> Result<Vec<ImportedSubscription>, String> {
    let export: PipedExport =
        serde_json::from_str(contents).map_err(|error| format!("not a Piped export: {error}"))?;
    Ok(export
        .subscriptions
        .into_iter()
        .map(|entry| ImportedSubscription {
            channel_id: channel_id_from_url(&entry.url).unwrap_or_default(),
            handle: handle_from_url(&entry.url).unwrap_or_default(),
            name: entry.name.trim().to_string(),
            content: SubscriptionContent::All,
        })
        .filter(ImportedSubscription::is_usable)
        .collect())
}

pub fn parse_tawny_json(
    contents: &str,
) -> Result<(Vec<ImportedSubscription>, Vec<SubscriptionGroup>), String> {
    let export: TawnyExport =
        serde_json::from_str(contents).map_err(|error| format!("not a Tawny export: {error}"))?;
    let subscriptions = export
        .subscriptions
        .into_iter()
        .map(|entry| ImportedSubscription {
            channel_id: entry.channel_id.trim().to_string(),
            handle: entry.handle.trim().to_string(),
            name: entry.name.trim().to_string(),
            content: entry.content,
        })
        .filter(ImportedSubscription::is_usable)
        .collect();
    Ok((subscriptions, export.groups))
}

/// Split one CSV record, honouring the doubled-quote escaping Takeout uses for
/// channel titles containing a comma.
fn split_csv_record(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut characters = line.chars().peekable();
    while let Some(character) = characters.next() {
        match character {
            '"' if quoted && characters.peek() == Some(&'"') => {
                current.push('"');
                characters.next();
            }
            '"' => quoted = !quoted,
            ',' if !quoted => fields.push(std::mem::take(&mut current)),
            _ => current.push(character),
        }
    }
    fields.push(current);
    fields
}

/// Google Takeout's `subscriptions.csv`.
///
/// The header is localized, so columns are located by shape rather than by
/// name: the id column is whichever one holds a `UC…` value. A header row is
/// simply a row where that never matches, and is skipped by the same rule.
pub fn parse_takeout_csv(contents: &str) -> Result<Vec<ImportedSubscription>, String> {
    let mut subscriptions = Vec::new();
    for line in contents.lines() {
        let line = line.trim_end_matches('\r');
        if line.trim().is_empty() {
            continue;
        }
        let fields = split_csv_record(line);
        let Some(channel_id) = fields.iter().find_map(|field| channel_id_from_url(field)) else {
            continue;
        };
        // The title is the last field that is neither the id nor a URL, which
        // survives the column reordering seen between Takeout versions.
        let name = fields
            .iter()
            .rev()
            .find(|field| {
                let value = field.trim();
                !value.is_empty() && value != channel_id && !value.contains("://")
            })
            .map(|field| field.trim().to_string())
            .unwrap_or_default();
        subscriptions.push(ImportedSubscription {
            channel_id,
            name,
            handle: String::new(),
            content: SubscriptionContent::All,
        });
    }
    if subscriptions.is_empty() {
        return Err("no channel ids found in this CSV".into());
    }
    Ok(subscriptions)
}

/// Read one XML attribute out of an element's text.
fn xml_attribute(element: &str, name: &str) -> Option<String> {
    let needle = format!("{name}=\"");
    let start = element.find(&needle)? + needle.len();
    let rest = &element[start..];
    let end = rest.find('"')?;
    Some(decode_xml_entities(&rest[..end]))
}

fn decode_xml_entities(value: &str) -> String {
    value
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        // Ampersand last: decoding it first would let `&amp;lt;` become `<`.
        .replace("&amp;", "&")
}

fn encode_xml_entities(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Feed-reader OPML, where each subscription is an `<outline>` whose `xmlUrl`
/// is the channel's RSS feed.
pub fn parse_opml(contents: &str) -> Result<Vec<ImportedSubscription>, String> {
    let mut subscriptions = Vec::new();
    for chunk in contents.split("<outline").skip(1) {
        let element = chunk.split('>').next().unwrap_or_default();
        let url = xml_attribute(element, "xmlUrl").unwrap_or_default();
        let Some(channel_id) = channel_id_from_url(&url) else {
            continue;
        };
        let name = xml_attribute(element, "title")
            .or_else(|| xml_attribute(element, "text"))
            .unwrap_or_default();
        subscriptions.push(ImportedSubscription {
            channel_id,
            name: name.trim().to_string(),
            handle: String::new(),
            content: SubscriptionContent::All,
        });
    }
    if subscriptions.is_empty() {
        return Err("no channel feeds found in this OPML".into());
    }
    Ok(subscriptions)
}

/// What an import actually did, so the confirmation can be specific rather than
/// just "imported".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ImportSummary {
    /// Channels newly subscribed, whether or not they were already cached.
    pub subscribed: usize,
    /// Channels the export listed that were already subscribed.
    pub already_subscribed: usize,
    /// Entries naming a channel only by handle, which cannot be resolved
    /// without a network lookup.
    pub unresolved: usize,
    pub groups: usize,
}

impl ImportSummary {
    pub fn changed(self) -> bool {
        self.subscribed > 0 || self.groups > 0 || self.already_subscribed > 0
    }

    /// A one-line description for the toast.
    pub fn message(self) -> String {
        if self.subscribed == 0 && self.already_subscribed == 0 {
            return "No subscriptions found in that file".into();
        }
        let mut message = if self.subscribed == 1 {
            "Subscribed to 1 channel".to_string()
        } else {
            format!("Subscribed to {} channels", self.subscribed)
        };
        if self.already_subscribed > 0 {
            message.push_str(&format!(", {} already saved", self.already_subscribed));
        }
        if self.groups > 0 {
            message.push_str(&format!(", {} groups", self.groups));
        }
        if self.unresolved > 0 {
            message.push_str(&format!(", {} could not be identified", self.unresolved));
        }
        message
    }
}

/// What a successfully-read file turned out to contain.
pub struct ParsedImport {
    pub format: SubscriptionFormat,
    pub subscriptions: Vec<ImportedSubscription>,
    /// Only ever non-empty for Tawny's own export.
    pub groups: Vec<SubscriptionGroup>,
}

/// Sniff the format from the content and parse it.
///
/// Detection is by content rather than file name: Takeout hands out a `.csv`
/// inside a `.zip`, browsers rename duplicates, and LibreTube and Piped both
/// use the same generic `subscriptions.json`.
pub fn parse_subscription_export(contents: &str) -> Result<ParsedImport, String> {
    let trimmed = contents.trim_start_matches('\u{feff}').trim();
    if trimmed.is_empty() {
        return Err("that file is empty".into());
    }

    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        // Tawny's own export is a superset of Piped's, so it has to be tried
        // first or its content preferences would be silently dropped.
        if trimmed.contains("\"channel_id\"") || trimmed.contains("\"groups\"") {
            let (subscriptions, groups) = parse_tawny_json(trimmed)?;
            if !subscriptions.is_empty() {
                return Ok(ParsedImport {
                    format: SubscriptionFormat::Tawny,
                    subscriptions,
                    groups,
                });
            }
        }
        let subscriptions = parse_piped_json(trimmed)?;
        if subscriptions.is_empty() {
            return Err("that export lists no subscriptions".into());
        }
        return Ok(ParsedImport {
            format: SubscriptionFormat::Piped,
            subscriptions,
            groups: Vec::new(),
        });
    }

    if trimmed.starts_with('<') {
        return Ok(ParsedImport {
            format: SubscriptionFormat::Opml,
            subscriptions: parse_opml(trimmed)?,
            groups: Vec::new(),
        });
    }

    Ok(ParsedImport {
        format: SubscriptionFormat::TakeoutCsv,
        subscriptions: parse_takeout_csv(trimmed)?,
        groups: Vec::new(),
    })
}

/// Render the subscribed channels in `channels` into `format`.
pub fn export_subscriptions(
    format: SubscriptionFormat,
    channels: &[Channel],
    groups: &[SubscriptionGroup],
) -> String {
    let subscribed = channels
        .iter()
        .filter(|channel| channel.subscribed)
        .collect::<Vec<_>>();
    match format {
        SubscriptionFormat::Tawny => export_tawny_json(&subscribed, groups),
        SubscriptionFormat::Piped => export_piped_json(&subscribed),
        SubscriptionFormat::TakeoutCsv => export_takeout_csv(&subscribed),
        SubscriptionFormat::Opml => export_opml(&subscribed),
    }
}

fn export_tawny_json(channels: &[&Channel], groups: &[SubscriptionGroup]) -> String {
    let export = TawnyExport {
        app: "Tawny".into(),
        version: 1,
        subscriptions: channels
            .iter()
            .map(|channel| TawnySubscription {
                channel_id: channel.id.clone(),
                name: channel.name.clone(),
                handle: channel.handle.clone(),
                content: channel.subscription_content,
            })
            .collect(),
        // Only groups that still reference a subscribed channel are worth
        // carrying across.
        groups: groups
            .iter()
            .filter(|group| {
                group
                    .channel_ids
                    .iter()
                    .any(|id| channels.iter().any(|channel| &channel.id == id))
            })
            .cloned()
            .collect(),
    };
    serde_json::to_string_pretty(&export).unwrap_or_else(|_| "{}".into())
}

fn export_piped_json(channels: &[&Channel]) -> String {
    let entries = channels
        .iter()
        .map(|channel| {
            serde_json::json!({
                "service_id": 0,
                "url": channel_url(&channel.id, &channel.handle),
                "name": channel.name,
            })
        })
        .collect::<Vec<_>>();
    let export = serde_json::json!({
        "app": "Tawny",
        "version": "1",
        "subscriptions": entries,
    });
    serde_json::to_string_pretty(&export).unwrap_or_else(|_| "{}".into())
}

fn export_takeout_csv(channels: &[&Channel]) -> String {
    let mut out = String::from("Channel Id,Channel Url,Channel Title\n");
    for channel in channels {
        let title = if channel.name.contains([',', '"']) {
            format!("\"{}\"", channel.name.replace('"', "\"\""))
        } else {
            channel.name.clone()
        };
        out.push_str(&format!(
            "{},{},{}\n",
            channel.id,
            channel_url(&channel.id, &channel.handle),
            title
        ));
    }
    out
}

fn export_opml(channels: &[&Channel]) -> String {
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<opml version=\"1.1\">\n  <head>\n    <title>Tawny subscriptions</title>\n  </head>\n  <body>\n",
    );
    for channel in channels {
        let name = encode_xml_entities(&channel.name);
        out.push_str(&format!(
            "    <outline text=\"{name}\" title=\"{name}\" type=\"rss\" xmlUrl=\"https://www.youtube.com/feeds/videos.xml?channel_id={}\" />\n",
            channel.id
        ));
    }
    out.push_str("  </body>\n</opml>\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channel(id: &str, name: &str, content: SubscriptionContent) -> Channel {
        Channel {
            id: id.into(),
            name: name.into(),
            handle: "@example".into(),
            avatar_url: None,
            subscriber_count: String::new(),
            subscribed: true,
            description: String::new(),
            banner_url: None,
            subscription_content: content,
        }
    }

    const ID_A: &str = "UCsXVk37bltHxD1rDPwtNM8Q";

    const ID_B: &str = "UC_x5XG1OV2P6uZZ5FSM9Ttw";

    #[test]
    fn reads_channel_ids_from_every_url_shape() {
        assert_eq!(
            channel_id_from_url(&format!("https://www.youtube.com/channel/{ID_A}")).as_deref(),
            Some(ID_A)
        );
        assert_eq!(
            channel_id_from_url(&format!("/channel/{ID_A}")).as_deref(),
            Some(ID_A)
        );
        assert_eq!(
            channel_id_from_url(&format!(
                "https://www.youtube.com/feeds/videos.xml?channel_id={ID_A}"
            ))
            .as_deref(),
            Some(ID_A)
        );
        assert_eq!(channel_id_from_url(ID_A).as_deref(), Some(ID_A));
    }

    /// A vanity or handle URL names a channel without identifying it, and must
    /// not be stored as though it were an id.
    #[test]
    fn rejects_urls_that_carry_no_channel_id() {
        assert_eq!(
            channel_id_from_url("https://www.youtube.com/@somebody"),
            None
        );
        assert_eq!(
            channel_id_from_url("https://www.youtube.com/c/Vanity"),
            None
        );
        assert_eq!(channel_id_from_url(""), None);
        assert_eq!(
            handle_from_url("https://www.youtube.com/@somebody").as_deref(),
            Some("@somebody")
        );
    }

    #[test]
    fn reads_a_piped_export() {
        let contents = format!(
            r#"{{"app":"Piped","version":"1","subscriptions":[
                {{"service_id":0,"url":"https://www.youtube.com/channel/{ID_A}","name":"First"}},
                {{"service_id":0,"url":"/channel/{ID_B}","name":"Second"}}
            ]}}"#
        );
        let parsed = parse_subscription_export(&contents).expect("piped export");
        assert_eq!(parsed.format, SubscriptionFormat::Piped);
        assert_eq!(parsed.subscriptions.len(), 2);
        assert_eq!(parsed.subscriptions[0].channel_id, ID_A);
        assert_eq!(parsed.subscriptions[1].name, "Second");
    }

    /// Takeout localizes its header row, so the parser locates columns by shape
    /// and the header is skipped because no field looks like an id.
    #[test]
    fn reads_a_takeout_csv_and_skips_its_header() {
        let contents = format!(
            "Channel Id,Channel Url,Channel Title\n\
             {ID_A},http://www.youtube.com/channel/{ID_A},Plain Name\n\
             {ID_B},http://www.youtube.com/channel/{ID_B},\"Name, with comma\"\n"
        );
        let parsed = parse_subscription_export(&contents).expect("takeout csv");
        assert_eq!(parsed.format, SubscriptionFormat::TakeoutCsv);
        assert_eq!(parsed.subscriptions.len(), 2);
        assert_eq!(parsed.subscriptions[0].name, "Plain Name");
        assert_eq!(parsed.subscriptions[1].name, "Name, with comma");
    }

    #[test]
    fn reads_an_opml_export() {
        let contents = format!(
            r#"<?xml version="1.0"?><opml version="1.1"><body>
            <outline text="Alpha &amp; Co" title="Alpha &amp; Co" type="rss"
                xmlUrl="https://www.youtube.com/feeds/videos.xml?channel_id={ID_A}" />
            <outline text="Beta" type="rss"
                xmlUrl="https://www.youtube.com/feeds/videos.xml?channel_id={ID_B}" />
            </body></opml>"#
        );
        let parsed = parse_subscription_export(&contents).expect("opml");
        assert_eq!(parsed.format, SubscriptionFormat::Opml);
        assert_eq!(parsed.subscriptions.len(), 2);
        assert_eq!(parsed.subscriptions[0].name, "Alpha & Co");
        assert_eq!(parsed.subscriptions[1].name, "Beta");
    }

    /// The whole point of the native format: content preferences and groups
    /// survive a round trip, which no third-party format can manage.
    #[test]
    fn tawny_export_round_trips_preferences_and_groups() {
        let channels = vec![
            channel(ID_A, "Shorts Only", SubscriptionContent::Shorts),
            channel(ID_B, "Videos Only", SubscriptionContent::Videos),
        ];
        let groups = vec![SubscriptionGroup {
            id: "group:tech".into(),
            name: "Tech".into(),
            channel_ids: vec![ID_A.into()],
        }];

        let rendered = export_subscriptions(SubscriptionFormat::Tawny, &channels, &groups);
        let parsed = parse_subscription_export(&rendered).expect("tawny export");

        assert_eq!(parsed.format, SubscriptionFormat::Tawny);
        assert_eq!(parsed.subscriptions.len(), 2);
        assert_eq!(parsed.subscriptions[0].content, SubscriptionContent::Shorts);
        assert_eq!(parsed.subscriptions[1].content, SubscriptionContent::Videos);
        assert_eq!(parsed.groups, groups);
    }

    /// Every third-party format has to survive its own round trip too, even
    /// though they all flatten the content preference to "everything".
    #[test]
    fn third_party_formats_round_trip_their_channels() {
        let channels = vec![
            channel(ID_A, "Alpha & Co", SubscriptionContent::Shorts),
            channel(ID_B, "Name, with comma", SubscriptionContent::All),
        ];
        for format in [
            SubscriptionFormat::Piped,
            SubscriptionFormat::TakeoutCsv,
            SubscriptionFormat::Opml,
        ] {
            let rendered = export_subscriptions(format, &channels, &[]);
            let parsed = parse_subscription_export(&rendered)
                .unwrap_or_else(|error| panic!("{}: {error}", format.label()));
            assert_eq!(parsed.format, format, "{}", format.label());
            let ids = parsed
                .subscriptions
                .iter()
                .map(|entry| entry.channel_id.as_str())
                .collect::<Vec<_>>();
            assert_eq!(ids, vec![ID_A, ID_B], "{}", format.label());
            assert_eq!(
                parsed.subscriptions[0].name,
                "Alpha & Co",
                "{}",
                format.label()
            );
        }
    }

    /// Unsubscribed channels are library cache, not subscriptions.
    #[test]
    fn export_covers_only_subscribed_channels() {
        let mut channels = vec![
            channel(ID_A, "Kept", SubscriptionContent::All),
            channel(ID_B, "Dropped", SubscriptionContent::All),
        ];
        channels[1].subscribed = false;

        let rendered = export_subscriptions(SubscriptionFormat::Piped, &channels, &[]);
        let parsed = parse_subscription_export(&rendered).expect("piped export");
        assert_eq!(parsed.subscriptions.len(), 1);
        assert_eq!(parsed.subscriptions[0].channel_id, ID_A);
    }

    #[test]
    fn reports_unreadable_files_rather_than_importing_nothing() {
        assert!(parse_subscription_export("   ").is_err());
        assert!(parse_subscription_export("not,a,subscription,file").is_err());
        assert!(parse_subscription_export("{\"subscriptions\":[]}").is_err());
    }
}
