use super::*;

impl AppServerState {
    /// A playback session for `video_id`, resolved once and handed out again.
    ///
    /// Resolving costs several seconds, almost all of it yt-dlp, and it is paid
    /// before the first frame. Clients ask ahead - for the video queued next,
    /// and on opening a watch page before its details arrive - so by the time
    /// the player asks, the answer is usually here or already on its way. Asks
    /// for a video whose resolve is still running wait on that one rather than
    /// starting another, which also keeps the sidecar's two extraction slots
    /// free. The resolve runs as its own task, so an ask that gives up (a
    /// client moving on) does not abandon it for the next one.
    ///
    /// `fresh` is for a player whose session stopped working: it discards what
    /// is held and resolves again.
    pub async fn playback_session(
        &self,
        video_id: &str,
        _prefer_sabr: bool,
        fresh: bool,
    ) -> std::result::Result<PlaybackSession, PlaybackFailure> {
        let resolve = self.shared_playback_resolve(video_id, fresh);
        let resolved = resolve.clone().await;
        if !resolved.as_ref().is_ok_and(|resolved| resolved.reusable) {
            self.forget_playback_resolve(video_id, &resolve);
        }
        let resolved = resolved?;
        Ok(self.proxy_session(resolved.session).await)
    }

    pub(crate) fn shared_playback_resolve(
        &self,
        video_id: &str,
        fresh: bool,
    ) -> SharedPlaybackResolve {
        let mut sessions = self
            .playback_sessions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let now = std::time::Instant::now();
        sessions.retain(|_, cached| now.duration_since(cached.started_at) < PLAYBACK_SESSION_TTL);
        if fresh {
            sessions.remove(video_id);
        }
        if let Some(cached) = sessions.get(video_id) {
            return cached.resolve.clone();
        }
        if sessions.len() >= PLAYBACK_SESSION_LIMIT
            && let Some(oldest) = sessions
                .iter()
                .min_by_key(|(_, cached)| cached.started_at)
                .map(|(id, _)| id.clone())
        {
            sessions.remove(&oldest);
        }
        let state = self.clone();
        let id = video_id.to_string();
        let task = tokio::spawn(async move { state.resolve_playback(&id).await });
        let resolve = async move {
            task.await.unwrap_or_else(|error| {
                Err(PlaybackFailure::new(
                    PlaybackFailureOrigin::Bug,
                    "The playback resolver crashed.",
                )
                .remedy("Try again. If it keeps happening, report it with the detail below.")
                .detail(error.to_string()))
            })
        }
        .boxed()
        .shared();
        sessions.insert(
            video_id.to_string(),
            CachedPlayback {
                started_at: now,
                resolve: resolve.clone(),
            },
        );
        resolve
    }

    /// Drop a resolve that is not worth handing out again - unless it has
    /// already been replaced, in which case the replacement stays.
    pub(crate) fn forget_playback_resolve(&self, video_id: &str, resolve: &SharedPlaybackResolve) {
        let mut sessions = self
            .playback_sessions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if sessions
            .get(video_id)
            .is_some_and(|cached| cached.resolve.ptr_eq(resolve))
        {
            sessions.remove(video_id);
        }
    }

