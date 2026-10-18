use super::*;

/// Innertube clients to try, in order, for player extraction.
///
/// Pinned rather than using rustypipe's `player()` default, because that
/// default is environment-dependent: it puts `Desktop` first whenever a
/// `rustypipe-botguard` binary happens to be on PATH. Every client except iOS
/// needs signature deobfuscation, which rustypipe 0.11.4 can no longer do
/// against the current player JS, so a `Desktop` attempt fails outright and
/// never falls through to the client that works. Installing an unrelated tool
/// should not be able to break playback.
pub(crate) const PLAYER_CLIENTS: &[ClientType] = &[ClientType::Ios, ClientType::Tv];

/// Locate a `yt-dlp` binary, honouring `TAWNY_YTDLP_BIN` first.
pub(crate) fn locate_ytdlp() -> Option<std::path::PathBuf> {
    if let Ok(configured) = std::env::var("TAWNY_YTDLP_BIN")
        && !configured.trim().is_empty()
    {
        let path = std::path::PathBuf::from(configured.trim());
        if path.exists() {
            return Some(path);
        }
        eprintln!("TAWNY_YTDLP_BIN does not exist: {}", path.display());
        return None;
    }
    // Fall back to PATH. `--version` is the cheapest way to confirm it runs.
    let name = if cfg!(windows) {
        "yt-dlp.exe"
    } else {
        "yt-dlp"
    };
    std::process::Command::new(name)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .ok()
        .filter(|status| status.success())
        .map(|_| std::path::PathBuf::from(name))
}

pub(crate) fn configured_ytdlp_po_provider_url() -> Option<String> {
    configured_http_url("TAWNY_PO_TOKEN_PROVIDER_URL")
}

pub(crate) fn configured_ytdlp_service_url() -> Option<String> {
    configured_http_url("TAWNY_YTDLP_SERVICE_URL")
}

pub(crate) fn configured_http_url(variable: &str) -> Option<String> {
    let configured = std::env::var(variable).ok()?;
    let configured = configured.trim().trim_end_matches('/');
    if configured.is_empty() {
        return None;
    }
    let valid = reqwest::Url::parse(configured)
        .is_ok_and(|url| matches!(url.scheme(), "http" | "https") && url.host_str().is_some());
    if !valid {
        eprintln!("ignoring {variable}: expected an absolute http(s) URL");
        return None;
    }
    Some(configured.to_string())
}

pub(crate) fn ytdlp_service_video_url(service_url: &str, video_id: &str) -> Option<reqwest::Url> {
    let mut url = reqwest::Url::parse(service_url).ok()?;
    url.path_segments_mut()
        .ok()?
        .extend(["v1", "videos", video_id]);
    Some(url)
}

pub(crate) fn ytdlp_service_channel_shorts_url(
    service_url: &str,
    channel_id: &str,
) -> Option<reqwest::Url> {
    let mut url = reqwest::Url::parse(service_url).ok()?;
    url.path_segments_mut()
        .ok()?
        .extend(["v1", "channels", channel_id, "shorts"]);
    Some(url)
}

pub(crate) fn ytdlp_po_provider_args(provider_url: Option<&str>) -> Vec<String> {
    let Some(provider_url) = provider_url else {
        return Vec::new();
    };
    vec![
        "--extractor-args".into(),
        format!("youtubepot-bgutilhttp:base_url={provider_url}"),
        "--extractor-args".into(),
        "youtube:player_client=mweb".into(),
    ]
}

/// A stream URL yt-dlp extracted, keyed by itag.
#[derive(Debug, Deserialize)]
pub(crate) struct YtdlpFormat {
    pub(crate) format_id: String,
    pub(crate) url: Option<String>,
    #[serde(default)]
    pub(crate) filesize: Option<u64>,
    #[serde(default)]
    pub(crate) filesize_approx: Option<u64>,
    #[serde(default)]
    pub(crate) ext: Option<String>,
    #[serde(default)]
    pub(crate) container: Option<String>,
    #[serde(default)]
    pub(crate) vcodec: Option<String>,
    #[serde(default)]
    pub(crate) acodec: Option<String>,
    #[serde(default)]
    pub(crate) width: Option<u32>,
    #[serde(default)]
    pub(crate) height: Option<u32>,
    #[serde(default)]
    pub(crate) fps: Option<f64>,
    #[serde(default)]
    pub(crate) tbr: Option<f64>,
    #[serde(default)]
    pub(crate) format_note: Option<String>,
    #[serde(default)]
    pub(crate) language: Option<String>,
    #[serde(default)]
    pub(crate) has_drm: Option<bool>,
    #[serde(default)]
    pub(crate) protocol: Option<String>,
    /// The master playlist an HLS format belongs to. Every m3u8 format of one
    /// extraction shares it.
    #[serde(default)]
    pub(crate) manifest_url: Option<String>,
    /// Filled in from the dump's top-level duration, which is where yt-dlp
    /// reports it: the per-format entries carry none, and a DASH manifest
    /// without one presents as a zero-length video.
    #[serde(skip)]
    pub(crate) duration_ms: Option<u64>,
}

