use super::*;

impl AppServerState {
    pub(crate) async fn register_proxy_target(
        &self,
        url: String,
        request_headers: Vec<crate::models::PlaybackRequestHeader>,
        ttl: Duration,
    ) -> String {
        let mut hasher = DefaultHasher::new();
        self.websub_secret.hash(&mut hasher);
        url.hash(&mut hasher);
        let counter = &self.proxy_counter;
        let mint = move || {
            let count = counter.fetch_add(1, Ordering::Relaxed);
            count.hash(&mut hasher);
            format!("{:x}{count:x}", hasher.finish())
        };
        let token = self.proxy_targets.write().await.token_for(
            url,
            request_headers,
            ttl,
            std::time::Instant::now(),
            mint,
        );
        format!("{}/api/v1/playback/proxy/{token}", self.public_url)
    }

    pub(crate) async fn proxy_source(&self, mut source: PlaybackSource) -> PlaybackSource {
        if matches!(source.protocol, PlaybackProtocol::EmbedFallback) {
            return source;
        }
        let ttl = source
            .expires_at
            .as_ref()
            .map(|_| Duration::from_secs(90 * 60))
            .unwrap_or_else(|| Duration::from_secs(2 * 60 * 60));
        if source.tracks.is_empty() {
            source.url = self
                .register_proxy_target(source.url, source.request_headers.clone(), ttl)
                .await;
        } else {
            for track in &mut source.tracks {
                let headers = if track.request_headers.is_empty() {
                    source.request_headers.clone()
                } else {
                    track.request_headers.clone()
                };
                track.url = self
                    .register_proxy_target(track.url.clone(), headers, ttl)
                    .await;
                track.request_headers.clear();
            }
            if let Some(video) = source
                .tracks
                .iter()
                .find(|track| track.kind == PlaybackTrackKind::Video)
            {
                source.url = video.url.clone();
            }
        }
        source.request_headers.clear();
        source
    }

    pub(crate) async fn proxy_session(&self, mut session: PlaybackSession) -> PlaybackSession {
        session.primary = self.proxy_source(session.primary).await;
        let mut proxied = Vec::with_capacity(session.alternatives.len());
        for source in session.alternatives {
            proxied.push(self.proxy_source(source).await);
        }
        session.alternatives = proxied;
        session
    }

    pub(crate) async fn proxy_captions(&self, mut details: VideoDetails) -> VideoDetails {
        for caption in &mut details.captions {
            if caption.url.starts_with(&self.public_url) {
                continue;
            }
            caption.url = self
                .register_proxy_target(
                    caption.url.clone(),
                    Vec::new(),
                    Duration::from_secs(2 * 60 * 60),
                )
                .await;
        }
        details
    }
}
pub(crate) fn proxy_error(status: StatusCode, message: &str) -> Response {
    let mut response = Response::new(Body::from(message.to_string()));
    *response.status_mut() = status;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    response
}

/// Identify a media container from its leading bytes.
///
/// YouTube serves every HLS segment as `application/octet-stream`, which leaves
/// the player unable to tell packed AAC from MPEG-TS. Shaka then assumes the
/// audio rendition is fMP4, builds an `audio/mp4` SourceBuffer, and the ADTS
/// bytes it appends decode to nothing: the video buffer fills, the audio buffer
/// stays empty, and the element sits at readyState 0 forever. Reporting the
/// real type is what lets the audio track play at all.
pub(crate) fn sniff_media_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"ID3") {
        return Some("audio/aac");
    }
    // ADTS sync word: 12 set bits, then layer/protection bits.
    if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] & 0xF6 == 0xF0 {
        return Some("audio/aac");
    }
    if bytes.first() == Some(&0x47) {
        return Some("video/mp2t");
    }
    if bytes.len() >= 8 && &bytes[4..8] == b"ftyp" {
        return Some("video/mp4");
    }
    if bytes.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) {
        return Some("video/webm");
    }
    if bytes.starts_with(b"OggS") {
        return Some("audio/ogg");
    }
    None
}

