//! Videos kept on this device to watch without a connection.
//!
//! The downloads themselves live in the page (`assets/tawny_downloads.js`),
//! in Shaka's offline storage: that is the only place the player can read
//! them back from, and a native build's Rust handlers block the WebView. This
//! module starts and removes them, mirrors the list into [`AppState`], and
//! hands the player a session for a kept video instead of asking the server.

use crate::{
    models::{PlaybackProtocol, PlaybackSession, PlaybackSource, Video},
    playback_resolver::resolve_streaming_session,
    state::AppState,
};
use dioxus::prelude::*;
use g3_ui::Color;
use serde::{Deserialize, Serialize};

/// How much of a video to keep. Each height means that or the closest below
/// it, since not every video is offered at every height.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DownloadQuality {
    Best,
    P1080,
    P720,
    P480,
    P360,
    AudioOnly,
}

impl DownloadQuality {
    pub const ALL: [Self; 6] = [
        Self::Best,
        Self::P1080,
        Self::P720,
        Self::P480,
        Self::P360,
        Self::AudioOnly,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Best => "Best available",
            Self::P1080 => "1080p",
            Self::P720 => "720p",
            Self::P480 => "480p",
            Self::P360 => "360p",
            Self::AudioOnly => "Audio only",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Best => "Highest quality, most space",
            Self::AudioOnly => "Smallest, for podcasts and music",
            _ => "Or the closest below it",
        }
    }

    /// What `TawnyDownloads.start` reads as `quality`.
    fn js_value(self) -> &'static str {
        match self {
            Self::Best => "best",
            Self::P1080 => "1080",
            Self::P720 => "720",
            Self::P480 => "480",
            Self::P360 => "360",
            Self::AudioOnly => "audio",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadState {
    Downloading,
    Done,
    Failed,
}

/// One download as the page reports it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DownloadEntry {
    pub video_id: String,
    pub state: DownloadState,
    /// From 0 to 1.
    pub progress: f64,
    /// Bytes kept so far.
    pub bytes: u64,
    /// The quality actually kept, such as "720p".
    pub quality: String,
    /// The video as it was when downloaded, so the list and the player have
    /// its title and artwork without asking the server.
    pub video: Option<Video>,
    /// The artwork as a data URL, when it could be fetched.
    pub thumbnail: Option<String>,
    pub error: Option<String>,
}

impl DownloadEntry {
    /// "720p · 84 MB", or how far along it is.
    pub fn summary(&self) -> String {
        match self.state {
            DownloadState::Done => format!("{} · {}", self.quality, size_label(self.bytes)),
            DownloadState::Downloading => {
                let percent = (self.progress * 100.0).floor() as u32;
                if self.bytes > 0 {
                    format!("Downloading {percent}% · {}", size_label(self.bytes))
                } else {
                    format!("Downloading {percent}%")
                }
            }
            DownloadState::Failed => "The download couldn't finish. Please try again.".to_string(),
        }
    }

    pub fn title(&self) -> String {
        self.video
            .as_ref()
            .map(|video| video.title.clone())
            .unwrap_or_else(|| self.video_id.clone())
    }
}

pub fn size_label(bytes: u64) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    let megabytes = bytes as f64 / MB;
    if megabytes >= 1024.0 {
        format!("{:.1} GB", megabytes / 1024.0)
    } else if megabytes >= 10.0 {
        format!("{megabytes:.0} MB")
    } else {
        format!("{megabytes:.1} MB")
    }
}