impl YtdlpFormat {
    /// The itag, which is the format id up to any `-drc` style suffix.
    fn itag(&self) -> Option<u32> {
        self.format_id.split('-').next()?.parse().ok()
    }

    fn codec(value: &Option<String>) -> Option<&str> {
        value
            .as_deref()
            .filter(|codec| !codec.is_empty() && *codec != "none")
    }

    fn video_codec(&self) -> Option<&str> {
        Self::codec(&self.vcodec)
    }

    fn audio_codec(&self) -> Option<&str> {
        Self::codec(&self.acodec)
    }

    /// Adaptive formats carry exactly one of the two streams.
    fn adaptive_kind(&self) -> Option<PlaybackTrackKind> {
        match (self.video_codec(), self.audio_codec()) {
            (Some(_), None) => Some(PlaybackTrackKind::Video),
            (None, Some(_)) => Some(PlaybackTrackKind::Audio),
            _ => None,
        }
    }

    /// A DASH-ready container, as opposed to the muxed progressive formats and
    /// the m3u8 entries that only appear on live streams.
    fn is_dash(&self) -> bool {
        self.container
            .as_deref()
            .is_some_and(|container| container.ends_with("_dash"))
    }

    /// `video/mp4; codecs="avc1.4d400c"`, assembled the way a DASH manifest
    /// wants it. WebM and MP4 are the only containers YouTube serves adaptive.
    fn mime_type(&self, kind: PlaybackTrackKind) -> Option<String> {
        let subtype = match self.ext.as_deref() {
            Some("mp4") | Some("m4a") => "mp4",
            Some("webm") => "webm",
            _ => return None,
        };
        let top = match kind {
            PlaybackTrackKind::Video => "video",
            PlaybackTrackKind::Audio => "audio",
        };
        let codec = self.video_codec().or_else(|| self.audio_codec())?;
        Some(format!("{top}/{subtype}; codecs=\"{codec}\""))
    }