/// Whether the URL's path — ignoring query and fragment — ends with `suffix`.
///
/// Substring matching is wrong here. YouTube derives HLS segment URLs from the
/// playlist path, so a segment looks like `.../file/index.m3u8/sq/3/goap/...`
/// and *contains* `.m3u8` while being MPEG-TS media. Treating those as
/// playlists rewrites the media into a list of proxy URLs, and playback stalls
/// at readyState 0 with the manifest parsed but nothing ever buffered.
pub(crate) fn url_path_ends_with(url: &str, suffix: &str) -> bool {
    url.split(['?', '#'])
        .next()
        .unwrap_or(url)
        .ends_with(suffix)
}

pub(crate) fn resolved_media_url(base: &str, value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || value.starts_with("data:") || value.starts_with("blob:") {
        return None;
    }
    reqwest::Url::parse(value)
        .or_else(|_| reqwest::Url::parse(base)?.join(value))
        .ok()
        .map(Into::into)
}

pub(crate) async fn proxy_manifest_url(
    state: &AppServerState,
    base: &str,
    value: &str,
    request_headers: &[crate::models::PlaybackRequestHeader],
) -> Option<String> {
    let url = resolved_media_url(base, value)?;
    Some(
        state
            .register_proxy_target(url, request_headers.to_vec(), Duration::from_secs(90 * 60))
            .await,
    )
}

pub(crate) async fn rewrite_hls_manifest(
    state: &AppServerState,
    base: &str,
    manifest: &str,
    request_headers: &[crate::models::PlaybackRequestHeader],
) -> String {
    let mut output = String::with_capacity(manifest.len() + 256);
    for line in manifest.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with('#') && !trimmed.is_empty() {
            if let Some(url) = proxy_manifest_url(state, base, trimmed, request_headers).await {
                output.push_str(&url);
            } else {
                output.push_str(line);
            }
        } else {
            let mut rewritten = line.to_string();
            let mut search_from = 0;
            while let Some(offset) = rewritten[search_from..].find("URI=\"") {
                let value_start = search_from + offset + 5;
                let Some(value_end_offset) = rewritten[value_start..].find('"') else {
                    break;
                };
                let value_end = value_start + value_end_offset;
                let value = rewritten[value_start..value_end].to_string();
                let Some(url) = proxy_manifest_url(state, base, &value, request_headers).await
                else {
                    break;
                };
                rewritten.replace_range(value_start..value_end, &url);
                search_from = value_start + url.len();
            }
            output.push_str(&rewritten);
        }
        output.push('\n');
    }
    output
}

pub(crate) async fn rewrite_dash_manifest(
    state: &AppServerState,
    base: &str,
    manifest: &str,
    request_headers: &[crate::models::PlaybackRequestHeader],
) -> String {
    let mut output = manifest.to_string();
    let mut search_from = 0;
    while let Some(start_offset) = output[search_from..].find("<BaseURL>") {
        let value_start = search_from + start_offset + "<BaseURL>".len();
        let Some(end_offset) = output[value_start..].find("</BaseURL>") else {
            break;
        };
        let value_end = value_start + end_offset;
        let value = output[value_start..value_end].to_string();
        let Some(url) = proxy_manifest_url(state, base, &value, request_headers).await else {
            search_from = value_end + "</BaseURL>".len();
            continue;
        };
        output.replace_range(value_start..value_end, &url);
        search_from = value_start + url.len() + "</BaseURL>".len();
    }
    output
}

pub(crate) async fn request_playback_upstream(
    state: &AppServerState,
    target: &ProxyTarget,
    request_headers: &HeaderMap,
) -> std::result::Result<reqwest::Response, reqwest::Error> {
    let mut request = state
        .media_http
        .get(&target.url)
        .header(reqwest::header::ACCEPT_ENCODING, "identity");
    for configured in &target.request_headers {
        if let (Ok(name), Ok(value)) = (
            reqwest::header::HeaderName::from_bytes(configured.name.as_bytes()),
            reqwest::header::HeaderValue::from_str(&configured.value),
        ) {
            request = request.header(name, value);
        }
    }
    for name in [header::RANGE, header::IF_RANGE, header::IF_NONE_MATCH] {
        if let Some(value) = request_headers.get(&name) {
            request = request.header(name, value.clone());
        }
    }
    request.send().await
}