/// Starts keeping `video` on this device. Returns once it is under way; the
/// list in [`AppState::downloads`] follows it from there.
pub async fn start_download(
    app_state: AppState,
    video: Video,
    quality: DownloadQuality,
) -> Result<(), String> {
    if video.is_live {
        return Err("A live stream cannot be downloaded".to_string());
    }
    let (prefer_sabr, from_device, preferred_audio_language) = {
        let settings = app_state.settings.read();
        (
            settings.prefer_sabr,
            settings.stream_from_device,
            settings.preferred_audio_language.clone(),
        )
    };
    let session = resolve_streaming_session(video.id.clone(), prefer_sabr, false, from_device)
        .await
        .map_err(|failure| failure.summary)?;
    let request = serde_json::json!({
        "videoId": video.id,
        "quality": quality.js_value(),
        "video": video,
        "session": session,
        "serverUrl": crate::server_url::backend_url(),
        "preferredAudioLanguage": preferred_audio_language,
    });
    let script = format!(
        r#"
        const request = {request};
        for (let attempt = 0; attempt < 200 && !window.TawnyDownloads; attempt++) {{
            await new Promise((resolve) => setTimeout(resolve, 25));
        }}
        try {{
            if (!window.TawnyDownloads) throw new Error('The downloader has not loaded');
            await window.TawnyDownloads.start(request);
            dioxus.send(null);
        }} catch (error) {{
            dioxus.send(String(error?.message || error));
        }}
        "#
    );
    match document::eval(&script).recv::<Option<String>>().await {
        Ok(None) => Ok(()),
        Ok(Some(error)) => Err(error),
        Err(error) => Err(error.to_string()),
    }
}

/// Cancels a download under way, or deletes a kept one.
pub async fn remove_download(video_id: &str) {
    let Ok(video_id) = serde_json::to_string(video_id) else {
        return;
    };
    let script = format!(
        "await window.TawnyDownloads?.remove({video_id}).catch(() => {{}}); dioxus.send(true);"
    );
    let _ = document::eval(&script).recv::<bool>().await;
}

/// A session that plays the kept copy of `video_id`, when there is one.
///
/// Asked before the server on every play, so a downloaded video starts
/// without a network round trip and plays with no connection at all. Where
/// the page cannot answer (the server's render), there is no download.
pub async fn offline_session(video_id: &str) -> Option<PlaybackSession> {
    let encoded = serde_json::to_string(video_id).ok()?;
    let script = format!(
        r#"
        for (let attempt = 0; attempt < 80 && !window.TawnyDownloads; attempt++) {{
            await new Promise((resolve) => setTimeout(resolve, 25));
        }}
        dioxus.send(window.TawnyDownloads ? await window.TawnyDownloads.offlineUri({encoded}) : null);
        "#
    );
    let uri = document::eval(&script)
        .recv::<Option<String>>()
        .await
        .ok()
        .flatten()?;
    Some(offline_playback_session(video_id, uri))
}

fn offline_playback_session(video_id: &str, offline_uri: String) -> PlaybackSession {
    PlaybackSession {
        primary: PlaybackSource {
            protocol: PlaybackProtocol::Dash,
            url: offline_uri,
            // Shaka's own parser for its stored manifests. Named rather than
            // guessed, since guessing asks the URI's scheme plugin first.
            mime_type: Some("application/x-offline-manifest".to_string()),
            po_token: None,
            expires_at: None,
            quality_label: Some("Downloaded".to_string()),
            request_headers: Vec::new(),
            tracks: Vec::new(),
        },
        alternatives: Vec::new(),
        fallback_url: format!(
            "https://www.youtube-nocookie.com/embed/{video_id}?enablejsapi=1&autoplay=1&playsinline=1"
        ),
    }
}

/// Whether this browser can keep videos at all, once the player library has
/// loaded: Shaka's offline storage needs IndexedDB. A browser that never
/// loads it in time counts as unable.
const DOWNLOADS_SUPPORTED_JS: &str = r#"
for (let attempt = 0; attempt < 800 && !(window.TawnyDownloads && window.shaka); attempt++) {
    await new Promise((resolve) => setTimeout(resolve, 25));
}
dioxus.send(Boolean(window.TawnyDownloads?.supported?.()));
"#;