    /// Resolve playback from scratch. The session is not yet proxied.
    pub(crate) async fn resolve_playback(
        &self,
        video_id: &str,
    ) -> std::result::Result<ResolvedPlayback, PlaybackFailure> {
        let fallback_url = embed_url(video_id);
        let mut provider_sources: Vec<PlaybackSource> = Vec::new();
        // Run both extractors together: rustypipe supplies the stream layout
        // (byte ranges, codecs, languages) and yt-dlp supplies URLs that are
        // not subject to the iOS client's 403 gate.
        let started = std::time::Instant::now();
        let query = self.youtube.query();
        let (player, ytdlp_result) = tokio::join!(
            youtube_call(
                query.player_from_clients(video_id, PLAYER_CLIENTS),
                "extract YouTube player"
            ),
            self.ytdlp_formats(video_id),
        );
        let extracted_at = started.elapsed();
        // Why yt-dlp gave nothing, kept for the error the viewer sees if no
        // other ungated source turns up either.
        let ytdlp_failure = ytdlp_result.as_ref().err().cloned();
        let known_ranges = player
            .as_ref()
            .map(rustypipe_segment_ranges)
            .unwrap_or_default();
        let mut ytdlp_formats = ytdlp_result.unwrap_or_default();

        // yt-dlp owns the adaptive source. The extractor is consulted only for
        // what yt-dlp does not produce: the HLS manifest a live stream needs.
        // Its own stream URLs are gated, so they are never played.
        let mut ytdlp_source =
            ytdlp_playback_source(&self.http, video_id, &ytdlp_formats, &known_ranges).await;
        let ranged_at = started.elapsed();
        // Now and then yt-dlp hands out URLs that serve the first minute or so
        // and then answer 403 to every later range: the video starts, plays its
        // buffered head, and dies. Which extraction gets gated is luck - on
        // 2026-09-24 the same long video came back gated 3 times in 18 tries,
        // across both provider versions - so one more extraction usually cures
        // it, and costs nothing when the first one was sound.
        let mut ytdlp_verified = None;
        if let Some(source) = ytdlp_source.as_ref() {
            let sound = self.adaptive_tail_is_available(source).await;
            ytdlp_verified = Some(sound);
            if !sound {
                crate::telemetry::diagnostic(crate::telemetry::Diagnostic::PlaybackTailRetry);
                let retried_formats = self.ytdlp_formats(video_id).await.unwrap_or_default();
                if let Some(retried) =
                    ytdlp_playback_source(&self.http, video_id, &retried_formats, &known_ranges)
                        .await
                    && self.adaptive_tail_is_available(&retried).await
                {
                    ytdlp_source = Some(retried);
                    ytdlp_formats = retried_formats;
                    ytdlp_verified = Some(true);
                }
            }
        }
        let verified_at = started.elapsed();
        // A session known to be gated is still handed to this player - its head
        // plays, and the transport refreshes when it fails - but it is not kept
        // for the next ask, which would otherwise inherit the same dead URLs for
        // the whole session TTL.
        let reusable = ytdlp_source.is_some() && ytdlp_verified != Some(false);
        // Identifies the yt-dlp source after sorting, so its tail is not
        // probed a second time below.
        let verified_track_url = ytdlp_source
            .as_ref()
            .filter(|_| ytdlp_verified == Some(true))
            .and_then(|source| source.tracks.first())
            .map(|track| track.url.clone());
        if let Some(source) = ytdlp_source {
            crate::telemetry::diagnostic(crate::telemetry::Diagnostic::PlaybackTracksResolved);
            provider_sources.push(source);
        } else if let Some(source) = ytdlp_hls_source(&ytdlp_formats) {
            crate::telemetry::diagnostic(crate::telemetry::Diagnostic::PlaybackHlsFallback);
            provider_sources.push(source);
        }

        let ytdlp_urls = Self::ytdlp_url_map(&ytdlp_formats);
        let youtube_sources = match player {
            // Only tracks yt-dlp covers, plus the extractor's manifests.
            Ok(player) => rusty_playback_sources(&player, &ytdlp_urls),
            Err(_) => {
                crate::telemetry::diagnostic(
                    crate::telemetry::Diagnostic::PlaybackExtractionFailed,
                );
                Vec::new()
            }
        };
        for source in youtube_sources {
            if !provider_sources
                .iter()
                .any(|existing| existing.url == source.url)
            {
                provider_sources.push(source);
            }
        }
        // Last resort when the default client order yielded nothing at all.
        //
        // Measured 2026-08-12 against rustypipe 0.11.4: both of these clients
        // fail signature deobfuscation ("could not extract sig fn name"), so
        // this loop is currently inert and only costs a request when nothing
        // else was found. It is kept because it starts working again whenever
        // upstream refreshes the deobfuscator.
        if provider_sources.is_empty() {
            for client in [ClientType::Android, ClientType::Tv] {
                if let Ok(player) = youtube_call(
                    self.youtube.query().player_from_client(video_id, client),
                    "extract alternate YouTube player",
                )
                .await
                {
                    for source in rusty_playback_sources(&player, &ytdlp_urls) {
                        if !provider_sources
                            .iter()
                            .any(|existing| existing.url == source.url)
                        {
                            provider_sources.push(source);
                        }
                    }
                }
            }
        }

        provider_sources.sort_by_key(playback_source_priority);
        let primary_verified = verified_track_url.is_some()
            && provider_sources
                .first()
                .and_then(|source| source.tracks.first())
                .map(|track| &track.url)
                == verified_track_url.as_ref();

        use futures_util::StreamExt;

        // The tail probe is a reachability hint with a short timeout, not proof,
        // so it may only reorder — never discard. An earlier version treated it
        // as a filter, which could empty the list and strand playback on the
        // embed. Probe in priority order, stop at the first source that answers,
        // and cap the whole phase: this runs before the user sees any video, so
        // it must never become the reason playback feels slow to start. If the
        // budget expires the priority order simply stands.
        //
        // Every source is probed at once, and the answer is in as soon as the
        // earliest source that answered has nothing still pending ahead of it:
        // the usual case - the first source confirming - costs one round trip,
        // and none at all when that source is the yt-dlp one verified above.
        let promoted = if primary_verified {
            Some(0)
        } else {
            tokio::time::timeout(Duration::from_secs(6), async {
                let mut probes = provider_sources
                    .iter()
                    .enumerate()
                    .map(|(index, source)| async move {
                        (index, self.adaptive_tail_is_available(source).await)
                    })
                    .collect::<futures_util::stream::FuturesUnordered<_>>();
                let mut answers = vec![None; provider_sources.len()];
                while let Some((index, available)) = probes.next().await {
                    answers[index] = Some(available);
                    for (index, answer) in answers.iter().enumerate() {
                        match answer {
                            Some(true) => return Some(index),
                            Some(false) => continue,
                            None => break,
                        }
                    }
                }
                None
            })
            .await
            .unwrap_or(None)
        };
        if let Some(index) = promoted.filter(|index| *index > 0) {
            let verified = provider_sources.remove(index);
            provider_sources.insert(0, verified);
        }

        if !provider_sources.is_empty() {
            // Cumulative, so the slow phase is the one with the big step. This
            // all runs before the first frame, and a slow start is otherwise
            // impossible to attribute from the log.
            crate::telemetry::playback_resolved(
                provider_sources.len(),
                extracted_at,
                ranged_at,
                verified_at,
                started.elapsed(),
            );
            let primary = provider_sources.remove(0);
            return Ok(ResolvedPlayback {
                session: PlaybackSession {
                    primary,
                    alternatives: provider_sources,
                    fallback_url,
                },
                reusable,
            });
        }

        // An error, not a session: the player shows this message, and the
        // viewer learns the cause (a stopped sidecar, say) instead of meeting
        // gated URLs that freeze a minute in.
        let failure = no_stream_failure(ytdlp_failure, ytdlp_formats.len());
        crate::telemetry::diagnostic(crate::telemetry::Diagnostic::PlaybackUnavailable);
        Err(failure)
    }
}
/// Ordering for playback sources, lowest first.
///
/// HLS leads deliberately. YouTube gates raw googlevideo (GVS) range requests
/// behind a PO token: the CDN serves roughly the first minute of content and
/// then answers 403 for further ranges. That budget is measured in bytes, so a
/// high-bitrate ladder burns through it in well under a minute, which surfaces
/// as quality collapsing and then playback stopping around 1:05-1:10. The
/// segments behind the HLS manifest are not gated the same way — a full
/// 19 Mbps variant was drained end to end (68 MiB) with no 403 on 2026-08-12,
/// while the adaptive ladder died mid-playback in the browser.
///
/// The adaptive track set stays as the next choice: it gives per-language audio
/// and finer quality control, and it plays fine for content whose bitrate never
/// exhausts the budget.
pub(crate) fn playback_source_priority(source: &PlaybackSource) -> u8 {
    match source.protocol {
        PlaybackProtocol::Hls => 0,
        PlaybackProtocol::Sabr if !source.tracks.is_empty() => 1,
        PlaybackProtocol::Dash if !source.tracks.is_empty() => 1,
        PlaybackProtocol::Progressive => 2,
        PlaybackProtocol::Dash => 3,
        PlaybackProtocol::Sabr => 4,
        PlaybackProtocol::EmbedFallback => 4,
    }
}