/// The first byte, and the last if it is given, of a `Range: bytes=a-b` header.
pub(crate) fn requested_byte_range(headers: &HeaderMap) -> Option<(u64, Option<u64>)> {
    let value = headers.get(header::RANGE)?.to_str().ok()?.trim();
    let (start, end) = value.strip_prefix("bytes=")?.split_once('-')?;
    let start = start.trim().parse().ok()?;
    let end = match end.trim() {
        "" => None,
        end => Some(end.parse().ok()?),
    };
    Some((start, end))
}

pub(crate) const RANGE_RESUME_ATTEMPTS: u32 = 3;

/// A ranged upstream body that picks up where it stopped if the CDN drops it.
///
/// Each resume asks for only the bytes that are still missing, so the browser
/// sees one continuous body. If the resumes run out, the stream ends with an
/// error. Content-Length is set, so the browser treats the short body as a
/// failed request and does not append a partial segment.
pub(crate) fn resumable_range_stream(
    state: AppServerState,
    target: ProxyTarget,
    request_headers: HeaderMap,
    upstream: reqwest::Response,
) -> futures_util::stream::BoxStream<'static, std::result::Result<Bytes, std::io::Error>> {
    use futures_util::StreamExt;
    type UpstreamBody =
        futures_util::stream::BoxStream<'static, std::result::Result<Bytes, String>>;
    fn upstream_body(response: reqwest::Response) -> UpstreamBody {
        response
            .bytes_stream()
            .map(|chunk| chunk.map_err(|error| error.to_string()))
            .boxed()
    }
    fn failed_body(reason: String) -> UpstreamBody {
        futures_util::stream::once(async move { Err(reason) }).boxed()
    }
    struct Resume {
        state: AppServerState,
        target: ProxyTarget,
        request_headers: HeaderMap,
        range: Option<(u64, Option<u64>)>,
        sent: u64,
        attempts_left: u32,
        body: Option<UpstreamBody>,
    }
    let range = requested_byte_range(&request_headers);
    let resume = Resume {
        state,
        target,
        request_headers,
        range,
        sent: 0,
        attempts_left: RANGE_RESUME_ATTEMPTS,
        body: Some(upstream_body(upstream)),
    };
    futures_util::stream::unfold(resume, |mut resume| async move {
        loop {
            let body = resume.body.as_mut()?;
            let failure = match body.next().await {
                Some(Ok(chunk)) => {
                    resume.sent += chunk.len() as u64;
                    return Some((Ok(chunk), resume));
                }
                None => return None,
                Some(Err(error)) => error,
            };
            let Some((start, end)) = resume.range.filter(|_| resume.attempts_left > 0) else {
                resume.body = None;
                return Some((Err(std::io::Error::other(failure)), resume));
            };
            let attempt = RANGE_RESUME_ATTEMPTS - resume.attempts_left;
            resume.attempts_left -= 1;
            tokio::time::sleep(Duration::from_millis(150 * u64::from(attempt + 1))).await;
            let next = start + resume.sent;
            let range = match end {
                Some(end) => format!("bytes={next}-{end}"),
                None => format!("bytes={next}-"),
            };
            let mut headers = resume.request_headers.clone();
            let Ok(value) = HeaderValue::from_str(&range) else {
                resume.body = None;
                return Some((Err(std::io::Error::other(failure)), resume));
            };
            headers.insert(header::RANGE, value);
            // A resume that is not a matching 206 would splice the wrong bytes
            // into the segment, so it counts as another failure instead.
            resume.body = Some(
                match request_playback_upstream(&resume.state, &resume.target, &headers).await {
                    Ok(response) if response.status() == StatusCode::PARTIAL_CONTENT => {
                        upstream_body(response)
                    }
                    Ok(response) => failed_body(format!("resume answered {}", response.status())),
                    Err(error) => failed_body(error.to_string()),
                },
            );
        }
    })
    .boxed()
}