/// Reports the page's downloads whenever they change.
const WATCH_DOWNLOADS_JS: &str = r#"
for (let attempt = 0; attempt < 400 && !window.TawnyDownloads; attempt++) {
    await new Promise((resolve) => setTimeout(resolve, 25));
}
if (window.TawnyDownloads) {
    let sending = false;
    let again = false;
    const send = async () => {
        if (sending) { again = true; return; }
        sending = true;
        try {
            dioxus.send(await window.TawnyDownloads.list());
        } finally {
            sending = false;
            if (again) { again = false; send(); }
        }
    };
    window.addEventListener('tawnydownloadschange', send);
    send();
}
await new Promise(() => {});
"#;

/// Mounted once, beneath [`AppState`]'s provider: keeps
/// [`AppState::downloads`] in step with the page, and says when one finishes.
#[component]
pub fn DownloadsSync() -> Element {
    let app_state = use_context::<AppState>();
    // An effect, so only the client runs it, reading no signal, so once.
    use_effect(move || {
        spawn(watch(app_state));
        spawn(check_support(app_state));
    });

    rsx! {}
}

/// Hides every way to download where it cannot work, rather than letting
/// each one fail when tapped.
async fn check_support(mut app_state: AppState) {
    if let Ok(supported) = document::eval(DOWNLOADS_SUPPORTED_JS).recv::<bool>().await
        && !supported
    {
        app_state.downloads_supported.set(false);
    }
}

async fn watch(mut app_state: AppState) {
    let mut eval = document::eval(WATCH_DOWNLOADS_JS);
    while let Ok(entries) = eval.recv::<Vec<DownloadEntry>>().await {
        let previous = app_state.downloads.peek().clone();
        for entry in &entries {
            let was_downloading = previous.iter().any(|old| {
                old.video_id == entry.video_id && old.state == DownloadState::Downloading
            });
            if !was_downloading {
                continue;
            }
            match entry.state {
                DownloadState::Done => {
                    app_state.show_toast(format!("Downloaded: {}", entry.title()), Color::Success)
                }
                DownloadState::Failed => app_state.show_toast(
                    format!("Download failed: {}", entry.summary()),
                    Color::Danger,
                ),
                DownloadState::Downloading => {}
            }
        }
        app_state.downloads.set(entries);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(state: DownloadState) -> DownloadEntry {
        DownloadEntry {
            video_id: "abcdefghijk".into(),
            state,
            progress: 0.426,
            bytes: 88 * 1024 * 1024,
            quality: "720p".into(),
            video: None,
            thumbnail: None,
            error: None,
        }
    }

    #[test]
    fn a_kept_video_plays_from_shakas_storage_not_the_server() {
        let session = offline_playback_session("abcdefghijk", "offline:manifest/3".into());
        assert_eq!(session.primary.protocol, PlaybackProtocol::Dash);
        assert_eq!(session.primary.url, "offline:manifest/3");
        assert_eq!(
            session.primary.mime_type.as_deref(),
            Some("application/x-offline-manifest")
        );
        assert!(session.alternatives.is_empty());
    }

    #[test]
    fn the_summary_says_what_was_kept_or_how_far_along() {
        assert_eq!(entry(DownloadState::Done).summary(), "720p · 88 MB");
        assert_eq!(
            entry(DownloadState::Downloading).summary(),
            "Downloading 42% · 88 MB"
        );
        assert_eq!(
            entry(DownloadState::Failed).summary(),
            "The download couldn't finish. Please try again."
        );
    }

    #[test]
    fn sizes_read_at_a_sensible_precision() {
        assert_eq!(size_label(1024 * 1024 / 2), "0.5 MB");
        assert_eq!(size_label(250 * 1024 * 1024), "250 MB");
        assert_eq!(size_label(3 * 1024 * 1024 * 1024), "3.0 GB");
    }

    /// The page sends what `TawnyDownloads.list()` builds.
    #[test]
    fn reads_the_list_the_page_sends() {
        let json = r#"[{"video_id":"abcdefghijk","state":"downloading","progress":0.5,
            "bytes":1000,"quality":"720p","video":null,"thumbnail":null,"error":null,
            "stored_at":9007199254740991}]"#;
        let entries: Vec<DownloadEntry> = serde_json::from_str(json).expect("parses");
        assert_eq!(entries[0].state, DownloadState::Downloading);
    }
}