/// A track yt-dlp does not cover is dropped, never played from the
/// extractor's own URL. Those URLs are gated: they serve about a minute and
/// then answer 403. Playing them when yt-dlp was down made a stopped sidecar
/// look like a mysterious freeze at ~1:01 instead of an error that names the
/// cause (2026-09-25).
pub(crate) fn resolved_stream_url(
    ytdlp_urls: &HashMap<u32, (String, Option<u64>)>,
    itag: u32,
    size: Option<u64>,
) -> Option<String> {
    match ytdlp_urls.get(&itag) {
        Some((url, reported)) if reported.is_none() || size.is_none() || *reported == size => {
            Some(url.clone())
        }
        _ => None,
    }
}

pub(crate) fn rusty_playback_sources(
    player: &rustypipe::model::VideoPlayer,
    ytdlp_urls: &HashMap<u32, (String, Option<u64>)>,
) -> Vec<PlaybackSource> {
    let expires_at = Some(player.valid_until.to_string());
    let mut streams = player.video_streams.clone();
    streams.sort_by_key(|stream| {
        let is_mp4 = stream.mime.contains("mp4");
        (is_mp4, stream.height.min(1080), stream.bitrate)
    });
    streams.reverse();
    let mut sources = streams
        .into_iter()
        .filter(|stream| !stream.url.is_empty())
        .filter_map(|stream| {
            Some(PlaybackSource {
                protocol: PlaybackProtocol::Progressive,
                url: resolved_stream_url(ytdlp_urls, stream.itag, stream.size)?,
                mime_type: Some(stream.mime),
                po_token: None,
                expires_at: expires_at.clone(),
                quality_label: Some(stream.quality),
                request_headers: Vec::new(),
                tracks: Vec::new(),
            })
        })
        .collect::<Vec<_>>();

    let mut adaptive_tracks = player
        .video_only_streams
        .iter()
        .filter(|stream| !stream.url.is_empty() && stream.drm_track_type.is_none())
        .filter_map(|stream| {
            Some(PlaybackTrack {
                kind: PlaybackTrackKind::Video,
                url: resolved_stream_url(ytdlp_urls, stream.itag, stream.size)?,
                mime_type: stream.mime.clone(),
                bitrate: Some(u64::from(stream.bitrate)),
                content_length: stream.size,
                duration_ms: stream.duration_ms.map(u64::from),
                width: Some(stream.width),
                height: Some(stream.height),
                fps: Some(u32::from(stream.fps)),
                quality_label: Some(stream.quality.clone()),
                language: None,
                label: None,
                is_default: false,
                init_range: stream.init_range.as_ref().map(|range| PlaybackByteRange {
                    start: u64::from(range.start),
                    end: u64::from(range.end),
                }),
                index_range: stream.index_range.as_ref().map(|range| PlaybackByteRange {
                    start: u64::from(range.start),
                    end: u64::from(range.end),
                }),
                request_headers: Vec::new(),
            })
        })
        .filter(|track| track.init_range.is_some() && track.index_range.is_some())
        .collect::<Vec<_>>();
    adaptive_tracks.extend(
        player
            .audio_streams
            .iter()
            .filter(|stream| !stream.url.is_empty() && stream.drm_track_type.is_none())
            .filter_map(|stream| {
                let track_info = stream.track.as_ref();
                Some(PlaybackTrack {
                    kind: PlaybackTrackKind::Audio,
                    url: resolved_stream_url(ytdlp_urls, stream.itag, Some(stream.size))?,
                    mime_type: stream.mime.clone(),
                    bitrate: Some(u64::from(stream.bitrate)),
                    content_length: Some(stream.size),
                    duration_ms: stream.duration_ms.map(u64::from),
                    width: None,
                    height: None,
                    fps: None,
                    quality_label: Some(format!("{} kbps", stream.bitrate / 1_000)),
                    language: track_info.and_then(|track| track.lang.clone()),
                    label: track_info.map(|track| track.lang_name.clone()),
                    is_default: track_info.is_none_or(|track| track.is_default),
                    init_range: stream.init_range.as_ref().map(|range| PlaybackByteRange {
                        start: u64::from(range.start),
                        end: u64::from(range.end),
                    }),
                    index_range: stream.index_range.as_ref().map(|range| PlaybackByteRange {
                        start: u64::from(range.start),
                        end: u64::from(range.end),
                    }),
                    request_headers: Vec::new(),
                })
            })
            .filter(|track| track.init_range.is_some() && track.index_range.is_some()),
    );
    if adaptive_tracks
        .iter()
        .any(|track| track.kind == PlaybackTrackKind::Video)
        && adaptive_tracks
            .iter()
            .any(|track| track.kind == PlaybackTrackKind::Audio)
    {
        adaptive_tracks.sort_by_key(|track| {
            (
                track.kind == PlaybackTrackKind::Audio,
                track.height.unwrap_or_default(),
                track.bitrate.unwrap_or_default(),
            )
        });
        let quality_label = adaptive_tracks
            .iter()
            .filter(|track| track.kind == PlaybackTrackKind::Video)
            .filter_map(|track| track.height)
            .max()
            .map(|height| format!("Adaptive up to {height}p"));
        let url = adaptive_tracks
            .iter()
            .find(|track| track.kind == PlaybackTrackKind::Video)
            .map(|track| track.url.clone())
            .unwrap_or_default();
        sources.push(PlaybackSource {
            protocol: PlaybackProtocol::Dash,
            url,
            mime_type: Some("application/dash+xml".into()),
            po_token: None,
            expires_at: expires_at.clone(),
            quality_label,
            request_headers: Vec::new(),
            tracks: adaptive_tracks,
        });
    }
    if let Some(hls) = &player.hls_manifest_url {
        sources.push(PlaybackSource {
            protocol: PlaybackProtocol::Hls,
            url: hls.clone(),
            mime_type: Some("application/vnd.apple.mpegurl".into()),
            po_token: None,
            expires_at: expires_at.clone(),
            quality_label: Some("Adaptive HLS".into()),
            request_headers: Vec::new(),
            tracks: Vec::new(),
        });
    }
    if let Some(dash) = &player.dash_manifest_url {
        sources.push(PlaybackSource {
            protocol: PlaybackProtocol::Dash,
            url: dash.clone(),
            mime_type: Some("application/dash+xml".into()),
            po_token: None,
            expires_at,
            quality_label: Some("Adaptive DASH".into()),
            request_headers: Vec::new(),
            tracks: Vec::new(),
        });
    }
    sources.dedup_by(|left, right| left.url == right.url);
    sources
}