    fn content_length(&self) -> Option<u64> {
        self.filesize.or(self.filesize_approx)
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct YtdlpDump {
    #[serde(default)]
    pub(crate) formats: Vec<YtdlpFormat>,
    #[serde(default)]
    pub(crate) duration: Option<f64>,
}

pub(crate) fn formats_from_ytdlp_dump(mut dump: YtdlpDump) -> Vec<YtdlpFormat> {
    let duration_ms = dump
        .duration
        .filter(|seconds| seconds.is_finite() && *seconds > 0.0)
        .map(|seconds| (seconds * 1000.0).round() as u64);
    for format in &mut dump.formats {
        format.duration_ms = duration_ms;
    }
    dump.formats
}

#[derive(Debug, Deserialize)]
pub(crate) struct YtdlpFlatEntry {
    #[serde(default)]
    pub(crate) id: String,
    pub(crate) title: Option<String>,
    #[serde(default)]
    pub(crate) duration: Option<f64>,
    #[serde(default)]
    pub(crate) view_count: Option<i64>,
    #[serde(default)]
    pub(crate) timestamp: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct YtdlpFlatPlaylist {
    #[serde(default)]
    pub(crate) entries: Vec<YtdlpFlatEntry>,
}

/// How long ago `timestamp` was, as YouTube writes it ("2 years ago").
///
/// A channel tab lists uploads with relative times, and yt-dlp's
/// `approximate_date` turns those into timestamps. Written as a calendar date,
/// "2 years ago" became today's date two years back and looked exact: it
/// outranked a real date, displayed a day the video was not posted, and sorted
/// among exact timestamps by that guess. Kept relative, it reads as the
/// approximation it is, and a finer date from RSS or the watch page wins.
pub(crate) fn approximate_age_label(timestamp: i64, now: i64) -> String {
    let seconds = now.saturating_sub(timestamp).max(0);
    let (count, unit) = match seconds {
        s if s < 3_600 => ((s / 60).max(1), "minute"),
        s if s < 86_400 => (s / 3_600, "hour"),
        s if s < 7 * 86_400 => (s / 86_400, "day"),
        s if s < 30 * 86_400 => (s / (7 * 86_400), "week"),
        s if s < 365 * 86_400 => (s / (30 * 86_400), "month"),
        s => (s / (365 * 86_400), "year"),
    };
    let plural = if count == 1 { "" } else { "s" };
    format!("{count} {unit}{plural} ago")
}

pub(crate) fn ytdlp_flat_playlist_videos(
    dump: YtdlpFlatPlaylist,
    channel_id: &str,
    channel_name: &str,
) -> Vec<Video> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    dump.entries
        .into_iter()
        .filter(|entry| !entry.id.is_empty())
        .map(|entry| Video {
            thumbnail_url: format!("https://i.ytimg.com/vi/{}/hqdefault.jpg", entry.id),
            id: entry.id,
            title: entry.title.unwrap_or_default(),
            channel_id: channel_id.to_string(),
            channel_name: channel_name.to_string(),
            published_at: entry
                .timestamp
                .map(|stamp| approximate_age_label(stamp, now))
                .unwrap_or_default(),
            duration_seconds: entry.duration.unwrap_or(0.0) as u64,
            view_count: entry
                .view_count
                .map(|count| compact_count(count, " views"))
                .unwrap_or_default(),
            progress_seconds: 0,
            watched: false,
            is_live: false,
            is_short: true,
            audio_only: false,
            channel_avatar_url: None,
        })
        .collect()
}

impl AppServerState {
    pub(crate) async fn ytdlp_channel_uploads(
        &self,
        channel_id: &str,
        channel_name: &str,
        tab: ChannelMediaTab,
    ) -> Result<Vec<Video>> {
        let kind = match tab {
            ChannelMediaTab::Videos => "videos",
            ChannelMediaTab::Shorts => "shorts",
            ChannelMediaTab::Live => "streams",
        };
        let dump = if let Some(service_url) = &self.ytdlp_service_url {
            let mut url = reqwest::Url::parse(service_url)?;
            url.path_segments_mut()
                .map_err(|_| anyhow!("invalid extractor URL"))?
                .extend(["v1", "channels", channel_id, "uploads", kind]);
            self.ytdlp_http
                .get(url)
                .send()
                .await?
                .error_for_status()?
                .json::<YtdlpFlatPlaylist>()
                .await?
        } else {
            let binary = self
                .ytdlp_bin
                .as_ref()
                .ok_or_else(|| anyhow!("channel extractor is unavailable"))?;
            let output = tokio::time::timeout(
                Duration::from_secs(40),
                tokio::process::Command::new(binary)
                    .args([
                        "--flat-playlist",
                        "-J",
                        "--no-warnings",
                        "--socket-timeout",
                        "15",
                        "--extractor-args",
                        "youtubetab:approximate_date",
                        &format!("https://www.youtube.com/channel/{channel_id}/{kind}"),
                    ])
                    .stdin(std::process::Stdio::null())
                    .output(),
            )
            .await??;
            if !output.status.success() {
                return Err(anyhow!("channel listing failed"));
            }
            serde_json::from_slice(&output.stdout)?
        };
        let mut videos = ytdlp_flat_playlist_videos(dump, channel_id, channel_name);
        for video in &mut videos {
            video.is_short = tab == ChannelMediaTab::Shorts;
            video.is_live = false;
        }
        Ok(videos)
    }
    pub(crate) async fn adaptive_tail_is_available(&self, source: &PlaybackSource) -> bool {
        if source.tracks.is_empty()
            || !matches!(
                source.protocol,
                PlaybackProtocol::Dash | PlaybackProtocol::Sabr
            )
        {
            return true;
        }
        let video = source
            .tracks
            .iter()
            .filter(|track| track.kind == PlaybackTrackKind::Video)
            .max_by_key(|track| {
                (
                    track.height.unwrap_or_default(),
                    track.bitrate.unwrap_or_default(),
                )
            });
        let audio = source
            .tracks
            .iter()
            .filter(|track| track.kind == PlaybackTrackKind::Audio)
            .max_by_key(|track| track.bitrate.unwrap_or_default());

        // Both ends at once: this sits in front of the first frame.
        let probes = [video, audio]
            .into_iter()
            .flatten()
            .map(|track| async move {
                let Some(length) = track.content_length.filter(|length| *length > 0) else {
                    return true;
                };
                let start = length.saturating_sub(2_048);
                let mut request = self
                    .media_http
                    .get(&track.url)
                    .header(
                        reqwest::header::RANGE,
                        format!("bytes={start}-{}", length - 1),
                    )
                    .header(reqwest::header::ACCEPT_ENCODING, "identity");
                for configured in source
                    .request_headers
                    .iter()
                    .chain(track.request_headers.iter())
                {
                    if let (Ok(name), Ok(value)) = (
                        reqwest::header::HeaderName::from_bytes(configured.name.as_bytes()),
                        reqwest::header::HeaderValue::from_str(&configured.value),
                    ) {
                        request = request.header(name, value);
                    }
                }
                let response = tokio::time::timeout(Duration::from_secs(4), request.send()).await;
                matches!(response, Ok(Ok(response)) if response.status().is_success())
            });
        futures_util::future::join_all(probes)
            .await
            .into_iter()
            .all(|available| available)
    }

    /// A channel's Shorts, listed by yt-dlp.
    ///
    /// rustypipe 0.11.4 cannot read the Shorts tab at all: verified 2026-08-13
    /// against two channels that publish Shorts, both returned zero items on
    /// the first page *and* zero after following the continuation token. yt-dlp
    /// parses the same tab correctly, and it is already a dependency for
    /// playback URLs, so it fills the gap rather than leaving the tab empty.
    pub(crate) async fn ytdlp_channel_shorts(
        &self,
        channel_id: &str,
        channel_name: &str,
    ) -> Vec<Video> {
        if let Some(service_url) = self.ytdlp_service_url.as_deref() {
            let Some(url) = ytdlp_service_channel_shorts_url(service_url, channel_id) else {
                return Vec::new();
            };
            let dump = match self.ytdlp_http.get(url).send().await {
                Ok(response) if response.status().is_success() => response
                    .json::<YtdlpFlatPlaylist>()
                    .await
                    .map_err(anyhow::Error::from),
                Ok(response) => Err(anyhow!(
                    "yt-dlp service returned {} for channel Shorts",
                    response.status()
                )),
                Err(error) => Err(error.into()),
            };
            return match dump {
                Ok(dump) => ytdlp_flat_playlist_videos(dump, channel_id, channel_name),
                Err(error) => {
                    eprintln!("yt-dlp service could not list Shorts for {channel_id}: {error}");
                    Vec::new()
                }
            };
        }
        let Some(binary) = self.ytdlp_bin.as_ref() else {
            return Vec::new();
        };
        let output = tokio::process::Command::new(binary)
            .args([
                "--flat-playlist",
                "-J",
                "--playlist-end",
                "30",
                "--no-warnings",
                "--socket-timeout",
                "15",
                &format!("https://www.youtube.com/channel/{channel_id}/shorts"),
            ])
            .stdin(std::process::Stdio::null())
            .output();
        let output = match tokio::time::timeout(Duration::from_secs(40), output).await {
            Ok(Ok(output)) if output.status.success() => output,
            _ => {
                eprintln!("yt-dlp could not list Shorts for {channel_id}");
                return Vec::new();
            }
        };
        let Ok(dump) = serde_json::from_slice::<YtdlpFlatPlaylist>(&output.stdout) else {
            return Vec::new();
        };
        ytdlp_flat_playlist_videos(dump, channel_id, channel_name)
    }

    /// Stream URLs from yt-dlp, keyed by itag.
    ///
    /// YouTube increasingly gates every client behind a video-bound GVS PO
    /// token. When `TAWNY_PO_TOKEN_PROVIDER_URL` is configured, yt-dlp uses its
    /// bgutil HTTP provider with the mweb client so those URLs remain valid for
    /// the full media object. Without a provider we keep yt-dlp's own default
    /// client selection as a best-effort fallback.
    ///
    /// Only the URL is taken. The byte ranges, codecs, and sizes still come
    /// from rustypipe, which is sound because an itag identifies one specific
    /// transcode: the two clients hand out different URLs for the same file.
    /// `filesize` is compared per itag so a mismatch is skipped rather than
    /// producing a manifest whose ranges point into the wrong bytes.
    /// Every format yt-dlp can see, which is the whole basis of playback now.
    pub(crate) async fn ytdlp_service_formats(
        &self,
        service_url: &str,
        video_id: &str,
    ) -> std::result::Result<Vec<YtdlpFormat>, PlaybackFailure> {
        let url = ytdlp_service_video_url(service_url, video_id).ok_or_else(|| {
            PlaybackFailure::new(
                PlaybackFailureOrigin::Setup,
                "The extractor service URL is not a valid URL.",
            )
            .remedy("Fix TAWNY_YTDLP_SERVICE_URL in the server's environment and restart it.")
            .detail(service_url)
        })?;
        let response = self.ytdlp_http.get(url).send().await.map_err(|error| {
            PlaybackFailure::new(
                PlaybackFailureOrigin::Setup,
                "Tawny could not reach its extractor service.",
            )
            .remedy(
                "Start the extractor container (docker compose up -d extractor) and check that \
                 TAWNY_YTDLP_SERVICE_URL points at it.",
            )
            .detail(format!("{service_url}: {error}"))
        })?;
        let status = response.status();
        let bytes = response.bytes().await.map_err(|error| {
            PlaybackFailure::new(
                PlaybackFailureOrigin::Setup,
                "The extractor service's answer was cut off.",
            )
            .remedy("Try again. If it keeps happening, check the extractor container's log.")
            .detail(error.to_string())
        })?;
        if !status.is_success() {
            return Err(extractor_status_failure(status, &bytes));
        }
        serde_json::from_slice::<YtdlpDump>(&bytes)
            .map(formats_from_ytdlp_dump)
            .map_err(|error| {
                PlaybackFailure::new(
                    PlaybackFailureOrigin::Bug,
                    "Tawny could not understand yt-dlp's answer.",
                )
                .remedy(
                    "Report it with the detail below. A yt-dlp upgrade that changed its output \
                     is the usual cause.",
                )
                .detail(error.to_string())
            })
    }

    /// Every format yt-dlp found, or why there are none. The reason is shown
    /// to the viewer, so it names the cause rather than the symptom.
    pub(crate) async fn ytdlp_formats(
        &self,
        video_id: &str,
    ) -> std::result::Result<Vec<YtdlpFormat>, PlaybackFailure> {
        let formats = self.run_ytdlp(video_id).await;
        if let Err(failure) = &formats {
            eprintln!("yt-dlp gave no formats for {video_id}: {failure}");
        }
        formats
    }

    pub(crate) async fn run_ytdlp(
        &self,
        video_id: &str,
    ) -> std::result::Result<Vec<YtdlpFormat>, PlaybackFailure> {
        if let Some(service_url) = self.ytdlp_service_url.as_deref() {
            return self.ytdlp_service_formats(service_url, video_id).await;
        }
        let Some(binary) = self.ytdlp_bin.as_ref() else {
            return Err(PlaybackFailure::new(
                PlaybackFailureOrigin::Setup,
                "The server has no extractor configured.",
            )
            .remedy("Set TAWNY_YTDLP_SERVICE_URL (or TAWNY_YTDLP_BIN) and restart the server."));
        };
        let mut command = tokio::process::Command::new(binary);
        command.args([
            "-J",
            "--no-warnings",
            "--no-playlist",
            "--socket-timeout",
            "15",
        ]);
        let po_provider_url = self.reachable_ytdlp_po_provider_url().await;
        command.args(ytdlp_po_provider_args(po_provider_url));
        command.arg(format!("https://www.youtube.com/watch?v={video_id}"));
        let output = command.stdin(std::process::Stdio::null()).output();
        let output = match tokio::time::timeout(Duration::from_secs(30), output).await {
            Ok(Ok(output)) if output.status.success() => output,
            Ok(Ok(output)) => {
                return Err(classify_ytdlp_error(&String::from_utf8_lossy(
                    &output.stderr,
                )));
            }
            Ok(Err(error)) => {
                return Err(PlaybackFailure::new(
                    PlaybackFailureOrigin::Setup,
                    "The server could not run yt-dlp.",
                )
                .remedy("Check that TAWNY_YTDLP_BIN points at a working yt-dlp.")
                .detail(error.to_string()));
            }
            Err(_) => return Err(extraction_timed_out()),
        };
        serde_json::from_slice::<YtdlpDump>(&output.stdout)
            .map(formats_from_ytdlp_dump)
            .map_err(|error| {
                PlaybackFailure::new(
                    PlaybackFailureOrigin::Bug,
                    "Tawny could not understand yt-dlp's answer.",
                )
                .remedy(
                    "Report it with the detail below. A yt-dlp upgrade that changed its output \
                     is the usual cause.",
                )
                .detail(error.to_string())
            })
    }

    /// The itag-to-URL view the extractor-layout path still needs.
    pub(crate) fn ytdlp_url_map(formats: &[YtdlpFormat]) -> HashMap<u32, (String, Option<u64>)> {
        formats
            .iter()
            .filter_map(|format| {
                let url = format.url.clone().filter(|url| !url.is_empty())?;
                Some((format.itag()?, (url, format.filesize)))
            })
            .collect()
    }
}
/// What the viewer is told when no ungated stream could be found.
pub(crate) fn no_stream_failure(
    ytdlp_failure: Option<PlaybackFailure>,
    ytdlp_format_count: usize,
) -> PlaybackFailure {
    match ytdlp_failure {
        Some(failure) => failure,
        None if ytdlp_format_count == 0 => PlaybackFailure::new(
            PlaybackFailureOrigin::Unknown,
            "yt-dlp read the video but found no streams in it.",
        )
        .remedy(
            "This happens with DRM-protected videos such as rented films, which Tawny cannot \
             play. Otherwise report it with the video link.",
        ),
        None => PlaybackFailure::new(
            PlaybackFailureOrigin::Bug,
            format!(
                "yt-dlp found {ytdlp_format_count} streams, but Tawny could not build a playable \
                 source from any of them."
            ),
        )
        .remedy("Report it with the video link; the server log has the per-stream detail."),
    }
}

pub(crate) fn extraction_timed_out() -> PlaybackFailure {
    PlaybackFailure::new(
        PlaybackFailureOrigin::Setup,
        "Extracting the video took too long and was stopped.",
    )
    .remedy(
        "Try again. If it keeps timing out, the server is slow to reach YouTube or the extractor \
         is overloaded.",
    )
    .retryable()
}

/// The extractor sidecar's non-success answers. Each status is one it sends
/// deliberately (see `docker/extractor/server.py`), and only 502 means yt-dlp
/// itself failed - with yt-dlp's own error in the body.
pub(crate) fn extractor_status_failure(
    status: reqwest::StatusCode,
    body: &[u8],
) -> PlaybackFailure {
    #[derive(Deserialize, Default)]
    struct ExtractorError {
        #[serde(default)]
        error: String,
        #[serde(default)]
        ytdlp_error: Option<String>,
    }
    let answer = serde_json::from_slice::<ExtractorError>(body).unwrap_or_default();
    let said = if answer.error.is_empty() {
        format!("extractor answered {status}")
    } else {
        format!("extractor answered {status}: {}", answer.error)
    };
    match status.as_u16() {
        502 => match answer.ytdlp_error {
            Some(error) => classify_ytdlp_error(&error),
            // An extractor older than the one that passes yt-dlp's error on.
            None => PlaybackFailure::new(
                PlaybackFailureOrigin::Unknown,
                "yt-dlp failed, but the extractor did not say why.",
            )
            .remedy(
                "The extractor container is out of date: rebuild it (docker compose up -d --build \
                 extractor) so it reports yt-dlp's error. Its log has the reason meanwhile.",
            )
            .detail(said),
        },
        503 => PlaybackFailure::new(
            PlaybackFailureOrigin::Setup,
            "The PO-token provider the extractor depends on is not running.",
        )
        .remedy("Start the pot-provider container and check its log.")
        .retryable()
        .detail(said),
        429 => PlaybackFailure::new(
            PlaybackFailureOrigin::Setup,
            "The extractor is busy with other videos.",
        )
        .remedy(
            "Try again in a moment. If it happens often, raise YTDLP_CONCURRENCY for the \
             extractor.",
        )
        .retryable()
        .detail(said),
        504 => extraction_timed_out().detail(said),
        400 => PlaybackFailure::new(
            PlaybackFailureOrigin::Bug,
            "The extractor rejected the video ID Tawny sent it.",
        )
        .remedy(
            "If you typed or pasted this link, check the video ID. Otherwise report it with the \
             link.",
        )
        .detail(said),
        _ => PlaybackFailure::new(
            PlaybackFailureOrigin::Unknown,
            "The extractor service answered with an unexpected error.",
        )
        .remedy("Check the extractor container's log.")
        .detail(said),
    }
}

/// The last `ERROR:` line of yt-dlp's stderr, without the `[youtube] <id>:`
/// prefix that only repeats what the viewer already knows.
pub(crate) fn ytdlp_error_line(stderr: &str) -> &str {
    let line = stderr
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| line.starts_with("ERROR:"))
        .or_else(|| {
            stderr
                .lines()
                .rev()
                .map(str::trim)
                .find(|line| !line.is_empty())
        })
        .unwrap_or("");
    let line = line.strip_prefix("ERROR:").unwrap_or(line).trim_start();
    match line
        .strip_prefix('[')
        .and_then(|rest| rest.split_once("] "))
    {
        Some((_, rest)) => match rest.split_once(": ") {
            Some((id, message)) if !id.contains(' ') => message,
            _ => rest,
        },
        None => line,
    }
}

/// Sort yt-dlp's error into whose problem it is.
///
/// The phrases are YouTube's own playability reasons, which yt-dlp passes on
/// verbatim, plus yt-dlp's own wording when it cannot parse YouTube's page.
/// Anything unmatched stays `Unknown` rather than guessed at: calling a
/// Tawny bug "YouTube's fault" would hide it.
pub(crate) fn classify_ytdlp_error(stderr: &str) -> PlaybackFailure {
    use PlaybackFailureOrigin::{Setup, Unknown, Youtube};
    let reason = ytdlp_error_line(stderr);
    let lower = reason.to_lowercase();
    let has = |needles: &[&str]| needles.iter().any(|needle| lower.contains(needle));
    let failure = if has(&[
        "not made this video available in your country",
        "in your country",
        "geo restrict",
    ]) {
        PlaybackFailure::new(
            Youtube,
            "YouTube blocks this video in the server's country.",
        )
        .remedy(
            "Nothing in Tawny can fix this. It would only play through a server (or proxy) \
                 in a country where the uploader allows it.",
        )
    } else if has(&["not a bot"]) {
        PlaybackFailure::new(
            Youtube,
            "YouTube has flagged the server as a bot and is refusing to serve it.",
        )
        .remedy(
            "Check that the pot-provider container is healthy, then wait: these blocks usually \
             lift within hours. If it persists, the server's IP address is blocked and \
             extraction has to go through another one.",
        )
    } else if has(&["private video"]) {
        PlaybackFailure::new(Youtube, "This video is private.").remedy(
            "Only accounts the uploader invited can watch it; Tawny has no YouTube account.",
        )
    } else if has(&[
        "confirm your age",
        "age-restricted",
        "inappropriate for some users",
    ]) {
        PlaybackFailure::new(
            Youtube,
            "YouTube only shows this video to signed-in adults.",
        )
        .remedy("Tawny does not sign in to YouTube, so it cannot play age-restricted videos.")
    } else if has(&["members-only", "members only", "join this channel"]) {
        PlaybackFailure::new(
            Youtube,
            "This video is only for the channel's paying members.",
        )
    } else if has(&[
        "premieres in",
        "live event will begin",
        "premiere will begin",
        "is upcoming",
    ]) {
        PlaybackFailure::new(Youtube, "This video has not started yet.")
            .remedy("Try again once the premiere or live stream begins.")
    } else if has(&["removed by the uploader"]) {
        PlaybackFailure::new(Youtube, "The uploader has removed this video.")
    } else if has(&["account associated with this video has been terminated"]) {
        PlaybackFailure::new(
            Youtube,
            "YouTube terminated the account that uploaded this video.",
        )
    } else if has(&["violating youtube", "copyright", "terms of service"]) {
        PlaybackFailure::new(Youtube, "YouTube has taken this video down.")
    } else if has(&["http error 429", "too many requests"]) {
        PlaybackFailure::new(Youtube, "YouTube is rate-limiting the server.").remedy(
            "Wait a few minutes and try again. Lowering YTDLP_CONCURRENCY makes it less likely.",
        )
        .retryable()
    } else if has(&["video unavailable", "not available", "no longer available"]) {
        // YouTube's catch-all. The watch page shows the same "Video
        // unavailable" to a browser on the server's network, so this is
        // YouTube's decision, not an extraction mistake - but it gives no
        // reason, and a regional or rights block is the usual one.
        PlaybackFailure::new(
            Youtube,
            "YouTube says this video is unavailable and gives no reason.",
        )
        .remedy(
            "It is usually blocked in the server's region or for rights reasons. Nothing in \
                 Tawny can fix that; it may still play on YouTube from somewhere else.",
        )
    } else if has(&[
        "unable to extract",
        "signature",
        "nsig",
        "n challenge",
        "latest version",
        "please report this issue",
    ]) {
        PlaybackFailure::new(
            Setup,
            "yt-dlp could not read YouTube's page; YouTube has probably changed it.",
        )
        .remedy(
            "Update yt-dlp: raise YTDLP_VERSION to the latest release and rebuild the \
                 extractor (docker compose up -d --build extractor).",
        )
    } else if has(&[
        "timed out",
        "unable to download",
        "connection",
        "name resolution",
        "network is unreachable",
    ]) {
        PlaybackFailure::new(Setup, "The server could not reach YouTube.")
            .remedy("Check the server's internet connection, then try again.")
            .retryable()
    } else {
        PlaybackFailure::new(
            Unknown,
            "yt-dlp failed with an error Tawny does not recognise.",
        )
        .remedy("Try again. If it keeps failing, report it with the detail below.")
    };
    failure.detail(reason)
}

/// Build the adaptive source entirely from yt-dlp.
///
/// This is the split: yt-dlp owns extraction, because its URLs are not subject
/// to the gate that kills the other client's after a few MiB, and the byte
/// ranges it does not report are derived from the containers directly. The
/// other extractor keeps metadata, search, channels and comments, where it is
/// faster and needs no subprocess.
pub(crate) async fn ytdlp_playback_source(
    client: &reqwest::Client,
    video_id: &str,
    formats: &[YtdlpFormat],
    known_ranges: &HashMap<(u32, u64), SegmentRanges>,
) -> Option<PlaybackSource> {
    let candidates = formats
        .iter()
        .filter(|format| !format.has_drm.unwrap_or(false))
        .filter(|format| format.is_dash())
        .filter_map(|format| {
            let url = format.url.as_deref().filter(|url| !url.is_empty())?;
            let kind = format.adaptive_kind()?;
            Some((format, format.itag()?, kind, url))
        })
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        return None;
    }

