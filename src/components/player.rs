use crate::{
    api::{
        get_comments_page, get_sponsor_segments, get_video_details, get_video_progress, get_videos,
        resolve_playback,
    },
    app::Route,
    models::{
        AudioTrackOption, CaptionTrack, PlaybackFailure, PlaybackFailureOrigin, PlaybackProtocol,
        PlaybackSession, PlaybackSource, SponsorAction, SponsorBlockSettings, SponsorSegment,
        Video, VideoChapter, VideoComment, VideoDetails, VideoPreviewFrames,
    },
    state::AppState,
};
use dioxus::prelude::*;
use dioxus_icons::lucide::{
    Captions, Check, ChevronLeft, ChevronRight, Clock, EyeOff, Heart, Languages, ListVideo,
    Maximize2, MessageSquare, Minimize2, PanelTopClose, PanelTopOpen, Pause, PictureInPicture,
    Play, RotateCcw, RotateCw, Settings2, Share2, ThumbsDown, ThumbsUp, ToggleLeft, ToggleRight,
    Volume2, VolumeX, X,
};
use g3_cache::use_cached;
use g3_route_transitions::{
    ROUTE_TRANSITION_OVERLAY_REGION_CLASS, animated_back_or_navigate, animated_navigate,
};
use g3_ui::{
    Avatar, AvatarSize, Badge, BottomSheet, Button, ButtonExpand, ButtonFill, ButtonSize, Card,
    CardVariant, Chip, Color, Content, EmptyState, Item, List, ListLines, SheetBackdrop, Shelf,
    Skeleton, SkeletonShape, Space, Spinner, Stack, StackAlign, Text, TextTone, TextVariant,
};

use super::{VideoGrid, linkify::Linkified};

/// The player's transport fetches from the webview, not through the server
/// functions, so it has to be told the backend the viewer chose in settings -
/// not just the compiled default, which on a device points back at itself.
fn playback_server_url() -> String {
    crate::server_url::backend_url()
}

// One argument per field of the player-metadata message it posts to the page.
#[allow(clippy::too_many_arguments)]
fn sync_player_metadata(
    video_id: String,
    tracks: Vec<CaptionTrack>,
    selected_caption: Option<usize>,
    captions_enabled: bool,
    chapters: Vec<VideoChapter>,
    preview_frames: Option<VideoPreviewFrames>,
    sponsor_segments: Vec<SponsorTimelineSegment>,
    sponsor_enabled: bool,
    sponsor_notify: bool,
    autoplay: bool,
) {
    let Ok(video_id) = serde_json::to_string(&video_id) else {
        return;
    };
    let Ok(tracks) = serde_json::to_string(&tracks) else {
        return;
    };
    let Ok(chapters) = serde_json::to_string(&chapters) else {
        return;
    };
    let Ok(preview_frames) = serde_json::to_string(&preview_frames) else {
        return;
    };
    let Ok(sponsor_segments) = serde_json::to_string(&sponsor_segments) else {
        return;
    };
    let Ok(server_url) = serde_json::to_string(&playback_server_url()) else {
        return;
    };
    let selected_caption = selected_caption
        .map(|index| index.to_string())
        .unwrap_or_else(|| "null".to_string());
    spawn(async move {
        let script = format!(
            r#"
            const rawTracks = {tracks};
            const chapters = {chapters};
            const previewFrames = {preview_frames};
            const sponsorSegments = {sponsor_segments};
            const sponsorEnabled = {sponsor_enabled};
            const sponsorNotify = {sponsor_notify};
            const autoplay = {autoplay};
            const serverUrl = {server_url};
            let media = document.getElementById('tawny-player-media');
            for (let attempt = 0; attempt < 800 && (!media || !window.TawnyPlayerControls || !window.TawnyTransport); attempt++) {{
                await new Promise((resolve) => setTimeout(resolve, 25));
                media = document.getElementById('tawny-player-media');
            }}
            if (media && window.TawnyPlayerControls && window.TawnyTransport) {{
                const options = {{ serverUrl }};
                const tracks = rawTracks.map((track) => ({{
                    ...track,
                    url: window.TawnyTransport.normalizePlaybackUrl(track.url, options),
                }}));
                const elements = media.querySelectorAll('track[data-tawny-caption]');
                tracks.forEach((track, index) => {{
                    const element = elements[index];
                    if (element && element.getAttribute('src') !== track.url) {{
                        element.setAttribute('src', track.url);
                    }}
                }});
                window.TawnyPlayerControls.setMetadata(media, {{
                    videoId: {video_id},
                    captions: tracks,
                    selectedCaption: {selected_caption},
                    captionsEnabled: {captions_enabled},
                    chapters,
                    previewFrames,
                    sponsorSegments,
                    sponsorEnabled,
                    sponsorNotify,
                    autoplay,
                }});
            }}
            dioxus.send(true);
            "#
        );
        let mut eval = document::eval(&script);
        let _ = eval.recv::<bool>().await;
    });
}

#[cfg(target_os = "android")]
#[component]
fn NativePlayerBridges(title: String) -> Element {
    let mut plugins = use_context::<g3_native_plugins::NativePlugins>();
    rsx! {
        button {
            r#type: "button", class: "player-caption-state-bridge", tabindex: "-1", aria_hidden: "true",
            "data-player-native-pip-landscape": "",
            onclick: move |_| { let _ = plugins.media.write().enter_picture_in_picture(16, 9); },
        }
        button {
            r#type: "button", class: "player-caption-state-bridge", tabindex: "-1", aria_hidden: "true",
            "data-player-native-pip-portrait": "",
            onclick: move |_| { let _ = plugins.media.write().enter_picture_in_picture(9, 16); },
        }
        button {
            r#type: "button", class: "player-caption-state-bridge", tabindex: "-1", aria_hidden: "true",
            "data-player-native-landscape": "",
            onclick: move |_| { let _ = plugins.media.write().set_orientation("landscape"); },
        }
        button {
            r#type: "button", class: "player-caption-state-bridge", tabindex: "-1", aria_hidden: "true",
            "data-player-native-orientation-unlock": "",
            onclick: move |_| { let _ = plugins.media.write().set_orientation("unspecified"); },
        }
        // The WebView's fullscreen stops at its own bounds, so the status bar
        // stays over the video unless the window hides it.
        button {
            r#type: "button", class: "player-caption-state-bridge", tabindex: "-1", aria_hidden: "true",
            "data-player-native-system-bars-hide": "",
            onclick: move |_| { let _ = plugins.media.write().set_system_bars_hidden(true); },
        }
        button {
            r#type: "button", class: "player-caption-state-bridge", tabindex: "-1", aria_hidden: "true",
            "data-player-native-system-bars-show": "",
            onclick: move |_| { let _ = plugins.media.write().set_system_bars_hidden(false); },
        }
        button {
            r#type: "button", class: "player-caption-state-bridge", tabindex: "-1", aria_hidden: "true",
            "data-player-native-playback-start": "",
            onclick: move |_| { let _ = plugins.media.write().set_playback_active(true, title.clone()); },
        }
        button {
            r#type: "button", class: "player-caption-state-bridge", tabindex: "-1", aria_hidden: "true",
            "data-player-native-playback-stop": "",
            onclick: move |_| { let _ = plugins.media.write().set_playback_active(false, String::new()); },
        }
    }
}

