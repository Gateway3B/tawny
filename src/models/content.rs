use super::*;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, SurrealValue)]
pub struct Playlist {
    pub id: String,
    pub name: String,
    pub video_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, SurrealValue)]
pub struct SubscriptionGroup {
    pub id: String,
    pub name: String,
    pub channel_ids: Vec<String>,
}

/// A followed channel that looks like it belongs in a group, from
/// `suggest_group_channels`. The client already has the channel itself in
/// the viewer, so only the id travels.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupSuggestion {
    pub channel_id: String,
    /// A clear match, offered already ticked. The rest are worth a look.
    pub likely: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchResults {
    pub query: String,
    pub videos: Vec<Video>,
    pub channels: Vec<Channel>,
    pub suggestion: Option<String>,
    pub remote_available: bool,
    #[serde(default)]
    pub next_page: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChannelMediaTab {
    #[default]
    Videos,
    Shorts,
    Live,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelMediaPage {
    pub channel_id: String,
    pub tab: ChannelMediaTab,
    pub videos: Vec<Video>,
    pub next_page: Option<String>,
    pub source: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelDetails {
    pub channel: Channel,
    pub videos: ChannelMediaPage,
    pub shorts: ChannelMediaPage,
    pub live: ChannelMediaPage,
    pub remote_available: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FeedRefreshResult {
    pub imported: usize,
    pub refreshed_channels: usize,
    pub failed_channels: usize,
    pub sources: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoChapter {
    pub title: String,
    pub start_seconds: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoPreviewFrames {
    pub page_urls: Vec<String>,
    pub frame_width: u32,
    pub frame_height: u32,
    pub total_count: u32,
    pub duration_per_frame_ms: u32,
    pub frames_per_page_x: u32,
    pub frames_per_page_y: u32,
}

impl VideoChapter {
    pub fn timestamp_label(&self) -> String {
        let hours = self.start_seconds / 3600;
        let minutes = (self.start_seconds % 3600) / 60;
        let seconds = self.start_seconds % 60;
        if hours > 0 {
            format!("{hours}:{minutes:02}:{seconds:02}")
        } else {
            format!("{minutes}:{seconds:02}")
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptionTrack {
    pub label: String,
    pub language_code: String,
    pub mime_type: String,
    pub url: String,
    pub auto_generated: bool,
}

/// Which of `tracks` a video opens with for a viewer who prefers `language`.
///
/// A written track beats YouTube's automatic one, which is a speech
/// recognition guess. Regional variants count as the language (`en-GB` for
/// `en`), since a viewer choosing a language is rarely choosing a country.
/// With nothing in the language, the first track, as before.
pub fn preferred_caption_index(tracks: &[CaptionTrack], language: &str) -> Option<usize> {
    let wanted = primary_language(language);
    let speaks = |track: &CaptionTrack| primary_language(&track.language_code) == wanted;
    tracks
        .iter()
        .position(|track| speaks(track) && !track.auto_generated)
        .or_else(|| tracks.iter().position(speaks))
        .or_else(|| (!tracks.is_empty()).then_some(0))
}

fn primary_language(tag: &str) -> String {
    tag.split(['-', '_'])
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
}

/// One selectable audio language for the video being watched.
///
/// Reported by the transport rather than the extractor: Shaka is the authority
/// on which of the manifest's audio adaptations it can actually decode and
/// switch between, and it carries the display names.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioTrackOption {
    pub language: String,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoComment {
    pub id: String,
    pub author: String,
    pub author_avatar_url: Option<String>,
    pub author_channel_id: Option<String>,
    pub text: String,
    pub published_at: String,
    pub like_count: u64,
    pub reply_count: u64,
    pub pinned: bool,
    pub hearted: bool,
    pub creator: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommentsPage {
    pub comments: Vec<VideoComment>,
    pub next_page: Option<String>,
    pub disabled: bool,
    pub remote_available: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoDetails {
    pub video: Video,
    pub channel: Option<Channel>,
    pub description: String,
    pub like_count: u64,
    pub dislike_count: u64,
    pub captions: Vec<CaptionTrack>,
    pub chapters: Vec<VideoChapter>,
    #[serde(default)]
    pub preview_frames: Option<VideoPreviewFrames>,
    pub related_videos: Vec<Video>,
    pub comments: CommentsPage,
    pub remote_available: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub video_id: String,
    pub played_at: String,
}