    // A few at a time, not all at once. Every format lives on the same
    // googlevideo host, which speaks HTTP/1.1 only, so each probe in flight is
    // its own connection - and opening two dozen at once gets connection
    // attempts dropped, each one waiting out SYN retries (1s, 3s, 7s) before
    // it gets through or times out. Measured 2026-09-24: all 25 at once took
    // 10s every time; four at a time over reused connections took 0.1-0.5s.
    //
    // Boxed so the stream's type does not carry the closure: unboxed, it is
    // not general enough to cross the `tokio::spawn` the resolve runs in.
    // What the other extractor already told us about the same file needs no
    // probe; see `unambiguous_segment_ranges`. The pair has to be unique on
    // this side too, or it could be any of the languages sharing it.
    let mut pairs = HashMap::<(u32, u64), usize>::new();
    for (format, itag, _, _) in &candidates {
        if let Some(size) = format.filesize {
            *pairs.entry((*itag, size)).or_default() += 1;
        }
    }
    let known = |format: &YtdlpFormat, itag: u32| {
        let pair = (itag, format.filesize?);
        (pairs.get(&pair) == Some(&1))
            .then(|| known_ranges.get(&pair).copied())
            .flatten()
    };

    use futures_util::StreamExt;
    let probes = candidates
        .iter()
        .map(|(format, itag, _, url)| {
            if let Some(ranges) = known(format, *itag) {
                return futures_util::future::ready(Some(ranges)).boxed();
            }
            // Without an exact size the file cannot be told apart from its
            // siblings, so it is probed every time rather than cached.
            let key = format.filesize.map(|size| SegmentRangeKey {
                video_id: video_id.to_string(),
                itag: *itag,
                language: format.language.clone(),
                size,
            });
            probe_segment_ranges(client, key, url).boxed()
        })
        .collect::<Vec<_>>();
    let ranges = futures_util::stream::iter(probes)
        .buffered(SEGMENT_PROBE_CONCURRENCY)
        .collect::<Vec<_>>()
        .await;