/// iOS keeps only the playback bridges: its picture-in-picture goes through
/// WebKit's own presentation mode, and nothing asks it to lock orientation.
/// Starting playback claims the `.playback` audio session that keeps audio
/// running once the app leaves the foreground.
#[cfg(target_os = "ios")]
#[component]
fn NativePlayerBridges(title: String) -> Element {
    let mut plugins = use_context::<g3_native_plugins::NativePlugins>();
    rsx! {
        button {
            r#type: "button", class: "player-caption-state-bridge", tabindex: "-1", aria_hidden: "true",
            "data-player-native-playback-start": "",
            onclick: move |_| { let _ = plugins.media.write().set_playback_active(true, title.clone()); },
        }
        button {
            r#type: "button", class: "player-caption-state-bridge", tabindex: "-1", aria_hidden: "true",
            "data-player-native-playback-stop": "",
            onclick: move |_| { let _ = plugins.media.write().set_playback_active(false, String::new()); },
        }
    }
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
#[component]
fn NativePlayerBridges(title: String) -> Element {
    let _ = title;
    rsx! {}
}

/// Pull the transport's own account of why playback stopped.
///
/// `TawnyTransport.events` outlives the media element, so this stays readable
/// after the failed player has been torn down.
fn read_transport_failure(mut detail: Signal<String>) {
    spawn(async move {
        let mut eval = document::eval(
            r#"
            const events = (window.TawnyTransport && window.TawnyTransport.events) || [];
            const notable = events.filter((event) =>
                event.phase === 'shaka-error'
                || event.phase === 'source-failed'
                || event.phase === 'recovering'
                || event.phase === 'retrying-stream');
            const last = notable[notable.length - 1] || window.__tawnyTransportDebug || null;
            const describe = (value) =>
                typeof value === 'string' ? value : JSON.stringify(value);
            const parts = [];
            if (last) {
                const source = last.protocol ? ` · ${last.protocol}` : '';
                const cause = last.error ? ` · ${describe(last.error)}` : '';
                const at = Number.isFinite(last.position)
                    ? ` · at ${Math.floor(last.position)}s`
                    : '';
                // A bare Shaka code says almost nothing. The status and the URI
                // it failed on are what separate a gated stream from a host the
                // device cannot reach, and there is no console on a handset.
                const code = last.code ? ` · code ${last.code}` : '';
                const status = last.httpStatus ? ` · HTTP ${last.httpStatus}` : '';
                const uri = last.uri ? ` · ${last.uri}` : '';
                parts.push(`${last.phase}${source}${code}${status}${at}${cause}${uri}`);
            }
            const media = document.getElementById('tawny-player-media');
            if (media && media.error) {
                parts.push(`media error ${media.error.code}: ${media.error.message || 'no detail'}`);
            }
            dioxus.send(parts.join(' — '));
            "#,
        );
        if let Ok(text) = eval.recv::<String>().await
            && !text.is_empty()
        {
            detail.set(text);
        }
    });
}

fn set_player_speed(speed: f64) {
    spawn(async move {
        let script = format!(
            r#"
            const media = document.querySelector('#tawny-player video');
            if (media) media.playbackRate = {speed};
            const frame = document.getElementById('tawny-player-frame');
            if (frame && frame.contentWindow) {{
                frame.contentWindow.postMessage(JSON.stringify({{
                    event: 'command', func: 'setPlaybackRate', args: [{speed}]
                }}), '*');
            }}
            dioxus.send(true);
            "#
        );
        let mut eval = document::eval(&script);
        let _ = eval.recv::<bool>().await;
    });
}

/// Switch the live audio language.
///
/// The transport owns the decision - Shaka rebuilds its variant list around the
/// chosen language - so this is a one-shot command rather than state Rust holds
/// and replays. The reporter below sends the result back.
fn set_player_audio_track(language: String) {
    spawn(async move {
        let script = format!(
            r#"
            const media = document.getElementById('tawny-player-media');
            if (media && window.TawnyTransport?.setAudioTrack) {{
                window.TawnyTransport.setAudioTrack(media, {language:?});
            }}
            dioxus.send(true);
            "#
        );
        let mut eval = document::eval(&script);
        let _ = eval.recv::<bool>().await;
    });
}

/// Resolves once the route transition has finished and the page is idle.
///
/// Anything the watch route renders costs real main-thread time, and spending
/// it while a transition is running is what starves that transition: the page
/// slide runs on the compositor and keeps its frames, while the player's morph
/// (which animates width and height, and so cannot) drops most of its own and
/// arrives late, catching up in one jump. Chromium schedules this differently
/// between builds, which is why the same page can look fine in one browser and
/// snap in another. Rendering a beat later costs a late-arriving row of
/// thumbnails and buys a transition that is smooth everywhere.
///
/// The related grid was the first thing held back here, but it was never the
/// only one: the two closed sheets build their whole lists regardless, and the
/// remote details land in the middle of the animation and re-render the body
/// under it. See `VideoDetailInner` for what each of them waits on.
const AFTER_ROUTE_TRANSITION_JS: &str = r#"
const frame = () => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
// Frames do not tick inside a view transition's update callback, but they do
// once its animations are running, so this settles exactly when they end.
let guard = 0;
while (document.documentElement.dataset.routeTransition && guard < 120) {
    guard += 1;
    await frame();
}
await frame();
dioxus.send(true);
"#;

/// Reports the transport's audio languages back to Rust.
///
/// Dubbed uploads carry a dozen or more, and which ones survive codec filtering
/// is a decision only the transport has made by this point - so the list is read
/// from it rather than from the extractor's track layout.
const AUDIO_TRACK_REPORTER_JS: &str = r#"
const key = Symbol.for('tawny.audio-track-reporter');
window[key]?.dispose?.();

const media = () => document.getElementById('tawny-player-media');
let last = '';
const send = () => {
    const element = media();
    if (!element || !window.TawnyTransport?.audioTrackState) return;
    const state = window.TawnyTransport.audioTrackState(element);
    // A single language is not a choice, so it is reported as none at all and
    // the chip stays away.
    const tracks = (state?.tracks || []).length > 1 ? state.tracks : [];
    const payload = [
        tracks.map((track) => [track.language, track.label]),
        tracks.length ? (state.active || '') : '',
    ];
    const encoded = JSON.stringify(payload);
    if (encoded === last) return;
    last = encoded;
    dioxus.send(payload);
};

// The list only exists once the manifest is parsed, and it changes again on a
// quality or language switch.
const timer = setInterval(send, 1000);
const onChange = () => send();
const element = media();
element?.addEventListener('tawnyaudiotrackchange', onChange);
element?.addEventListener('tawnytransportchange', onChange);
element?.addEventListener('loadedmetadata', onChange);
window[key] = {
    dispose() {
        clearInterval(timer);
        element?.removeEventListener('tawnyaudiotrackchange', onChange);
        element?.removeEventListener('tawnytransportchange', onChange);
        element?.removeEventListener('loadedmetadata', onChange);
    },
};
"#;

fn persist_player_speed(mut app_state: AppState, is_short: bool) {
    spawn(async move {
        let mut eval = document::eval(
            r#"
            const media = document.getElementById('tawny-player-media');
            dioxus.send(media ? media.playbackRate : 1);
            "#,
        );
        if let Ok(speed) = eval.recv::<f64>().await
            && speed.is_finite()
            && speed > 0.0
        {
            app_state.settings.write().set_speed_for(is_short, speed);
        }
    });
}

// One argument per field of the attach call it makes on the page's player.
#[allow(clippy::too_many_arguments)]
fn attach_player_session(
    session: PlaybackSession,
    playback_rate: f64,
    video_id: &str,
    player_key: &str,
    prefer_sabr: bool,
    audio_only: bool,
    preferred_audio_language: &str,
    resume_seconds: u64,
) {
    let Ok(session) = serde_json::to_string(&session) else {
        return;
    };
    let Ok(server_url) = serde_json::to_string(&playback_server_url()) else {
        return;
    };
    let video_id = video_id.to_string();
    let player_key = player_key.to_string();
    let preferred_audio_language = preferred_audio_language.to_string();
    spawn(async move {
        let script = format!(
            r#"
            const session = {session};
            const videoId = {video_id:?};
            const playerKey = {player_key:?};
            const preferredAudioLanguage = {preferred_audio_language:?};
            const serverUrl = {server_url};
            const media = document.getElementById('tawny-player-media');
            window.__tawnyAttachEval = {{ phase: 'starting', hasMedia: Boolean(media) }};
            // This work is launched from an onmounted callback, so a fast
            // watch-to-watch navigation can replace the element before this
            // script runs.  Never let an old resolver attach its stream to the
            // newly mounted element simply because they share an id.
            if (!media || media.dataset.playerKey !== playerKey) {{ dioxus.send(false); return; }}
            for (let attempt = 0; attempt < 100 && !window.TawnyTransport; attempt++) {{
                await new Promise((resolve) => setTimeout(resolve, 25));
            }}
            if (!window.TawnyTransport) {{
                window.__tawnyAttachEval = {{ phase: 'transport-missing', hasMedia: true }};
                media.dispatchEvent(new Event('error'));
                dioxus.send(false);
                return;
            }}
            try {{
                // A removed media node can continue playing in Chromium. Keep
                // its element around long enough to detach its transport and
                // pause it before the replacement starts, otherwise tapping a
                // new player can leave both videos audible.
                const previous = window.__tawnyActivePlayerMedia;
                if (previous && previous !== media) {{
                    window.TawnyPlayerControls?.detach?.(previous);
                    await Promise.resolve(window.TawnyTransport.detach?.(previous)).catch(() => {{}});
                    previous.pause();
                    previous.removeAttribute('src');
                    previous.load();
                }}
                window.__tawnyActivePlayerMedia = media;
                if (window.TawnyPlayerControls) {{
                    window.TawnyPlayerControls.attach(media);
                }}
                const serverBase = window.TawnyTransport.playbackServerBase({{ serverUrl }});
                const refreshPath = `/api/v1/playback/${{encodeURIComponent(videoId)}}?prefer_sabr={prefer_sabr}&fresh=true`;
                await window.TawnyTransport.attach(media, session, {{
                    playbackRate: {playback_rate},
                    videoId,
                    serverUrl,
                    audioOnly: {audio_only},
                    preferredAudioLanguage,
                    startTime: {resume_seconds},
                    refreshUrl: new URL(refreshPath, `${{serverBase}}/`).href,
                }});
                if (window.__tawnyActivePlayerMedia !== media || media.dataset.playerKey !== playerKey) {{
                    await Promise.resolve(window.TawnyTransport.detach?.(media)).catch(() => {{}});
                    media.pause();
                    dioxus.send(false);
                    return;
                }}
                window.__tawnyAttachEval = {{ phase: 'attached', hasMedia: true }};
                dioxus.send(true);
            }} catch (error) {{
                window.__tawnyAttachEval = {{ phase: 'failed', hasMedia: true, error: String(error) }};
                console.warn('Tawny transport exhausted its sources', error);
                media.dispatchEvent(new Event('error'));
                dioxus.send(false);
            }}
            "#
        );
        let mut eval = document::eval(&script);
        let _ = eval.recv::<bool>().await;
    });
}

/// Reports the media element's position back to Rust on a timer.
///
/// Each run replaces the previous registration outright: leaving the old timer
/// and listeners in place leaked one set per video change, and a stale one would
/// keep writing its own position over the video that replaced it.
/// How far the server's saved position may differ from the one the player
/// opened at before the player moves to it. Within this, the two are the
/// same place, give or take the last few seconds of a sample.
const RESUME_DRIFT_SECONDS: u64 = 5;

/// Waits before each automatic retry of a playback that failed for a reason
/// that tends to pass. Spread out because a busy extractor or a rate limit
/// needs time, not another immediate request.
const AUTO_RETRY_DELAYS_MS: [u32; 3] = [1_500, 4_000, 9_000];

/// Moves the playing video to the position the server has saved, once it is
/// attached, unless the viewer has already moved it.
///
/// `__FROM__` is where the player opened, `__TO__` the saved position. Gives
/// up after twenty seconds: a video that has not attached by then has bigger
/// problems, and a late jump would be a surprise.
const RESUME_FROM_SERVER_JS: &str = r#"
const from = __FROM__;
const to = __TO__;
const started = Date.now();
const tryResume = () => {
    const media = document.getElementById('tawny-player-media');
    const attached = window.__tawnyAttachEval?.phase === 'attached';
    if (media && attached && media.readyState >= 1) {
        if (Math.abs(media.currentTime - from) <= 5) media.currentTime = to;
        return;
    }
    if (Date.now() - started < 20000) setTimeout(tryResume, 200);
};
tryResume();
"#;

const PROGRESS_REPORTER_JS: &str = r#"
const key = Symbol.for('tawny.progress-reporter');
window[key]?.dispose?.();

const media = () => document.getElementById('tawny-player-media');
const send = (flush) => {
    const element = media();
    if (element && Number.isFinite(element.currentTime)) {
        dioxus.send([Math.max(0, Math.floor(element.currentTime)), Boolean(flush)]);
    }
};

const timer = setInterval(() => send(false), 5000);
// The moments where losing the position would actually be felt.
const onPause = () => send(true);
const onHide = () => send(true);
// pagehide is not reliable when a mobile browser is merely backgrounded;
// visibility is, and it is the edge where a position is most often lost.
const onVisibility = () => { if (document.visibilityState !== 'visible') send(true); };
document.addEventListener('pause', onPause, true);
document.addEventListener('visibilitychange', onVisibility);
window.addEventListener('pagehide', onHide);
window.addEventListener('beforeunload', onHide);
window[key] = {
    dispose() {
        clearInterval(timer);
        document.removeEventListener('pause', onPause, true);
        document.removeEventListener('visibilitychange', onVisibility);
        window.removeEventListener('pagehide', onHide);
        window.removeEventListener('beforeunload', onHide);
    },
};
"#;

#[component]
pub fn PersistentPlayer(expanded: bool) -> Element {
    let app_state = use_context::<AppState>();
    // Remembered per viewer rather than per video: chosen once, every desktop
    // video opens on the wider stage. The setting is the state rather than
    // something copied into a signal at mount - the stored settings are read
    // back a moment after the first render, so a copy would be the default
    // and a reload would forget the choice.
    let mut settings_for_theater = app_state.settings;
    let theater_mode = app_state.settings.read().theater_mode;
    let mut playback_attempt = use_signal(|| 0_u8);
    // The privacy-enhanced iframe is a manual escape hatch, never an automatic
    // one. Silently swapping to it hides extraction regressions behind a player
    // that ignores the app's controls, captions, and chapters.
    let mut use_embed = use_signal(|| false);
    let mut playback_failed = use_signal(|| false);
    let mut failure_detail = use_signal(String::new);
    let mut observed_video_id = use_signal(String::new);
    // The video whose session stopped working. The server hands out the session
    // it resolved earlier, so a retry for this video has to ask for a new one
    // rather than get the same broken answer back. Keyed by video so the flag
    // cannot leak into the next one and cost it the warm session.
    let mut stale_session_for = use_signal(|| None::<String>);
    // How many times the player has asked again on its own for this video, and a
    // counter that re-runs the resolver. The counter is separate from
    // `playback_attempt` because that one is also the budget for transport
    // errors and part of the element's key.
    let mut auto_retries = use_signal(|| 0_u8);
    let mut resolve_nonce = use_signal(|| 0_u32);
    let prefer_sabr = app_state.settings.read().prefer_sabr;
    let preferred_audio_language = app_state.settings.read().preferred_audio_language.clone();

    // The resolved session is tagged with the video it belongs to. `use_resource`
    // keeps serving its previous value while it re-runs, so without the tag a
    // freshly opened video reads the *previous* video's session and renders a
    // conclusion — including a failure — that was never about this video.
    let playback_resource = use_resource(move || {
        let active_video = app_state.active_video();
        let attempt = playback_attempt();
        let nonce = resolve_nonce();
        async move {
            let _attempt = (attempt, nonce);
            match active_video {
                Some(video) => {
                    let id = video.id.clone();
                    let fresh = stale_session_for.peek().as_deref() == Some(id.as_str());
                    Some((id, resolve_playback(video.id, prefer_sabr, fresh).await))
                }
                None => None,
            }
        }
    });

    // Resolve whatever is up next while this one plays, so moving on - by
    // autoplay, a swipe, or the next button - finds its session already on the
    // server instead of waiting several seconds for it. Held back until this
    // video's own session is in, so the two never race for the extractor.
    let mut warmed_next = use_signal(|| None::<String>);
    // The video itself, fetched with its session. Moving on can then hand it
    // to the player the moment this one ends, and the stage goes straight to
    // loading the next one instead of sitting paused with its controls up
    // until the next watch page has fetched its details.
    let mut upcoming_video = use_signal(|| None::<Video>);
    use_effect(move || {
        let resolved_video_id = playback_resource
            .read()
            .as_ref()
            .and_then(|value| value.as_ref())
            .map(|(id, _)| id.clone());
        let Some(current) = resolved_video_id else {
            return;
        };
        if app_state
            .active_video()
            .is_none_or(|active| active.id != current)
        {
            return;
        }
        let Some(next) = app_state.next_in_run(&current) else {
            return;
        };
        if warmed_next.peek().as_deref() == Some(next.as_str()) {
            return;
        }
        warmed_next.set(Some(next.clone()));
        warm_playback(next.clone(), prefer_sabr);
        spawn(async move {
            if let Ok(videos) = get_videos(vec![next.clone()]).await
                && let Some(video) = videos.into_iter().find(|video| video.id == next)
            {
                upcoming_video.set(Some(video));
            }
        });
    });
    // Moves on to `next`: shown in the player at once when it is the video
    // fetched above, then the watch page follows.
    //
    // Minimized, the viewer chose to keep browsing, so the next video takes
    // over the mini bar where it is. Navigating to it would throw the whole
    // player open over whatever they were reading.
    let advance_to = move |next: String| async move {
        let upcoming = upcoming_video.peek().clone();
        let known = upcoming.filter(|video| video.id == next);
        if expanded {
            if let Some(video) = known {
                app_state.play(video);
            }
            animated_navigate(Route::VideoDetail { id: next }).await;
            return;
        }
        let video = match known {
            Some(video) => Some(video),
            None => get_videos(vec![next.clone()])
                .await
                .ok()
                .and_then(|videos| videos.into_iter().find(|video| video.id == next)),
        };
        if let Some(video) = video {
            app_state.record_history(&video.id);
            app_state.play(video);
        }
    };

    let active_video = app_state.active_video();
    let video_id = active_video
        .as_ref()
        .map(|video| video.id.clone())
        .unwrap_or_default();
    use_effect(move || {
        if observed_video_id() != video_id {
            observed_video_id.set(video_id.clone());
            playback_attempt.set(0);
            stale_session_for.set(None);
            auto_retries.set(0);
            use_embed.set(false);
            playback_failed.set(false);
            failure_detail.set(String::new());
        }
    });
    // Persist where playback has reached. Polled rather than driven by
    // `timeupdate`, which fires several times a second and would drag a
    // re-render along with it.
    //
    // Declared before the early return below: a hook that only runs on renders
    // where a video happens to be active is a conditional hook, and it never
    // registers. Tracking the signal here also means the effect re-runs when the
    // video changes, which is what replaces the previous reporter.
    //
    // Keyed on the playing video's id, through a memo. Reading `active_video`
    // itself re-ran this every five seconds, because recording progress writes
    // it: each run replaced the reporter and reset `ticks`, so the periodic
    // save to the server never came round, and progress only reached it on
    // pause, backgrounding or a change of video.
    let playing_id = use_memo(move || {
        app_state
            .active_video
            .read()
            .as_ref()
            .map(|video| video.id.clone())
    });
    // A failure that is likely to pass - a busy extractor, a rate limit, a
    // stream cut off after the app sat idle - is asked again by the player
    // itself, a few times and a little further apart each time, while the
    // stage keeps showing it loading. Only when that runs out does the failure
    // panel appear, with the manual retry it has always had. The value is how
    // many automatic retries have been made, and ends at `None` once spent.
    let auto_retry_attempt = use_memo(move || {
        let attempts = usize::from(auto_retries());
        if attempts >= AUTO_RETRY_DELAYS_MS.len() {
            return None;
        }
        let active = playing_id()?;
        let resolve_failed_for_now = playback_resource
            .read()
            .as_ref()
            .and_then(|value| value.as_ref())
            .is_some_and(|(id, result)| {
                *id == active
                    && result
                        .as_ref()
                        .err()
                        .is_some_and(|failure| failure.retryable)
            });
        (resolve_failed_for_now || playback_failed()).then_some(attempts)
    });
    use_effect(move || {
        let Some(attempt) = auto_retry_attempt() else {
            return;
        };
        spawn(async move {
            let mut eval = document::eval(&format!(
                "await new Promise((resolve) => setTimeout(resolve, {}));
                dioxus.send(true);",
                AUTO_RETRY_DELAYS_MS[attempt]
            ));
            let _ = eval.recv::<bool>().await;
            // The viewer may have retried by hand, or moved on, while waiting.
            if auto_retry_attempt.peek().as_ref() != Some(&attempt) {
                return;
            }
            stale_session_for.set(playing_id.peek().clone());
            auto_retries.set(attempt as u8 + 1);
            playback_failed.set(false);
            playback_attempt.set(0);
            resolve_nonce += 1;
        });
    });
    let progress_state = app_state;
    // The reporter's JS disposes the previous video's timers itself, but its
    // eval channel stays open, so without cancelling here each video left a
    // receive loop suspended for the life of the page.
    let progress_task =
        use_hook(|| std::rc::Rc::new(std::cell::Cell::new(None::<dioxus::core::Task>)));
    use_effect(move || {
        if let Some(previous) = progress_task.take() {
            previous.cancel();
        }
        let Some(playing_id) = playing_id() else {
            return;
        };
        progress_task.set(Some(spawn(async move {
            let mut eval = document::eval(PROGRESS_REPORTER_JS);
            // Every position that moved is sent - about every five seconds
            // while playing - so another device picks up close to where this
            // one is; a position that did not move has nothing to save. The
            // flag marks the page going away or playback pausing, and is when
            // the cached lists are brought up to date as well.
            while let Ok((seconds, going_away)) = eval.recv::<(u64, bool)>().await {
                if progress_state.record_progress(&playing_id, seconds) {
                    progress_state.sync_progress(going_away);
                } else if going_away {
                    progress_state.flush_progress();
                }
            }
        })));
    });

    // Resume from the server's copy of the position, not only the one the
    // video was opened with. That came from a list, which on a second device
    // may have been fetched before the viewer stopped on the first: opening
    // on the desktop the video just left on the phone started it wherever the
    // desktop's copy of the list last saw it. Keyed on the id, like the
    // reporter, so it asks once per video.
    let resume_state = app_state;
    use_effect(move || {
        let Some(video_id) = playing_id() else {
            return;
        };
        let opened = resume_state
            .active_video
            .peek()
            .as_ref()
            .map(|video| (video.resume_seconds(), video.clone()));
        let Some((opened_at, opened_video)) = opened else {
            return;
        };
        spawn(async move {
            let Ok(Some(saved)) = get_video_progress(video_id.clone()).await else {
                return;
            };
            // Finished elsewhere: opening it again means starting over, which
            // is what the player already did.
            if saved.watched
                || opened_video.is_effectively_over(saved.progress_seconds)
                || saved.progress_seconds.abs_diff(opened_at) <= RESUME_DRIFT_SECONDS
            {
                return;
            }
            if playing_id.peek().as_deref() != Some(video_id.as_str()) {
                return;
            }
            document::eval(
                &RESUME_FROM_SERVER_JS
                    .replace("__FROM__", &opened_at.to_string())
                    .replace("__TO__", &saved.progress_seconds.to_string()),
            );
        });
    });

    // Same shape as the progress reporter above, and declared before the same
    // early return for the same reason: a hook that only runs when a video
    // happens to be active never registers at all.
    // Keyed on the id for the same reason: this re-registered the reporter
    // every five seconds.
    let mut audio_state = app_state;
    let audio_task =
        use_hook(|| std::rc::Rc::new(std::cell::Cell::new(None::<dioxus::core::Task>)));
    use_effect(move || {
        if let Some(previous) = audio_task.take() {
            previous.cancel();
        }
        if playing_id.read().is_none() {
            return;
        }
        audio_task.set(Some(spawn(async move {
            let mut eval = document::eval(AUDIO_TRACK_REPORTER_JS);
            while let Ok((tracks, active)) = eval.recv::<(Vec<(String, String)>, String)>().await {
                audio_state.active_audio_tracks.set(
                    tracks
                        .into_iter()
                        .map(|(language, label)| AudioTrackOption { language, label })
                        .collect(),
                );
                audio_state
                    .selected_audio_track
                    .set((!active.is_empty()).then_some(active));
            }
        })));
    });

    // The transport gets the saved preference at attachment time. This effect
    // additionally applies a preference changed in Settings to the video that
    // is already playing, without waiting for the next one to start.
    let audio_preference_state = app_state;
    use_effect(move || {
        let preferred_language = audio_preference_state
            .settings
            .read()
            .preferred_audio_language
            .clone();
        // Through the id memo: `active_video` changes every five seconds with
        // progress, and re-applied the track each time.
        if playing_id.read().is_some() {
            set_player_audio_track(preferred_language);
        }
    });

    // Metadata belongs to the media element, so a replacement element starts
    // with none: no chapters, no segments, a bare timeline. Both things that
    // build a new one - a retried stream, and the audio-only switch, which
    // remounts the element on purpose - are read here so the metadata follows
    // it over. The video is read through memos that change only with its id,
    // its audio-only choice or its kind, not with every progress update.
    // Ahead of the early return below, like every hook.
    let synced_video = use_memo(move || {
        app_state
            .active_video()
            .map(|video| (video.id, video.audio_only, video.is_short))
    });
    use_effect(move || {
        let Some((video_id, _audio_only, is_short)) = synced_video() else {
            return;
        };
        let _ = playback_attempt();
        let settings = app_state.settings.read();
        let autoplay = settings.autoplay_for(is_short);
        let sponsor_settings = &settings.sponsor_block;
        let sponsor_enabled = !sponsor_settings.requested_categories().is_empty();
        sync_player_metadata(
            video_id,
            (app_state.active_captions)(),
            (app_state.selected_caption)(),
            (app_state.captions_enabled)(),
            (app_state.active_chapters)(),
            (app_state.active_preview_frames)(),
            timeline_segments(&(app_state.active_sponsor_segments)(), sponsor_settings),
            sponsor_enabled,
            sponsor_settings.notify_on_skip,
            autoplay,
        )
    });

    let Some(video) = active_video else {
        return rsx! {};
    };

    let resolved_for_this_video = playback_resource
        .read()
        .as_ref()
        .and_then(|value| value.as_ref())
        .is_some_and(|(id, _)| *id == video.id);
    let playback_session = playback_resource
        .read()
        .as_ref()
        .and_then(|value| value.as_ref())
        .filter(|(id, _)| *id == video.id)
        .and_then(|(_, result)| result.as_ref().ok())
        .cloned();
    // Why the server found nothing to play, in its own words - a stopped
    // yt-dlp sidecar, for one - rather than a generic "check the log".
    let resolve_error = playback_resource
        .read()
        .as_ref()
        .and_then(|value| value.as_ref())
        .filter(|(id, _)| *id == video.id)
        .and_then(|(_, result)| result.as_ref().err())
        .cloned();
    let fallback_url = playback_session
        .as_ref()
        .map(|session| session.fallback_url.clone())
        .unwrap_or_else(|| {
            format!(
                "https://www.youtube-nocookie.com/embed/{}?enablejsapi=1&autoplay=1&playsinline=1",
                video.id
            )
        });
    let playback_source = playback_session.as_ref().map(|session| {
        if source_is_transportable(&session.primary) {
            session.primary.clone()
        } else {
            session
                .alternatives
                .iter()
                .find(|source| source_is_transportable(source))
                .cloned()
                .unwrap_or_else(|| session.primary.clone())
        }
    });
    // Only a session actually resolved for *this* video can justify the failure
    // panel; anything else means the resolve is still in flight.
    let resolving = !resolved_for_this_video || auto_retry_attempt().is_some();
    let direct_stream = !use_embed()
        && !playback_failed()
        && playback_source
            .as_ref()
            .is_some_and(source_is_transportable);
    // Two very different failures land on the same panel, and they need
    // different fixes: the server extracting nothing at all, versus the server
    // handing over streams that the browser then failed to play.
    let nothing_extracted = resolved_for_this_video
        && !playback_source
            .as_ref()
            .is_some_and(source_is_transportable);
    let is_short = video.is_short;
    let errored_video_id = video.id.clone();
    let retried_video_id = video.id.clone();
    let current_speed = app_state.settings.read().speed_for(is_short);
    let session_to_attach = playback_session.clone();
    let attach_video_id = video.id.clone();
    // Per-video preference, read here so a change re-attaches the transport with
    // (or without) the video stream rather than only hiding the picture.
    let attach_audio_only = video.audio_only;
    // Resume where this video was left, unless it already ran to the end - then
    // starting over is what replaying it means.
    let attach_resume_seconds = video.resume_seconds();
    let audio_only_active = video.audio_only;
    let audio_only_video_id = video.id.clone();
    let mut captions_enabled = app_state.captions_enabled;
    let captions_for_toggle = app_state.active_captions;
    let caption_tracks_to_render = (app_state.active_captions)();
    let mut chapters_sheet = app_state.chapters_sheet_open;
    let open_video_id = video.id.clone();
    // The audio-only choice is baked into the manifest the transport builds, so
    // it only takes effect on attach. Keying the element on it forces a remount
    // and re-attach; without it the toggle changed state while the already-
    // attached video stream kept playing.
    let player_key = format!(
        "{}-{}-{}",
        video.id,
        playback_attempt(),
        if video.audio_only { "audio" } else { "av" }
    );
    let auto_landscape = app_state.settings.read().auto_landscape_fullscreen;
    let autoplay_enabled = app_state.settings.read().autoplay_for(is_short);
    let autoplay_video_id = video.id.clone();
    let mut autoplay_settings = app_state.settings;
    // A run is anything with somewhere to go: a queue ahead, a playlist being
    // run, a video stepped back from, or any combination. Each direction is
    // independently rendered below, so an unavailable side is absent rather than
    // a permanent disabled button.
    //
    // Asked of the run rather than of the queue's length: a queue holding only a
    // spent playlist marker has nothing left to play, and offering a Next that
    // does nothing is worse than not offering one.
    let has_next_queued = app_state.has_next_in_run(&video.id);
    let can_step_back = app_state.can_step_back_in_run(&video.id);
    // Naming the playlist is the only thing that tells a viewer these arrows
    // walk the list they pressed play on rather than a queue they hand-built.
    let run_name = app_state
        .running_playlist_name()
        .unwrap_or_else(|| "queue".to_string());
    let next_label = format!("Next in {run_name}");
    let previous_label = format!("Previous in {run_name}");
    let step_forward_id = video.id.clone();
    let step_back_id = video.id.clone();
    let playback_title = video.title.clone();

    rsx! {
        div { class: if expanded {
                if theater_mode { "persistent-player expanded theater-mode" } else { "persistent-player expanded" }
            } else { "persistent-player mini" },
            section {
                id: "tawny-player",
                // Dropping the video AdaptationSets covers the adaptive path, but a
                // progressive or HLS source has no separable video track to leave
                // out. Covering the surface with the artwork makes "audio only"
                // mean the same thing whichever transport ends up being used.
                class: if video.audio_only { "persistent-player-stage audio-only" } else { "persistent-player-stage" },
                "data-video-id": "{video.id}",
                "data-thumbnail": "{video.thumbnail_url}",
                // Read by the Android media plugin for the system player.
                "data-title": "{video.title}",
                "data-channel": "{video.channel_name}",
                // Feeds the audio-only artwork above; a CSS rule cannot reach the
                // thumbnail on its own.
                style: "--player-artwork: url('{video.thumbnail_url}');",
                "data-auto-landscape": auto_landscape.to_string(),
                if direct_stream {
                    video {
                        id: "tawny-player-media",
                        key: "{player_key}",
                        "data-player-key": "{player_key}",
                        autoplay: true,
                        playsinline: true,
                        crossorigin: "anonymous",
                        poster: "{video.thumbnail_url}",
                        onmounted: move |_| {
                            if let Some(session) = session_to_attach.clone() {
                                attach_player_session(
                                    session,
                                    current_speed,
                                    &attach_video_id,
                                    &player_key,
                                    prefer_sabr,
                                    attach_audio_only,
                                    &preferred_audio_language,
                                    attach_resume_seconds,
                                );
                            }
                        },
                        onloadedmetadata: move |_| set_player_speed(current_speed),
                        onratechange: move |_| persist_player_speed(app_state, is_short),
                        onerror: move |_| {
                            // Re-resolve the session a couple of times: playback
                            // URLs expire, and a fresh resolve is the fix. When
                            // that stops helping, say so instead of quietly
                            // handing the video to YouTube's own player.
                            if playback_attempt() < 2 {
                                stale_session_for.set(Some(errored_video_id.clone()));
                                playback_attempt += 1;
                            } else {
                                read_transport_failure(failure_detail);
                                playback_failed.set(true);
                            }
                        },
                        for caption in caption_tracks_to_render {
                            track {
                                kind: "subtitles",
                                src: "{caption.url}",
                                srclang: "{caption.language_code}",
                                label: "{caption.label}",
                                "data-tawny-caption": "",
                            }
                        }
                    }
                    div { class: "player-buffering-indicator", aria_hidden: "true",
                        div { class: "player-buffering-ring" }
                    }
                    div { class: "player-seek-feedback player-seek-feedback-left", aria_hidden: "true",
                        RotateCcw { size: 30 }
                        span { "10 seconds" }
                    }
                    div { class: "player-seek-feedback player-seek-feedback-right", aria_hidden: "true",
                        RotateCw { size: 30 }
                        span { "10 seconds" }
                    }
                    // A segment set to "show" is left for the viewer to decide
                    // about, which until now meant colour on the timeline and
                    // no way to act on it. The offer stands while the playhead
                    // is inside the segment, and lives outside the control bar:
                    // that bar takes no pointer events and fades on its own
                    // schedule, which would have made this the one control with
                    // a deadline that you cannot press.
                    button {
                        r#type: "button",
                        class: "player-sponsor-skip",
                        "data-player-sponsor-skip": "",
                        hidden: true,
                        span { "Skip" }
                        ChevronRight { size: 16 }
                    }
                    div {
                        class: "tawny-player-controls controls-visible",
                        role: "group",
                        aria_label: "Video controls",
                        div { class: "player-controls-scrim" }
                        div { class: "player-controls-top",
                            // Same affordance as every other page: this leaves
                            // the video, it does not shrink it in place.
                            button {
                                r#type: "button",
                                class: "player-control-button player-minimize-button",
                                aria_label: "Back",
                                title: "Back",
                                // Deliberately the same call the swipe-down
                                // makes: popping history instead picks its
                                // animation from whatever came before, which
                                // is not always the page under the sheet.
                                "data-player-action": "back",
                                ChevronLeft { size: 26 }
                            }
                            strong { class: "player-overlay-title", "{video.title}" }
                            div { class: "player-controls-top-actions",
                                // Autoplay is a per-sitting decision as often
                                // as a preference, so it is reachable without
                                // leaving the video. It writes the same setting
                                // the settings page does, split by kind: a run
                                // of Shorts and a long video are not the same
                                // choice.
                                button {
                                    r#type: "button",
                                    class: if autoplay_enabled {
                                        "player-control-button player-autoplay-button is-on"
                                    } else {
                                        "player-control-button player-autoplay-button"
                                    },
                                    aria_label: if is_short { "Autoplay Shorts" } else { "Autoplay videos" },
                                    title: if autoplay_enabled { "Autoplay is on" } else { "Autoplay is off" },
                                    aria_pressed: autoplay_enabled.to_string(),
                                    onclick: move |event: Event<MouseData>| {
                                        // Root owns media-surface taps. This is
                                        // an independent state control, never a
                                        // request to toggle playback too.
                                        event.stop_propagation();
                                        let enabled = autoplay_settings().autoplay_for(is_short);
                                        autoplay_settings.write().set_autoplay_for(is_short, !enabled);
                                    },
                                    // A switch rather than a glyph: this button
                                    // reports a state as much as it invites a
                                    // tap, and the list icon it used to carry
                                    // said "queue" - which is the thing beside
                                    // it in the header.
                                    if autoplay_enabled {
                                        ToggleRight { size: 23 }
                                    } else {
                                        ToggleLeft { size: 23 }
                                    }
                                }
                                // Clicked by the player JS when a video ends.
                                // Advancing is decided here because the queue
                                // and the setting both live in Rust.
                                button {
                                    r#type: "button",
                                    class: "player-caption-state-bridge",
                                    tabindex: "-1",
                                    aria_hidden: "true",
                                    "data-player-autoplay-next": "",
                                    // Read by the JS at the end of a video, so a
                                    // stage about to move on shows the next one
                                    // loading rather than its own paused controls.
                                    "data-has-next": has_next_queued.to_string(),
                                    onclick: move |_| {
                                        let finished = autoplay_video_id.clone();
                                        if let Some(next) = app_state.take_next_queued(&finished) {
                                            spawn(advance_to(next));
                                        }
                                    },
                                }
                                button {
                                    r#type: "button",
                                    class: "player-control-button",
                                    aria_label: "Picture in picture",
                                    title: "Picture in picture",
                                    "data-player-action": "pip",
                                    PictureInPicture { size: 21 }
                                }
                                button {
                                    r#type: "button",
                                    class: "player-control-button",
                                    aria_label: "Player settings",
                                    title: "Player settings",
                                    "data-player-action": "settings",
                                    Settings2 { size: 22 }
                                }
                            }
                        }
                        // Seeking has no buttons here on purpose - it is a
                        // double-tap on either side of the video, and the arrow
                        // keys on a keyboard - but moving through a queue is not
                        // seeking, and there is no gesture for it. The pair only
                        // appears when there is a run to move through.
                        div { class: "player-controls-center",
                            if can_step_back {
                                button {
                                    r#type: "button",
                                    class: "player-control-button player-step-button player-step-previous",
                                    aria_label: "{previous_label}",
                                    title: "{previous_label}",
                                    onclick: move |_| {
                                        let current = step_back_id.clone();
                                        spawn(async move {
                                            let Some(previous) = app_state.step_back_in_run(&current) else {
                                                return;
                                            };
                                            animated_navigate(Route::VideoDetail { id: previous }).await;
                                        });
                                    },
                                    ChevronLeft { size: 27 }
                                }
                            }
                            button {
                                r#type: "button",
                                class: "player-control-button player-play-button",
                                aria_label: "Play",
                                title: "Play or pause",
                                "data-player-action": "toggle",
                                span { class: "player-play-icon", Play { size: 38, fill: "currentColor" } }
                                span { class: "player-pause-icon", Pause { size: 38, fill: "currentColor" } }
                            }
                            if has_next_queued {
                                button {
                                    r#type: "button",
                                    class: "player-control-button player-step-button player-step-next",
                                    aria_label: "{next_label}",
                                    title: "{next_label}",
                                    onclick: move |_| {
                                        let current = step_forward_id.clone();
                                        if let Some(next) = app_state.take_next_queued(&current) {
                                            spawn(advance_to(next));
                                        }
                                    },
                                    ChevronRight { size: 27 }
                                }
                            }
                        }
                        div { class: "player-options-menu", "data-player-options-menu": "", hidden: true,
                            div { class: "player-options-section",
                                strong { "Playback speed" }
                                div { class: "player-option-grid",
                                    for speed in [0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0] {
                                        button {
                                            r#type: "button",
                                            class: "player-option-button",
                                            "data-player-speed": "{speed}",
                                            "{speed}×"
                                        }
                                    }
                                }
                            }
                            div { class: "player-options-section",
                                strong { "Audio only" }
                                div { class: "player-option-grid",
                                    button {
                                        r#type: "button",
                                        class: if audio_only_active { "player-option-button selected" } else { "player-option-button" },
                                        onclick: move |_| {
                                            let active = app_state.active_video.peek().clone();
                                            if let Some(video) = active.filter(|video| video.id == audio_only_video_id) {
                                                app_state.set_audio_only(&video, !audio_only_active);
                                            }
                                        },
                                        if audio_only_active { "On" } else { "Off" }
                                    }
                                }
                            }
                            div { class: "player-options-section",
                                strong { "Quality" }
                                div { class: "player-option-grid", "data-player-quality-options": "",
                                    button { r#type: "button", class: "player-option-button selected", "Auto" }
                                }
                            }
                        }
                        // Announces an automatic skip. Without it a video that
                        // jumps forward on its own reads as a fault.
                        div {
                            class: "player-sponsor-notice",
                            "data-player-sponsor-notice": "",
                            role: "status",
                            aria_live: "polite",
                        }
                        div { class: "player-controls-bottom",
                            div { class: "player-control-row",
                                span { class: "player-time",
                                    span { "data-player-current-time": "", "0:00" }
                                    span { class: "player-time-separator", " / " }
                                    span { "data-player-duration": "", "0:00" }
                                    // Current chapter, to the right of the clock; tapping it
                                    // opens the chapter list rather than making the user scrub.
                                button {
                                    r#type: "button",
                                    class: "player-chapter-label",
                                    "data-player-chapter-label": "",
                                    "data-player-action": "chapters",
                                    hidden: true,
                                    span { class: "player-chapter-title", "data-player-chapter-title": "" }
                                    ChevronRight { class: "player-chapter-chevron", size: 15 }
                                }
                                }
                                span { class: "player-control-spacer" }
                                button {
                                    r#type: "button",
                                    class: "player-control-button player-mute-button",
                                    aria_label: "Mute",
                                    title: "Mute",
                                    "data-player-action": "mute",
                                    span { class: "player-volume-icon", Volume2 { size: 20 } }
                                    span { class: "player-muted-icon", VolumeX { size: 20 } }
                                }
                                button {
                                    r#type: "button",
                                    class: "player-control-button player-captions-button",
                                    aria_label: "Captions",
                                    title: "Captions",
                                    "data-player-action": "captions",
                                    Captions { size: 21 }
                                }
                                button {
                                    r#type: "button",
                                    class: "player-caption-state-bridge",
                                    tabindex: "-1",
                                    aria_hidden: "true",
                                    "data-player-chapters-open": "",
                                    onclick: move |_| chapters_sheet.set(true),
                                }
                                NativePlayerBridges { title: playback_title.clone() }
                                button {
                                    r#type: "button",
                                    class: "player-caption-state-bridge",
                                    tabindex: "-1",
                                    aria_hidden: "true",
                                    "data-player-minimize": "",
                                    // Minimizing dismisses the watch sheet, so it has to pop
                                    // history rather than push Feed: pushing sent anyone who
                                    // opened the video from Subscriptions or a playlist to the
                                    // wrong page, and left the watch route ahead in history.
                                    // Feed is only the deep-link fallback.
                                    onclick: move |_| { spawn(animated_back_or_navigate(Route::Feed {})); },
                                }
                                button {
                                    r#type: "button",
                                    class: "player-caption-state-bridge",
                                    tabindex: "-1",
                                    aria_hidden: "true",
                                    "data-player-caption-toggle": "",
                                    onclick: move |_| {
                                        if !captions_for_toggle().is_empty() {
                                            captions_enabled.toggle();
                                        }
                                    },
                                }
                                button {
                                    r#type: "button",
                                    class: "player-control-button player-speed-control",
                                    aria_label: "Playback speed",
                                    title: "Change playback speed",
                                    "data-player-action": "speed",
                                    span { "data-player-speed-label": "", "{current_speed}×" }
                                }
                                if expanded {
                                    button {
                                        r#type: "button",
                                        class: "player-control-button player-theater-button",
                                        aria_label: if theater_mode { "Exit theater mode" } else { "Enter theater mode" },
                                        title: if theater_mode { "Exit theater mode" } else { "Theater mode" },
                                        aria_pressed: theater_mode.to_string(),
                                        onclick: move |event: Event<MouseData>| {
                                            event.stop_propagation();
                                            settings_for_theater.write().theater_mode = !theater_mode;
                                        },
                                        if theater_mode {
                                            PanelTopClose { size: 21 }
                                        } else {
                                            PanelTopOpen { size: 21 }
                                        }
                                    }
                                }
                                // Both icons ship; CSS shows whichever matches
                                // the current state, the same way play/pause does.
                                button {
                                    r#type: "button",
                                    class: "player-control-button player-fullscreen-button",
                                    aria_label: "Enter fullscreen",
                                    title: "Fullscreen",
                                    "data-player-action": "fullscreen",
                                    span { class: "player-fullscreen-enter-icon", Maximize2 { size: 21 } }
                                    span { class: "player-fullscreen-exit-icon", Minimize2 { size: 21 } }
                                }
                            }
                            div { class: "player-timeline",
                                div {
                                    class: "player-seek-preview",
                                    "data-player-seek-preview": "",
                                    hidden: true,
                                    div { class: "player-seek-preview-image", "data-player-preview-image": "" }
                                    div { class: "player-seek-preview-copy",
                                        strong { "data-player-preview-chapter": "" }
                                        span { "data-player-preview-time": "", "0:00" }
                                    }
                                }
                                input {
                                    r#type: "range",
                                    class: "player-progress",
                                    min: "0",
                                    max: "10000",
                                    step: "1",
                                    value: "0",
                                    aria_label: "Video progress",
                                    "data-player-progress": "",
                                }
                            }
                        }
                    }
                } else if use_embed() {
                    iframe {
                        id: "tawny-player-frame",
                        src: "{fallback_url}",
                        title: "{video.title}",
                        allow: "accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture",
                        allowfullscreen: true,
                    }
                } else if resolving {
                    // The spinner already says "loading"; naming the mechanism
                    // only reads as something going wrong.
                    div {
                        class: "player-stage-status",
                        Spinner {}
                    }
                } else {
                    // A drag down puts the player away here too; the controls
                    // script handles it, since there is no attached controller.
                    div {
                        class: "player-stage-status player-stage-error",
                    {
                        let failure = resolve_error.clone().unwrap_or_else(|| {
                            if nothing_extracted {
                                PlaybackFailure::new(
                                    PlaybackFailureOrigin::Bug,
                                    "The server returned streams this player cannot use.",
                                )
                                .remedy("Report it with the video link; the server log has the stream list.")
                            } else {
                                // The browser failed on streams the server
                                // handed over. The transport's detail below
                                // says where; the cause can be either side.
                                PlaybackFailure::new(
                                    PlaybackFailureOrigin::Unknown,
                                    "Streams were found, but the player could not play them.",
                                )
                                .remedy(
                                    "Try again. A 403 in the detail means YouTube cut the stream off; \
                                     anything else that repeats is likely a Tawny bug worth reporting.",
                                )
                                .detail(failure_detail())
                            }
                        });
                        let origin_class = match failure.origin {
                            PlaybackFailureOrigin::Youtube => "youtube",
                            PlaybackFailureOrigin::Setup => "setup",
                            PlaybackFailureOrigin::Bug => "bug",
                            PlaybackFailureOrigin::Unknown => "unknown",
                        };
                        rsx! {
                            span {
                                class: "player-stage-origin player-stage-origin-{origin_class}",
                                "{failure.origin.label()}"
                            }
                            p { class: "player-stage-summary", "{failure.summary}" }
                            if let Some(remedy) = failure.remedy.clone() {
                                p { class: "player-stage-remedy", "{remedy}" }
                            }
                            if let Some(detail) = failure.detail.clone() {
                                code { class: "player-stage-detail", "{detail}" }
                            }
                        }
                    }
                        div { class: "player-stage-actions",
                            Button {
                                fill: ButtonFill::Solid,
                                onclick: move |_| {
                                    playback_failed.set(false);
                                    stale_session_for.set(Some(retried_video_id.clone()));
                                    auto_retries.set(0);
                                    resolve_nonce += 1;
                                    // Rewinding to zero both re-runs the
                                    // resolver and restores the retry budget.
                                    playback_attempt.set(0);
                                },
                                "Try again"
                            }
                            Button {
                                fill: ButtonFill::Outline,
                                color: Color::Neutral,
                                size: ButtonSize::Sm,
                                onclick: move |_| use_embed.set(true),
                                "Use YouTube embed"
                            }
                        }
                    }
                }
                // The attached control bar carries its own Back, but it only
                // exists once a stream has resolved. Until then, and when
                // resolving failed, this is the way out.
                if expanded && !direct_stream && !use_embed() {
                    button {
                        r#type: "button",
                        class: "player-control-button player-stage-back",
                        aria_label: "Back",
                        title: "Back",
                        onclick: move |_| { spawn(animated_back_or_navigate(Route::Feed {})); },
                        ChevronLeft { size: 26 }
                    }
                }
            }
            if !expanded {
                button {
                    class: "mini-player-copy",
                    aria_label: "Open {video.title}",
                    onclick: move |_| { spawn(animated_navigate(Route::VideoDetail { id: open_video_id.clone() })); },
                    Text { variant: TextVariant::Label, class: "truncate", "{video.title}" }
                    Text { variant: TextVariant::Caption, class: "truncate", "{video.channel_name}" }
                }
                Button {
                    class: "mini-player-close self-center",
                    fill: ButtonFill::Clear,
                    color: Color::Neutral,
                    aria_label: "Close player",
                    onclick: move |_| app_state.stop_playback(),
                    X { size: 20 }
                }
            }
        }
    }
}

fn seek_player(seconds: u64) {
    spawn(async move {
        let script = format!(
            r#"
            const media = document.querySelector('#tawny-player video');
            if (media) {{ media.currentTime = {seconds}; media.play(); }}
            const frame = document.getElementById('tawny-player-frame');
            if (frame && frame.contentWindow) {{
                frame.contentWindow.postMessage(JSON.stringify({{
                    event: 'command', func: 'seekTo', args: [{seconds}, true]
                }}), '*');
            }}
            dioxus.send(true);
            "#
        );
        let mut eval = document::eval(&script);
        let _ = eval.recv::<bool>().await;
    });
}

fn toggle_picture_in_picture() {
    spawn(async move {
        let script = r#"
            const media = document.getElementById('tawny-player-media');
            let changed = false;
            try {
                if (document.pictureInPictureElement && document.exitPictureInPicture) {
                    await document.exitPictureInPicture();
                    changed = true;
                } else if (media && media.requestPictureInPicture) {
                    await media.requestPictureInPicture();
                    changed = true;
                } else if (media && media.webkitSupportsPresentationMode) {
                    const mode = media.webkitPresentationMode === 'picture-in-picture'
                        ? 'inline'
                        : 'picture-in-picture';
                    media.webkitSetPresentationMode(mode);
                    changed = true;
                }
            } catch (_) {}
            if (!changed) {
                if (media) media.__tawnyPlaybackIntent = !media.paused && !media.ended;
                window.dispatchEvent(new Event('tawnynativepiprequest'));
                const portrait = media && media.videoHeight > media.videoWidth;
                const selector = portrait
                    ? '#tawny-player [data-player-native-pip-portrait]'
                    : '#tawny-player [data-player-native-pip-landscape]';
                document.querySelector(selector)?.click();
                changed = Boolean(document.querySelector(selector));
            }
            dioxus.send(changed);
        "#;
        let mut eval = document::eval(script);
        let _ = eval.recv::<bool>().await;
    });
}

fn scroll_video_page_to_top() {
    spawn(async move {
        // The shell's scroll container is `.g3-body-content`; its `.g3-body`
        // parent is `overflow: hidden`, so resetting the parent (or the window)
        // does nothing. That container outlives route changes, so it arrives on
        // the video page still holding the feed's offset.
        //
        // Content lands in stages — placeholder, cached details, then the
        // remote payload — and each stage can restore scroll height that the
        // browser had clamped away, so reassert the reset across a few frames
        // instead of once.
        let mut eval = document::eval(
            r#"
            const reset = () => {
                const containers = document.querySelectorAll(
                    '.g3-body-content, .g3-body, [data-g3-body], .tawny-shell'
                );
                for (const element of containers) {
                    if (element.scrollTop !== 0) element.scrollTop = 0;
                }
                if (document.scrollingElement) document.scrollingElement.scrollTop = 0;
                if (window.scrollY !== 0) window.scrollTo({ top: 0, left: 0, behavior: 'auto' });
            };
            reset();
            for (let frame = 0; frame < 4; frame++) {
                await new Promise((resolve) => requestAnimationFrame(resolve));
                reset();
            }
            for (const delay of [60, 180]) {
                await new Promise((resolve) => setTimeout(resolve, delay));
                reset();
            }
            dioxus.send(true);
            "#,
        );
        let _ = eval.recv::<bool>().await;
    });
}

fn compact_number(value: u64) -> String {
    let value = value as f64;
    if value >= 1_000_000_000.0 {
        format!("{:.1}B", value / 1_000_000_000.0)
    } else if value >= 1_000_000.0 {
        format!("{:.1}M", value / 1_000_000.0)
    } else if value >= 1_000.0 {
        format!("{:.1}K", value / 1_000.0)
    } else {
        format!("{}", value as u64)
    }
    .replace(".0", "")
}

fn local_details(video: Video, related_videos: Vec<Video>) -> VideoDetails {
    VideoDetails {
        video,
        channel: None,
        description: String::new(),
        like_count: 0,
        dislike_count: 0,
        captions: Vec::new(),
        chapters: Vec::new(),
        preview_frames: None,
        related_videos,
        comments: Default::default(),
        remote_available: false,
    }
}

/// Have the server resolve a video's playback now, ahead of the player asking.
///
/// The answer is dropped: what matters is that the server holds the session,
/// or is already resolving it, when the player's own request arrives.
fn warm_playback(video_id: String, prefer_sabr: bool) {
    spawn(async move {
        let _ = resolve_playback(video_id, prefer_sabr, false).await;
    });
}

fn source_is_transportable(source: &PlaybackSource) -> bool {
    if !source.request_headers.is_empty() {
        return false;
    }
    match source.protocol {
        PlaybackProtocol::Hls | PlaybackProtocol::Dash | PlaybackProtocol::Progressive => true,
        PlaybackProtocol::Sabr => {
            !source.tracks.is_empty()
                || source
                    .mime_type
                    .as_deref()
                    .is_some_and(|mime| mime.contains("dash+xml") || mime.contains("mpegurl"))
        }
        PlaybackProtocol::EmbedFallback => false,
    }
}

/// Sizes the comments sheet so it reaches up to the bottom of the video and
/// stops there.
///
/// Where that edge sits depends on the shell's width, the player's own cap, and
/// how far the page is scrolled, none of which a stylesheet can see. So it is
/// measured when the sheet opens, again a frame later once the sheet has laid
/// out, and on every resize while it is up. `offsetHeight` rather than a
/// bounding box, because the sheet is mid-transform while it opens.
const COMMENTS_SHEET_FIT_JS: &str = r#"
const key = Symbol.for('tawny.comments-sheet-fit');
window[key]?.dispose?.();
const fit = () => {
    const sheet = document.querySelector('.comments-sheet');
    const content = sheet?.querySelector('.g3-sheet-content');
    const player = document.querySelector('#tawny-player');
    if (!sheet || !content || !player) return;
    const viewport = window.visualViewport?.height ?? window.innerHeight;
    const playerBottom = Math.min(viewport, Math.max(0, player.getBoundingClientRect().bottom));
    // A wide shell floats the sheet above the bottom edge; a phone does not.
    const gap = parseFloat(getComputedStyle(sheet).bottom) || 0;
    // The handle and anything else the sheet wraps around its content.
    const chrome = sheet.offsetHeight - content.offsetHeight;
    const style = getComputedStyle(content);
    const padding = style.boxSizing === 'border-box'
        ? 0
        : parseFloat(style.paddingTop) + parseFloat(style.paddingBottom);
    // Up to the video, but never less than half the screen. On a phone the
    // video leaves most of the height free; a wide window shows a 1180px player
    // that can leave a strip too short to read a single comment in, and there
    // the sheet covers the bottom of the video instead.
    const fitted = viewport - gap - chrome - playerBottom - padding;
    const floor = viewport * 0.5 - gap - chrome - padding;
    const height = Math.floor(Math.max(fitted, floor));
    document.documentElement.style.setProperty('--tawny-comments-sheet-height', `${height}px`);
};
// Holds the page still underneath, so the video cannot scroll away from the
// edge the sheet was sized to. See `html.tawny-comments-open` in the stylesheet.
document.documentElement.classList.add('tawny-comments-open');
fit();
requestAnimationFrame(fit);
window.addEventListener('resize', fit);
window.visualViewport?.addEventListener('resize', fit);
window[key] = {
    dispose() {
        window.removeEventListener('resize', fit);
        window.visualViewport?.removeEventListener('resize', fit);
    },
};
"#;

/// Stops re-fitting once the sheet closes and lets the page scroll again. The
/// measured height is left in place so the closing animation does not change
/// size under it.
const COMMENTS_SHEET_UNFIT_JS: &str = r#"
window[Symbol.for('tawny.comments-sheet-fit')]?.dispose?.();
document.documentElement.classList.remove('tawny-comments-open');
"#;

/// Splits the route param off from the page so that changing it builds a new
/// page rather than re-rendering the old one.
///
/// The key has to hang off a *list* to do anything. Dioxus only compares keys in
/// `diff_keyed_children`, which runs for siblings; a single keyed child goes down
/// the positional path where the key is never read, so the inner page kept its
/// scope across a watch-to-watch navigation and every `use_resource` and
/// `use_effect` in it went on serving the video it had captured at mount. The
/// address bar changed and nothing else did - by autoplay, by a related video,
/// by anything that moved between two watch pages.
///
/// A one-element loop is a keyed sibling list, so the key is compared, and a new
/// id unmounts the old page and mounts a fresh one.
#[component]
pub fn VideoDetail(id: String) -> Element {
    rsx! {
        for video_id in [id.clone()] {
            VideoDetailInner { key: "{video_id}", id: video_id.clone() }
        }
    }
}

#[component]
fn VideoDetailInner(id: String) -> Element {
    let mut app_state = use_context::<AppState>();

    let mut description_expanded = use_signal(|| false);
    let mut chapters_open = app_state.chapters_sheet_open;
    let mut captions_open = use_signal(|| false);
    let mut audio_open = use_signal(|| false);
    let audio_tracks = app_state.active_audio_tracks;
    let selected_audio = app_state.selected_audio_track;
    let mut comments_open = use_signal(|| false);
    use_effect(move || {
        let _ = document::eval(if comments_open() {
            COMMENTS_SHEET_FIT_JS
        } else {
            COMMENTS_SHEET_UNFIT_JS
        });
    });
    // Leaving the video with comments still open - Back, autoplay, a related
    // video - unmounts this page without the effect above ever seeing the sheet
    // close, and the next page would inherit a column that cannot scroll.
    use_drop(|| {
        let _ = document::eval(COMMENTS_SHEET_UNFIT_JS);
    });
    // Everything this route can put off until the sheet has landed - see
    // AFTER_ROUTE_TRANSITION_JS.
    //
    // The player is the only thing in a cover transition whose two snapshots
    // differ in size, so its `::view-transition-group` animates width and
    // height while every other group is a pure translate. Width and height are
    // main-thread work: the sheet glides on the compositor whatever happens,
    // and the player alone starves on any long task this route runs while the
    // animation is in flight - which is why opening stutters and minimizing,
    // landing on a page that is already built, never does.
    //
    // So nothing renders here that the first 0.6s does not need.
    let mut transition_settled = use_signal(|| false);
    use_effect(move || {
        spawn(async move {
            let mut eval = document::eval(AFTER_ROUTE_TRANSITION_JS);
            if eval.recv::<bool>().await.is_ok() {
                transition_settled.set(true);
            }
        });
    });
    // SponsorBlock, fetched per video and per category set. Held behind the
    // same gate as everything else here: segments are not needed in the first
    // frame, and the request landing mid-morph is what starves it.
    let mut sponsor_segments = app_state.active_sponsor_segments;
    let sponsor_video_id = id.clone();
    use_effect(move || {
        if !transition_settled() {
            return;
        }
        let video_id = sponsor_video_id.clone();
        let categories = app_state
            .settings()
            .sponsor_block
            .requested_categories()
            .iter()
            .map(|category| category.api_name())
            .collect::<Vec<_>>()
            .join(",");
        spawn(async move {
            if categories.is_empty() {
                sponsor_segments.set(Vec::new());
                return;
            }
            // A failure here is not worth a toast: SponsorBlock being
            // unreachable means the video plays exactly as it would have.
            // It does not mean the segments already on the timeline are
            // wrong, though, so a failed request leaves them alone. This
            // effect runs again on every settings change, and answering each
            // failure with "no segments" is what wiped the colours part way
            // through a video.
            if let Ok(fetched) = get_sponsor_segments(video_id, categories).await {
                sponsor_segments.set(fetched);
            }
        });
    });
    let mut comments = use_signal(Vec::<VideoComment>::new);
    let mut comments_next_page = use_signal(|| None::<String>);
    let mut comments_initialized = use_signal(|| false);
    let mut comments_loading = use_signal(|| false);
    let mut details_cached = use_signal(|| false);
    // Keyed by video rather than a flag: this page is the only place history is
    // recorded, so it must still record when it is reused for another video.
    let mut history_recorded_for = use_signal(|| None::<String>);
    let route_video_id = id.clone();
    use_effect(move || {
        let _ = &route_video_id;
        scroll_video_page_to_top();
    });

    // A video missing from the library only starts playing once its details
    // arrive, and the player resolves playback only once it starts - two
    // multi-second requests back to back. Asking for playback now runs them
    // side by side; the player's own request then joins this one. The video
    // already playing (a mini player being expanded) has its session.
    let warm_video_id = id.clone();
    use_hook(move || {
        if app_state
            .active_video
            .peek()
            .as_ref()
            .is_none_or(|active| active.id != warm_video_id)
        {
            warm_playback(warm_video_id, app_state.settings.peek().prefer_sabr);
        }
    });

    // Cached per video, so reopening one (or opening it offline) shows its
    // details at once while they refresh behind it.
    let details_resource = use_cached(get_video_details, (id.clone(),));
    // What a card handed over when it was tapped (see `AppState::play`), so
    // the page has something to show before the details arrive. A peek: the
    // playing copy changes every second of playback.
    let local_id = id.clone();
    let cached_video = app_state
        .active_video
        .peek()
        .clone()
        .filter(|video| video.id == local_id);
    // Watched, as the button below last set it. The details are fetched once
    // and never hear that it changed. A memo, so playback ticks don't rerender.
    let watched_id_for_memo = id.clone();
    let active_watched = use_memo(move || {
        app_state
            .active_video
            .read()
            .as_ref()
            .filter(|video| video.id == watched_id_for_memo)
            .map(|video| video.watched)
    });
    // Swapping the handed-over copy for the remote one re-renders the whole
    // details body, and opening from the mini player is exactly the case where
    // the local copy is already correct. Waiting costs nothing visible: the
    // page is showing the right video throughout, and 0.6s later it quietly
    // gets the remote refinements. With nothing local there is a loading state
    // on screen instead, and holding *that* back would be worse than the
    // stutter.
    let hold_remote_details = cached_video.is_some() && !transition_settled();
    let remote_details = if hold_remote_details {
        None
    } else {
        details_resource
            .read()
            .as_ref()
            .and_then(|result| result.as_ref().ok())
            .cloned()
    };
    use_effect(move || {
        if !transition_settled() || details_cached() {
            return;
        }
        let details = details_resource
            .read()
            .as_ref()
            .and_then(|result| result.as_ref().ok())
            .cloned();
        if let Some(details) = details.as_ref() {
            comments.set(details.comments.comments.clone());
            comments_next_page.set(details.comments.next_page.clone());
            comments_initialized.set(true);
            details_cached.set(true);
        }
    });

    let details = remote_details.clone().or_else(|| {
        cached_video
            .clone()
            .map(|video| local_details(video, Vec::new()))
    });
    let player_details_resource = details_resource;
    let local_for_player = cached_video.clone();
    use_effect(move || {
        let local = local_for_player
            .clone()
            .map(|video| local_details(video, Vec::new()));
        // `set_player_metadata` re-renders the player - its caption `<track>`
        // children come from here - and during a cover transition that player
        // is the element the morph is painting live. So the remote copy waits
        // out the animation whenever the library already answered, on the same
        // terms as `hold_remote_details`. Nothing cached means nothing is
        // playing yet, and starting playback is worth more than a smooth 0.6s.
        let details = if local.is_some() && !transition_settled() {
            local
        } else {
            player_details_resource
                .read()
                .as_ref()
                .and_then(|result| result.as_ref().ok())
                .cloned()
                .or(local)
        };
        if let Some(details) = details {
            let video = &details.video;
            if app_state
                .active_video()
                .as_ref()
                .is_none_or(|active| active.id != video.id)
            {
                app_state.play(video.clone());
            }
            app_state.set_player_metadata(
                details.captions,
                details.chapters,
                details.preview_frames,
            );
            // A library write, so it waits out the sheet like the details
            // cache does: it re-renders everything on this page that reads the
            // library, in the middle of the morph.
            if transition_settled()
                && history_recorded_for.peek().as_deref() != Some(video.id.as_str())
            {
                app_state.record_history(&video.id);
                history_recorded_for.set(Some(video.id.clone()));
            }
        }
    });

    let Some(details) = details else {
        let failed = details_resource.read().as_ref().is_some();
        return rsx! {
            // Carries the overlay region too: the sheet has to exist in the new
            // DOM when the snapshot is taken, and details arrive later than
            // that. Without it the route committed first and the animation
            // then played over the page it had already swapped to.
            Content {
                class: "player-loading {ROUTE_TRANSITION_OVERLAY_REGION_CLASS}",
                if failed {
                    EmptyState {
                        title: "Video unavailable",
                        color: Color::Danger,
                        "This video could not be loaded from YouTube or the server's cache."
                    }
                } else {
                    Spinner { center: true }
                }
            }
        };
    };

    let video = details.video.clone();
    let channel = details.channel.clone();
    let related = details.related_videos.clone();
    let watched_video = video.clone();
    let is_watched = active_watched().unwrap_or(video.watched);
    let watched_icon = if is_watched {
        rsx! { EyeOff { size: 17 } }
    } else {
        rsx! { Check { size: 17 } }
    };
    let playlist_video = video.clone();
    let share_video = video.clone();
    let open_channel_id = video.channel_id.clone();
    // The viewer first, for the same reason as `is_watched` above: it is what
    // the button writes, while the details are fetched once and never hear of
    // the change - and they can come from a cache that predates your follow.
    let is_subscribed = app_state.follows(&video.channel_id);
    let channel_to_toggle = channel.clone();
    let caption_tracks = details.captions.clone();
    let chapters = details.chapters.clone();
    let comments_disabled = details.comments.disabled;
    // The seeding effect waits out the transition, so until it has run the
    // count comes from the details themselves rather than reading 0.
    let comments_count = if comments_initialized() {
        comments().len()
    } else {
        details.comments.comments.len()
    };
    let details_remote_available = details.remote_available;

    rsx! {
        // The sheet itself: this is what slides up over the shell and back
        // down off it, so it is the overlay region.
        Content { class: "player-content player-detail-content {ROUTE_TRANSITION_OVERLAY_REGION_CLASS}",
            Stack { gap: Space::Lg, class: "player-details",
                // Title and stats are one unit: the page's section gap belongs
                // between blocks, not between a title and its own byline.
                Stack { gap: Space::Xs,
                    Text { variant: TextVariant::Title, "{video.title}" }
                    Text { tone: TextTone::Secondary, "{video.stats_label()}" }
                }

                // Captions and chapters are chips that open their own sheet,
                // so the page is not padded out with two panels that are mostly
                // idle. The counts are not actions, so they are badges rather
                // than chips that do nothing.
                if details.like_count > 0 || details.dislike_count > 0 || !caption_tracks.is_empty() || !chapters.is_empty() || audio_tracks().len() > 1 {
                    Stack { horizontal: true, wrap: true, gap: Space::Sm, align: StackAlign::Center,
                        if details.like_count > 0 {
                            Chip { start: rsx! { ThumbsUp { size: 15 } },
                                "{compact_number(details.like_count)}"
                            }
                        }
                        if details.dislike_count > 0 {
                            Chip { start: rsx! { ThumbsDown { size: 15 } },
                                "{compact_number(details.dislike_count)}"
                            }
                        }
                        if !caption_tracks.is_empty() {
                            Chip {
                                start: rsx! { Captions { size: 15 } },
                                onclick: move |_| captions_open.set(true),
                                "{caption_tracks.len()} captions"
                            }
                        }
                        // Only a dubbed upload offers a choice here, and only
                        // then is the chip worth a slot in this row.
                        if audio_tracks().len() > 1 {
                            Chip {
                                start: rsx! { Languages { size: 15 } },
                                onclick: move |_| audio_open.set(true),
                                "{audio_track_label(&audio_tracks(), &selected_audio())}"
                            }
                        }
                        if !chapters.is_empty() {
                            Chip {
                                start: rsx! { ListVideo { size: 15 } },
                                onclick: move |_| chapters_open.set(true),
                                "{chapters.len()} chapters"
                            }
                        }
                    }
                }

                Shelf { aria_label: "Video actions", gap: Space::Sm,
                    Button {
                        fill: ButtonFill::Outline,
                        color: Color::Neutral,
                        size: ButtonSize::Sm,
                        start: watched_icon,
                        onclick: move |_| {
                            app_state.mark_watched(&watched_video, !is_watched);
                            app_state.show_toast(
                                if is_watched { "Marked as unwatched" } else { "Marked as watched" },
                                Color::Success,
                            );
                        },
                        if is_watched { "Unwatched" } else { "Watched" }
                    }
                    Button {
                        fill: ButtonFill::Outline,
                        color: Color::Neutral,
                        size: ButtonSize::Sm,
                        start: rsx! { Clock { size: 17 } },
                        onclick: move |_| {
                            app_state.playlist_picker_video.set(Some(playlist_video.clone()));
                            app_state.playlist_picker_open.set(true);
                        },
                        "Save"
                    }
                    Button {
                        fill: ButtonFill::Outline,
                        color: Color::Neutral,
                        size: ButtonSize::Sm,
                        start: rsx! { Share2 { size: 17 } },
                        onclick: move |_| app_state.open_share(share_video.clone()),
                        "Share"
                    }
                    Button {
                        fill: ButtonFill::Outline,
                        color: Color::Neutral,
                        size: ButtonSize::Sm,
                        start: rsx! { PictureInPicture { size: 17 } },
                        onclick: move |_| toggle_picture_in_picture(),
                        "PiP"
                    }
                }

                // The cached copy of a video knows neither its channel nor its
                // description, so both cards were absent until the details
                // request answered and then inserted themselves, growing the
                // page under whatever the reader was already looking at. A
                // placeholder of the same height holds their place instead.
                if channel.is_none() && !details_remote_available {
                    Card {
                        start: rsx! { Skeleton { shape: SkeletonShape::Avatar } },
                        Stack { gap: Space::Sm,
                            Skeleton { width: "45%" }
                            Skeleton { width: "28%" }
                        }
                    }
                }
                if let Some(channel) = channel {
                    Card {
                        title: channel.name.clone(),
                        subtitle: (!channel.subscriber_count.trim().is_empty())
                            .then(|| format!("{} subscribers", channel.subscriber_count.trim())),
                        start: rsx! {
                            Avatar { name: channel.name.clone(), src: channel.avatar_url.clone(), size: AvatarSize::Md }
                        },
                        onclick: move |_| { spawn(animated_navigate(Route::ChannelDetail { id: open_channel_id.clone() })); },
                        end: rsx! {
                            Button {
                                size: ButtonSize::Sm,
                                fill: if is_subscribed { ButtonFill::Outline } else { ButtonFill::Solid },
                                color: if is_subscribed { Color::Neutral } else { Color::Accent },
                                "aria-pressed": if is_subscribed { "true" } else { "false" },
                                onclick: move |_| {
                                    if let Some(channel) = channel_to_toggle.as_ref() {
                                        app_state.toggle_subscription_for(channel);
                                    }
                                },
                                if is_subscribed { "Subscribed" } else { "Subscribe" }
                            }
                        },
                    }
                }

                if details.description.is_empty() && !details_remote_available {
                    Card { title: "Description",
                        Stack { gap: Space::Sm,
                            // Four lines: the same as the clamp the real
                            // description opens at.
                            Skeleton { width: "100%" }
                            Skeleton { width: "96%" }
                            Skeleton { width: "88%" }
                            Skeleton { width: "60%" }
                            // The 'Show more' button below the text is part of
                            // the height being held.
                            Skeleton { class: "h-8 mt-1", width: "5.5rem" }
                        }
                    }
                }
                if !details.description.is_empty() {
                    Card {
                        title: "Description",
                        end: (!details_remote_available).then(|| rsx! { Badge { "Cached" } }),
                        Text {
                            class: if description_expanded() { "whitespace-pre-line break-words" } else { "line-clamp-4 whitespace-pre-line break-words" },
                            Linkified {
                                text: details.description.clone(),
                                on_seek: move |seconds| seek_player(seconds),
                            }
                        }
                        Button {
                            fill: ButtonFill::Clear,
                            size: ButtonSize::Sm,
                            onclick: move |_| description_expanded.toggle(),
                            if description_expanded() { "Show less" } else { "Show more" }
                        }
                    }
                }

                Button {
                    fill: ButtonFill::Outline,
                    color: Color::Neutral,
                    expand: ButtonExpand::Block,
                    disabled: comments_disabled,
                    start: rsx! { MessageSquare { size: 17 } },
                    onclick: move |_| comments_open.set(true),
                    if comments_disabled {
                        "Comments are disabled"
                    } else {
                        "Comments · {comments_count}"
                    }
                }

                // Heading and grid are one block, held closer together than
                // the sections above them.
                Stack { gap: Space::Sm,
                    Text { variant: TextVariant::Title,
                        if details.remote_available { "Related videos" } else { "From your library" }
                    }
                    if transition_settled() {
                        VideoGrid { videos: related }
                    }
                }
            }
        }

        BottomSheet { open: chapters_open, title: "Chapters", class: "player-sheet",
            max_height: "74dvh",
            List { lines: ListLines::Inset,
                // A closed sheet still renders its list. This one cannot be
                // open while the route that owns it is animating in, so it is
                // pure cost there - and a long upload has a hundred rows.
                for chapter in if transition_settled() { chapters.clone() } else { Vec::new() } {
                    {
                        let start_seconds = chapter.start_seconds;
                        rsx! {
                            Item {
                                key: "{chapter.start_seconds}",
                                label: chapter.title.clone(),
                                metadata: chapter.timestamp_label(),
                                onclick: move |_| {
                                    seek_player(start_seconds);
                                    chapters_open.set(false);
                                },
                            }
                        }
                    }
                }
            }
        }

        BottomSheet { open: captions_open, title: "Captions", class: "player-sheet",
            max_height: "74dvh",
            CaptionPicker {
                tracks: caption_tracks.clone(),
                selected: app_state.selected_caption,
            }
        }

        BottomSheet { open: audio_open, title: "Audio track", class: "player-sheet",
            max_height: "74dvh",
            AudioTrackPicker {
                tracks: audio_tracks(),
                selected: selected_audio(),
            }
        }

        // No heading and no running count: the button that opened this already
        // said "Comments", so the sheet is just the list. Load-more lives at the
        // end of the scrolled list rather than pinned above it.
        // No backdrop: the video above stays untinted and usable while the
        // thread is open, and dragging the handle down is how it closes.
        BottomSheet {
            open: comments_open,
            aria_label: "Comments",
            backdrop: SheetBackdrop::None,
            class: "player-sheet comments-sheet",
            max_height: "var(--tawny-comments-sheet-height, 60dvh)",
            // Same as the chapter list, and the heaviest of the two: twenty
            // cards, each with an avatar, built while the player is trying to
            // grow.
            if !transition_settled() {
                Spinner { center: true }
            } else if comments().is_empty() {
                EmptyState {
                    title: "No comments",
                    if comments_initialized() && !details.comments.remote_available {
                        "Comments need a connected video source."
                    } else {
                        "No comments are available."
                    }
                }
            } else {
                Stack { gap: Space::Sm,
                    for comment in comments() {
                        CommentCard { comment }
                    }
                    if let Some(next_page) = comments_next_page() {
                        Button {
                            fill: ButtonFill::Outline,
                            color: Color::Neutral,
                            expand: ButtonExpand::Block,
                            loading: comments_loading(),
                            onclick: move |_| {
                                let video_id = video.id.clone();
                                let token = next_page.clone();
                                comments_loading.set(true);
                                spawn(async move {
                                    match get_comments_page(video_id, token).await {
                                        Ok(page) => {
                                            comments.write().extend(page.comments);
                                            comments_next_page.set(page.next_page);
                                        }
                                        Err(_) => app_state.show_toast(
                                            "Could not load more comments",
                                            Color::Warning,
                                        ),
                                    }
                                    comments_loading.set(false);
                                });
                            },
                            "Load more comments"
                        }
                    }
                }
            }
        }
    }
}

/// What the chip says: the language you are hearing, not a count.
///
/// A count would be the wrong fact here - the question a dubbed video raises is
/// "which language is this?", and the answer is the thing to show.
fn audio_track_label(tracks: &[AudioTrackOption], selected: &Option<String>) -> String {
    selected
        .as_ref()
        .and_then(|language| tracks.iter().find(|track| &track.language == language))
        .map(|track| track.label.clone())
        .unwrap_or_else(|| "Audio".to_string())
}

#[component]
fn AudioTrackPicker(tracks: Vec<AudioTrackOption>, selected: Option<String>) -> Element {
    rsx! {
        Stack { horizontal: true, wrap: true, gap: Space::Sm,
            for track in tracks {
                Chip {
                    key: "{track.language}",
                    selected: selected.as_deref() == Some(track.language.as_str()),
                    onclick: {
                        let language = track.language.clone();
                        move |_| set_player_audio_track(language.clone())
                    },
                    "{track.label}"
                }
            }
        }
    }
}

#[component]
fn CaptionPicker(tracks: Vec<CaptionTrack>, mut selected: Signal<Option<usize>>) -> Element {
    rsx! {
        Stack { horizontal: true, wrap: true, gap: Space::Sm,
            for (index, track) in tracks.into_iter().enumerate() {
                Chip {
                    key: "{index}",
                    selected: selected() == Some(index),
                    onclick: move |_| selected.set(Some(index)),
                    "{track.label}"
                    if track.auto_generated { " · Auto" }
                }
            }
        }
    }
}

#[component]
fn CommentCard(comment: VideoComment) -> Element {
    rsx! {
        Card {
            variant: if comment.pinned { CardVariant::Flat } else { CardVariant::Filled },
            title: comment.author.clone(),
            heading_level: 3,
            subtitle: comment.published_at.clone(),
            start: rsx! {
                Avatar { name: comment.author.clone(), src: comment.author_avatar_url.clone(), size: AvatarSize::Sm }
            },
            end: rsx! {
                if comment.creator { Badge { color: Color::Accent, "Creator" } }
                if comment.pinned { Badge { "Pinned" } }
            },
            Stack { gap: Space::Xs,
                Text { class: "whitespace-pre-line break-words", "{comment.text}" }
                Stack { horizontal: true, gap: Space::Md, align: StackAlign::Center,
                    Text { variant: TextVariant::Caption, class: "inline-flex items-center gap-1", ThumbsUp { size: 13 } "{compact_number(comment.like_count)}" }
                    if comment.reply_count > 0 {
                        Text { variant: TextVariant::Caption, "{comment.reply_count} replies" }
                    }
                    if comment.hearted {
                        Text { variant: TextVariant::Caption, class: "inline-flex items-center gap-1", Heart { size: 13 } "Hearted" }
                    }
                }
            }
        }
    }
}

/// A segment in the shape the player JS wants.
///
/// Deliberately not `SponsorSegment` itself: the JS needs the resolved colour,
/// the resolved action and the message to show, none of which are properties of
/// the segment - they come from the viewer's settings. Resolving them here
/// keeps that decision in Rust, where the settings live, instead of shipping
/// the settings to JS and duplicating the logic.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
struct SponsorTimelineSegment {
    uuid: String,
    start_seconds: f64,
    end_seconds: f64,
    color: &'static str,
    /// `"skip"` or `"show"`. Ignored categories never reach here.
    action: &'static str,
    message: &'static str,
    label: &'static str,
}

/// Resolve segments against the viewer's settings, dropping the ignored ones.
fn timeline_segments(
    segments: &[SponsorSegment],
    settings: &SponsorBlockSettings,
) -> Vec<SponsorTimelineSegment> {
    segments
        .iter()
        .filter_map(|segment| {
            let action = match settings.action_for(segment.category) {
                SponsorAction::Off => return None,
                SponsorAction::Skip => "skip",
                SponsorAction::Show => "show",
            };
            Some(SponsorTimelineSegment {
                uuid: segment.uuid.clone(),
                start_seconds: segment.start_seconds,
                end_seconds: segment.end_seconds,
                color: segment.category.color(),
                action,
                message: segment.category.skip_message(),
                label: segment.category.label(),
            })
        })
        .collect()
}

#[cfg(test)]
mod timeline_tests {
    use super::*;
    // Imported here rather than at file scope: the non-test build resolves
    // categories on the server side only, so a top-level import is dead there.
    use crate::models::SponsorCategory;

    fn segment(category: SponsorCategory, start: f64, end: f64) -> SponsorSegment {
        SponsorSegment {
            uuid: format!("{category:?}"),
            category,
            start_seconds: start,
            end_seconds: end,
            locked: false,
            votes: 0,
        }
    }

    #[test]
    fn an_ignored_category_never_reaches_the_player() {
        let mut settings = SponsorBlockSettings::default();
        settings.set_action(SponsorCategory::Sponsor, SponsorAction::Off);
        let resolved = timeline_segments(
            &[
                segment(SponsorCategory::Sponsor, 0.0, 10.0),
                segment(SponsorCategory::Intro, 10.0, 20.0),
            ],
            &settings,
        );
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].action, "show");
    }

    #[test]
    fn the_action_the_player_gets_is_the_viewers_choice() {
        let mut settings = SponsorBlockSettings::default();
        settings.set_action(SponsorCategory::Intro, SponsorAction::Skip);
        let resolved = timeline_segments(&[segment(SponsorCategory::Intro, 0.0, 5.0)], &settings);
        assert_eq!(resolved[0].action, "skip");
        assert_eq!(resolved[0].color, SponsorCategory::Intro.color());
    }

    #[test]
    fn turning_sponsorblock_off_empties_the_timeline() {
        let settings = SponsorBlockSettings {
            enabled: false,
            ..SponsorBlockSettings::default()
        };
        assert!(
            timeline_segments(&[segment(SponsorCategory::Sponsor, 0.0, 10.0)], &settings)
                .is_empty()
        );
    }
}

#[cfg(test)]
mod route_tests {
    /// A string check, because the thing being guarded is a diffing rule rather
    /// than anything this crate can call: Dioxus compares keys only in
    /// `diff_keyed_children`, which runs for a list of siblings. Written as a
    /// single keyed child the key is never read, the page keeps its scope across
    /// a watch-to-watch navigation, and every hook in it goes on serving the
    /// video it captured at mount. Nothing errors - the address bar changes and
    /// the page does not - so only the shape of the call can defend it.
    #[test]
    fn the_watch_page_is_keyed_through_a_sibling_list() {
        let source = include_str!("player.rs");
        let body = source
            .split("pub fn VideoDetail(id: String) -> Element {")
            .nth(1)
            .expect("the route component");
        let body = body.split("#[component]").next().expect("its body");
        assert!(
            body.contains("for video_id in [id.clone()]"),
            "VideoDetailInner must be keyed inside a list or it never remounts: {body}"
        );
    }
}