pub async fn playback_proxy(
    Extension(state): Extension<AppServerState>,
    Path(token): Path<String>,
    request_headers: HeaderMap,
) -> Response {
    let now = std::time::Instant::now();
    let (target, needs_touch) = {
        let targets = state.proxy_targets.read().await;
        (
            targets.get(&token).cloned(),
            targets.needs_touch(&token, now),
        )
    };
    let Some(target) = target else {
        return media_proxy_error(StatusCode::NOT_FOUND, "Playback session was not found");
    };
    if needs_touch && !state.proxy_targets.write().await.touch(&token, now) {
        state.proxy_targets.write().await.remove(&token);
        return media_proxy_error(
            StatusCode::GONE,
            "Playback session expired; resolve it again",
        );
    }

    let range_requested = request_headers.contains_key(header::RANGE);
    // A refusal here is usually transient: the CDN gates a burst of ranged
    // requests and clears within a second or so. Passing the 403 straight
    // through instead makes it worse, because the player answers with its own
    // four attempts per segment across every segment in flight, and that
    // burst is what keeps the gate shut. Absorbing it here turns a retry
    // storm into a few paced requests.
    let mut upstream = match request_playback_upstream(&state, &target, &request_headers).await {
        Ok(response) => response,
        Err(_) => {
            return media_proxy_error(StatusCode::BAD_GATEWAY, "Media source is unavailable");
        }
    };
    for attempt in 0..3u32 {
        if !matches!(
            upstream.status(),
            StatusCode::FORBIDDEN | StatusCode::TOO_MANY_REQUESTS
        ) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200 * u64::from(attempt + 1) * 2)).await;
        match request_playback_upstream(&state, &target, &request_headers).await {
            Ok(response) => upstream = response,
            Err(_) => break,
        }
    }
    let status = upstream.status();
    let upstream_headers = upstream.headers().clone();
    let content_type = upstream_headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    // Content type is the trustworthy signal; the path suffix is only a
    // fallback for upstreams that serve a playlist as a generic byte stream.
    let generic_type = content_type.is_empty() || content_type.contains("octet-stream");
    let is_hls = content_type.contains("mpegurl")
        || (generic_type && url_path_ends_with(&target.url, ".m3u8"));
    let is_dash = content_type.contains("dash+xml")
        || (generic_type && url_path_ends_with(&target.url, ".mpd"));
    let mut sniffed_type: Option<&'static str> = None;
    let is_vtt = content_type.contains("text/vtt")
        || target.url.contains("fmt=vtt")
        || target.url.contains("format=vtt");

    let body = if is_hls || is_dash {
        let bytes = match upstream.bytes().await {
            Ok(bytes) => bytes,
            Err(_) => {
                return media_proxy_error(StatusCode::BAD_GATEWAY, "Media manifest is unavailable");
            }
        };
        let text = String::from_utf8_lossy(&bytes);
        let rewritten = if is_hls {
            rewrite_hls_manifest(&state, &target.url, &text, &target.request_headers).await
        } else {
            rewrite_dash_manifest(&state, &target.url, &text, &target.request_headers).await
        };
        Body::from(rewritten)
    } else if is_vtt {
        let bytes = match upstream.bytes().await {
            Ok(bytes) => bytes,
            Err(_) => {
                return media_proxy_error(StatusCode::BAD_GATEWAY, "Caption track is unavailable");
            }
        };
        Body::from(center_vtt_cues(&String::from_utf8_lossy(&bytes)))
    } else if range_requested && status == StatusCode::PARTIAL_CONTENT {
        // Stream the range; never hold the headers until all of it has
        // arrived. Buffering a 4K segment (~8 MB) first delayed the headers
        // past the player's 8 s connection timeout whenever the server's link
        // was busy. The player then aborted with zero bytes, so ABR never got
        // a throughput sample and kept asking for the same 4K segment. Every
        // abandoned request also kept downloading on the server, which slowed
        // the next one. A body that fails midway is resumed from the byte it
        // reached, so MediaSource still gets one whole segment.
        use futures_util::StreamExt;
        let mut stream = resumable_range_stream(
            state.clone(),
            target.clone(),
            request_headers.clone(),
            upstream,
        );
        match stream.next().await {
            Some(Ok(head)) => {
                if generic_type {
                    sniffed_type = sniff_media_type(&head);
                }
                let head = futures_util::stream::once(async move { Ok::<_, std::io::Error>(head) });
                Body::from_stream(head.chain(stream))
            }
            Some(Err(_)) => {
                return media_proxy_error(StatusCode::BAD_GATEWAY, "Media range is unavailable");
            }
            None => Body::empty(),
        }
    } else {
        // Peek the first chunk so the container can be identified, then put it
        // back at the front of the stream. Buffering the whole body instead
        // would stall progressive playback of a multi-hundred-megabyte file.
        use futures_util::StreamExt;
        let mut stream = upstream.bytes_stream();
        match stream.next().await {
            Some(Ok(head)) => {
                if generic_type {
                    sniffed_type = sniff_media_type(&head);
                }
                let head = futures_util::stream::once(async move { Ok::<_, reqwest::Error>(head) });
                Body::from_stream(head.chain(stream))
            }
            Some(Err(_)) => {
                return media_proxy_error(StatusCode::BAD_GATEWAY, "Media stream is unavailable");
            }
            None => Body::empty(),
        }
    };
    let mut response = Response::new(body);
    *response.status_mut() = status;
    for name in [
        header::CONTENT_TYPE,
        header::CONTENT_RANGE,
        header::ACCEPT_RANGES,
        header::CACHE_CONTROL,
        header::ETAG,
        header::LAST_MODIFIED,
        header::CONTENT_DISPOSITION,
    ] {
        if let Some(value) = upstream_headers.get(&name) {
            response.headers_mut().insert(name, value.clone());
        }
    }
    if let Some(kind) = sniffed_type
        && let Ok(value) = HeaderValue::from_str(kind)
    {
        response.headers_mut().insert(header::CONTENT_TYPE, value);
    }
    if !is_hls
        && !is_dash
        && !is_vtt
        && let Some(value) = upstream_headers.get(header::CONTENT_LENGTH)
    {
        response
            .headers_mut()
            .insert(header::CONTENT_LENGTH, value.clone());
    }
    add_media_cors_headers(&mut response);
    response
}

pub(crate) fn add_media_cors_headers(response: &mut Response) {
    let headers = response.headers_mut();
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    headers.insert(
        header::ACCESS_CONTROL_EXPOSE_HEADERS,
        HeaderValue::from_static(
            "Accept-Ranges, Content-Length, Content-Range, ETag, Last-Modified",
        ),
    );
    headers.insert(
        header::HeaderName::from_static("cross-origin-resource-policy"),
        HeaderValue::from_static("cross-origin"),
    );
    // Chromium's Private Network Access preflight uses this when a secure
    // WebView asset origin reads the loopback development server.
    headers.insert(
        header::HeaderName::from_static("access-control-allow-private-network"),
        HeaderValue::from_static("true"),
    );
}

pub(crate) fn media_proxy_error(status: StatusCode, message: &str) -> Response {
    let mut response = proxy_error(status, message);
    add_media_cors_headers(&mut response);
    response
}

pub async fn playback_proxy_options() -> Response {
    let mut response = Response::new(Body::empty());
    *response.status_mut() = StatusCode::NO_CONTENT;
    add_media_cors_headers(&mut response);
    response.headers_mut().insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET, OPTIONS"),
    );
    response.headers_mut().insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static("Range, Content-Type"),
    );
    response
}