    let mut tracks = candidates
        .into_iter()
        .zip(ranges)
        .filter_map(|((format, _, kind, url), ranges)| {
            let ranges = ranges?;
            let mime_type = format.mime_type(kind.clone())?;
            Some(PlaybackTrack {
                kind,
                url: url.to_string(),
                mime_type,
                bitrate: format.tbr.map(|rate| (rate * 1000.0).round() as u64),
                content_length: format.content_length(),
                duration_ms: format.duration_ms,
                width: format.width,
                height: format.height,
                fps: format.fps.map(|fps| fps.round() as u32),
                quality_label: format.format_note.clone(),
                language: format.language.clone(),
                label: None,
                is_default: false,
                init_range: Some(PlaybackByteRange {
                    start: 0,
                    end: ranges.init_end,
                }),
                index_range: Some(PlaybackByteRange {
                    start: ranges.index_start,
                    end: ranges.index_end,
                }),
                request_headers: Vec::new(),
            })
        })
        .collect::<Vec<_>>();
    if tracks.is_empty() {
        return None;
    }
    // Highest quality first, so the manifest lists them the way the ABR manager
    // expects to read them.
    tracks.sort_by_key(|track| {
        std::cmp::Reverse((
            track.height.unwrap_or_default(),
            track.bitrate.unwrap_or_default(),
        ))
    });
    if let Some(first) = tracks.first_mut() {
        first.is_default = true;
    }

