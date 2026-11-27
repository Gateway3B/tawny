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
use dioxus::{core::spawn_forever, prelude::*};
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
    /// Waiting its turn in [`AppState::download_queue`]. Never sent by the
    /// page: the queue is this module's.
    Queued,
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
            DownloadState::Queued => format!("Waiting · {}", self.quality),
            DownloadState::Failed => match &self.error {
                Some(error) if !error.is_empty() => format!("Failed: {error}"),
                _ => "The download couldn't finish. Please try again.".to_string(),
            },
        }
    }

    /// Whether this download is still to come: waiting or under way.
    pub fn pending(&self) -> bool {
        matches!(
            self.state,
            DownloadState::Queued | DownloadState::Downloading
        )
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

/// Videos offered for download together, and what the sheet calls them.
#[derive(Clone, Debug, PartialEq)]
pub struct BulkDownload {
    pub title: String,
    pub videos: Vec<Video>,
}

/// A video waiting for its turn to download.
#[derive(Clone, Debug, PartialEq)]
pub struct QueuedDownload {
    pub video: Video,
    pub quality: DownloadQuality,
    /// Its turn has come and its streams are being resolved. It stays queued
    /// until the page lists it, as under way or failed: resolving is where a
    /// bad connection fails, and taken off the queue first it was shown
    /// nowhere while it did.
    pub starting: bool,
}

impl QueuedDownload {
    /// How the Downloads page and the download sheet show it.
    pub fn entry(&self) -> DownloadEntry {
        DownloadEntry {
            video_id: self.video.id.clone(),
            state: if self.starting {
                DownloadState::Downloading
            } else {
                DownloadState::Queued
            },
            progress: 0.0,
            bytes: 0,
            quality: self.quality.label().to_string(),
            video: Some(self.video.clone()),
            thumbnail: None,
            error: None,
        }
    }
}

/// What the page lists, with the queue laid over it: a video waiting to be
/// retried shows as waiting rather than as its earlier failure too, and one
/// starting gives way to the page's own entry once the page has one.
pub fn merged_entries(listed: &[DownloadEntry], queue: &[QueuedDownload]) -> Vec<DownloadEntry> {
    let mut entries = listed.to_vec();
    for queued in queue {
        let entry = queued.entry();
        match entries
            .iter_mut()
            .find(|listed| listed.video_id == entry.video_id)
        {
            Some(listed) if queued.starting && listed.state != DownloadState::Failed => {}
            Some(listed) => *listed = entry,
            None => entries.push(entry),
        }
    }
    entries
}

/// How many downloads run at once. A whole playlist started together shared
/// one weak connection between all of them, and most then failed; two keep
/// the line busy without starving each other.
const PARALLEL_DOWNLOADS: usize = 2;

/// Queues `videos` to download at `quality`, two at a time, answering how
/// many were added. Videos already kept, under way or waiting are left as
/// they are; a failed one is tried again.
pub fn queue_downloads(
    mut app_state: AppState,
    videos: Vec<Video>,
    quality: DownloadQuality,
) -> usize {
    let added = {
        let downloads = app_state.downloads.peek();
        let mut queue = app_state.download_queue.write();
        let mut added = 0;
        for video in videos {
            let busy = downloads
                .iter()
                .any(|entry| entry.video_id == video.id && entry.state != DownloadState::Failed)
                || queue.iter().any(|queued| queued.video.id == video.id);
            if video.is_live || busy {
                continue;
            }
            queue.push(QueuedDownload {
                video,
                quality,
                starting: false,
            });
            added += 1;
        }
        added
    };
    if added > 0 && !*app_state.download_worker.peek() {
        app_state.download_worker.set(true);
        // Owned by no page: leaving the one that queued them must not stop
        // the rest.
        spawn_forever(run_queue(app_state));
    }
    added
}

/// Takes waiting videos off the queue while fewer than
/// [`PARALLEL_DOWNLOADS`] are under way. Each is resolved only when its turn
/// comes, so a long playlist does not hand its last videos stream addresses
/// that expired while they waited.
async fn run_queue(mut app_state: AppState) {
    loop {
        let (active, next) = {
            let downloads = app_state.downloads.peek();
            let queue = app_state.download_queue.peek();
            let active = merged_entries(&downloads, &queue)
                .iter()
                .filter(|entry| entry.state == DownloadState::Downloading)
                .count();
            let next = queue.iter().find(|queued| !queued.starting).cloned();
            (active, next)
        };
        if active >= PARALLEL_DOWNLOADS {
            wait_for_downloads_change().await;
            continue;
        }
        let Some(next) = next else {
            if active == 0 {
                break;
            }
            wait_for_downloads_change().await;
            continue;
        };
        let id = next.video.id.clone();
        if let Some(queued) = app_state
            .download_queue
            .write()
            .iter_mut()
            .find(|queued| queued.video.id == id)
        {
            queued.starting = true;
        }
        spawn_forever(async move {
            let video = next.video.clone();
            if let Err(error) = start_download(app_state, next.video, next.quality).await {
                record_failure(&video, &error).await;
            }
            // The page lists it by now, as under way or failed.
            app_state
                .download_queue
                .write()
                .retain(|queued| !(queued.starting && queued.video.id == id));
        });
    }
    app_state.download_worker.set(false);
    // Something may have been queued between the last look and the flag
    // coming down.
    if !app_state.download_queue.peek().is_empty() {
        app_state.download_worker.set(true);
        spawn_forever(run_queue(app_state));
    }
}

/// Returns when the page's downloads change, or after a couple of seconds.
async fn wait_for_downloads_change() {
    let _ = document::eval(
        r#"
        await new Promise((resolve) => {
            window.addEventListener('tawnydownloadschange', resolve, { once: true });
            setTimeout(resolve, 2000);
        });
        dioxus.send(true);
        "#,
    )
    .recv::<bool>()
    .await;
}

/// Lists `video` as failed with `error`, so a download that never got under
/// way (no connection to resolve it, say) can be retried from Downloads like
/// any other rather than vanishing with a toast.
pub async fn record_failure(video: &Video, error: &str) {
    let request = serde_json::json!({ "video": video, "error": error });
    let script = format!(
        r#"
        for (let attempt = 0; attempt < 200 && !window.TawnyDownloads; attempt++) {{
            await new Promise((resolve) => setTimeout(resolve, 25));
        }}
        window.TawnyDownloads?.recordFailure({request});
        dioxus.send(true);
        "#
    );
    let _ = document::eval(&script).recv::<bool>().await;
}

/// Stops every download, waiting or under way, and deletes every kept one.
pub async fn remove_all_downloads(mut app_state: AppState) {
    app_state.download_queue.write().clear();
    let _ = document::eval(
        "await window.TawnyDownloads?.removeAll().catch(() => {}); dioxus.send(true);",
    )
    .recv::<bool>()
    .await;
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

/// Takes a download off the queue, cancels one under way, or deletes a kept
/// one.
pub async fn remove_download(mut app_state: AppState, video_id: &str) {
    app_state
        .download_queue
        .write()
        .retain(|queued| queued.video.id != video_id);
    let Ok(video_id) = serde_json::to_string(video_id) else {
        return;
    };
    let script = format!(
        "await window.TawnyDownloads?.remove({video_id}).catch(() => {{}}); dioxus.send(true);"
    );
    let _ = document::eval(&script).recv::<bool>().await;
}

/// Saves a kept video as one file another app can open (Keyscribe reads
/// piano videos from one), answering what to tell the viewer. On Android the
/// file goes in Download/Tawny; elsewhere, wherever the browser saves.
pub async fn export_download(entry: &DownloadEntry) -> Result<String, String> {
    let (extension, mime_type) = if entry.quality == AUDIO_ONLY_LABEL {
        ("m4a", "audio/mp4")
    } else {
        ("mp4", "video/mp4")
    };
    let file_name = format!("{}.{extension}", export_file_stem(&entry.title()));
    let video_id = serde_json::to_string(&entry.video_id).map_err(|error| error.to_string())?;

    #[cfg(target_os = "android")]
    {
        let export = crate::video_export::begin(file_name, mime_type).await?;
        let upload =
            serde_json::to_string(export.upload_url()).map_err(|error| error.to_string())?;
        let failure = run_export(&format!(
            "window.TawnyExport.exportToUpload({video_id}, {upload})"
        ))
        .await;
        // Finished either way: a failed export deletes its half-written file.
        let saved = crate::video_export::finish(export, failure.is_none()).await;
        if let Some(failure) = failure {
            return Err(failure);
        }
        saved.map(|path| format!("Saved to {path}"))
    }

    #[cfg(not(target_os = "android"))]
    {
        let name = serde_json::to_string(&file_name).map_err(|error| error.to_string())?;
        let mime = serde_json::to_string(mime_type).map_err(|error| error.to_string())?;
        match run_export(&format!(
            "window.TawnyExport.saveInBrowser({video_id}, {name}, {mime})"
        ))
        .await
        {
            Some(failure) => Err(failure),
            None => Ok(format!("Saved {file_name}")),
        }
    }
}

/// The label `TawnyDownloads` gives an audio-only download.
const AUDIO_ONLY_LABEL: &str = "Audio only";

/// Runs `call` (a promise) once the exporter has loaded, answering its error.
async fn run_export(call: &str) -> Option<String> {
    let script = format!(
        r#"
        for (let attempt = 0; attempt < 200 && !window.TawnyExport; attempt++) {{
            await new Promise((resolve) => setTimeout(resolve, 25));
        }}
        try {{
            if (!window.TawnyExport) throw new Error('The exporter has not loaded');
            await {call};
            dioxus.send(null);
        }} catch (error) {{
            dioxus.send(String(error?.message || error));
        }}
        "#
    );
    match document::eval(&script).recv::<Option<String>>().await {
        Ok(failure) => failure,
        Err(error) => Some(error.to_string()),
    }
}

/// A video title as a file name every system accepts: no path separators or
/// characters Windows and Android's storage refuse, and not too long.
fn export_file_stem(title: &str) -> String {
    let cleaned: String = title
        .chars()
        .map(|character| match character {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => ' ',
            character if character.is_control() => ' ',
            character => character,
        })
        .collect();
    let joined = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let short: String = joined.chars().take(120).collect();
    // Windows drops trailing dots and spaces, and a name of only dots is none.
    let trimmed = short.trim_matches(|character: char| character == '.' || character == ' ');
    if trimmed.is_empty() {
        "Tawny video".to_string()
    } else {
        trimmed.to_string()
    }
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
                DownloadState::Queued | DownloadState::Downloading => {}
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
        let mut cut_off = entry(DownloadState::Failed);
        cut_off.error = Some("The download was interrupted".into());
        assert_eq!(cut_off.summary(), "Failed: The download was interrupted");
        assert_eq!(entry(DownloadState::Queued).summary(), "Waiting · 720p");
    }

    fn queued(id: &str, starting: bool) -> QueuedDownload {
        QueuedDownload {
            video: Video {
                id: id.into(),
                title: id.into(),
                channel_id: "channel".into(),
                channel_name: "Channel".into(),
                thumbnail_url: String::new(),
                published_at: String::new(),
                duration_seconds: 60,
                view_count: String::new(),
                progress_seconds: 0,
                watched: false,
                is_live: false,
                is_short: false,
                audio_only: false,
                channel_avatar_url: None,
            },
            quality: DownloadQuality::P720,
            starting,
        }
    }

    #[test]
    fn the_queue_is_shown_over_the_page_list_once_per_video() {
        let mut failed = entry(DownloadState::Failed);
        failed.video_id = "retried".into();
        let mut under_way = entry(DownloadState::Downloading);
        under_way.video_id = "started".into();
        let queue = [
            queued("retried", false),
            queued("started", true),
            queued("resolving", true),
        ];
        let merged = merged_entries(&[failed, under_way], &queue);
        let states = merged
            .iter()
            .map(|entry| (entry.video_id.as_str(), entry.state, entry.progress))
            .collect::<Vec<_>>();
        assert_eq!(
            states,
            vec![
                // A retry replaces its earlier failure.
                ("retried", DownloadState::Queued, 0.0),
                // The page's own entry, with its progress, once it has one.
                ("started", DownloadState::Downloading, 0.426),
                // Resolving, not yet listed by the page: still shown.
                ("resolving", DownloadState::Downloading, 0.0),
            ]
        );
    }

    #[test]
    fn sizes_read_at_a_sensible_precision() {
        assert_eq!(size_label(1024 * 1024 / 2), "0.5 MB");
        assert_eq!(size_label(250 * 1024 * 1024), "250 MB");
        assert_eq!(size_label(3 * 1024 * 1024 * 1024), "3.0 GB");
    }

    #[test]
    fn export_names_are_titles_any_system_accepts() {
        assert_eq!(
            export_file_stem("Clair de Lune - Debussy (Piano Tutorial)"),
            "Clair de Lune - Debussy (Piano Tutorial)"
        );
        assert_eq!(
            export_file_stem("AC/DC: \"Thunderstruck\" | Piano?*"),
            "AC DC Thunderstruck Piano"
        );
        assert_eq!(export_file_stem("  ...  "), "Tawny video");
        assert_eq!(export_file_stem("Ending in dots..."), "Ending in dots");
        assert_eq!(export_file_stem(&"é".repeat(300)).chars().count(), 120);
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