    Some(PlaybackSource {
        protocol: PlaybackProtocol::Dash,
        url: String::new(),
        mime_type: None,
        tracks,
        expires_at: None,
        po_token: None,
        quality_label: None,
        request_headers: Vec::new(),
    })
}

/// The HLS master playlist yt-dlp found, as a source of its own.
///
/// Some extractions come back with no DASH formats at all, only the m3u8
/// ladder (the Safari web client's). Its itags (229-234, 269-270, 602-625)
/// match nothing the other extractor lists, so without this the resolve fell
/// through to that extractor's gated URLs: playback started, then stalled on a
/// 403 at about 1:09. The HLS segments are not gated - a 1080p variant of a
/// 40-minute video was fetched at its middle and end with no 403 on
/// 2026-09-24 - so this is the one to play when the DASH ladder is missing.
pub(crate) fn ytdlp_hls_source(formats: &[YtdlpFormat]) -> Option<PlaybackSource> {
    let manifest = formats
        .iter()
        .filter(|format| {
            format
                .protocol
                .as_deref()
                .is_some_and(|protocol| protocol.starts_with("m3u8"))
        })
        .find_map(|format| format.manifest_url.as_deref().filter(|url| !url.is_empty()))?;
    Some(PlaybackSource {
        protocol: PlaybackProtocol::Hls,
        url: manifest.to_string(),
        mime_type: Some("application/vnd.apple.mpegurl".into()),
        po_token: None,
        expires_at: None,
        quality_label: Some("Adaptive HLS".into()),
        request_headers: Vec::new(),
        tracks: Vec::new(),
    })
}

#[cfg(test)]
mod retry_tests {
    use super::*;

    #[test]
    fn busy_and_rate_limited_failures_are_retried_by_the_player() {
        let busy = extractor_status_failure(
            reqwest::StatusCode::TOO_MANY_REQUESTS,
            br#"{"error":"extractor is busy"}"#,
        );
        assert!(busy.retryable);
        assert!(extraction_timed_out().retryable);
        assert!(classify_ytdlp_error("ERROR: HTTP Error 429: Too Many Requests").retryable);
    }

    #[test]
    fn a_video_youtube_will_never_serve_is_not_retried() {
        assert!(!classify_ytdlp_error("ERROR: [youtube] abc: Video unavailable").retryable);
        assert!(!classify_ytdlp_error("ERROR: Sign in to confirm your age").retryable);
    }
}
