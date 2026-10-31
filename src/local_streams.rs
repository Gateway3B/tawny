//! Resolving playback on the phone, so video comes straight from YouTube
//! instead of through the home server.
//!
//! The server resolves with its yt-dlp sidecar, and the stream URLs it gets
//! belong to its own address, so every byte has to come back through it: far
//! from home, that round trip is the slow part. Here yt-dlp runs in the app
//! (`android/localstreams`: an embedded Python, with YouTube's challenges solved
//! in Android's V8 sandbox), the URLs belong to the phone, and the local relay
//! (`local_relay`) hands them to the player. See docs/on-device-playback.md.
//!
//! Any failure falls back to the server (`playback_resolver`), so this can only
//! add the seconds a failed attempt took, never lose playback.

use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

use jni::{
    JavaVM,
    objects::{GlobalRef, JClass, JObject, JString, JValue},
};
use serde::Deserialize;

use crate::{
    local_relay,
    models::{PlaybackSession, PlaybackSource},
};

/// Declared for its side effect: this is what tells `dx` to build and bundle
/// the Kotlin module. The calls go through JNI below, like g3-native-plugins'.
#[manganis::ffi("android/localstreams")]
unsafe extern "Kotlin" {
    pub type LocalStreamsPlugin;
}

const PLUGIN_CLASS: &str = "net.gateway3.tawny.localstreams.LocalStreamsPlugin";

/// A first resolve also starts Python and the JavaScript sandbox. Past this
/// the server is asked instead.
const RESOLVE_TIMEOUT: Duration = Duration::from_secs(45);

/// How long a resolved session is reused, as the server holds its own. The
/// URLs last about six hours.
const SESSION_LIFETIME: Duration = Duration::from_secs(60 * 60);

static SESSIONS: OnceLock<Mutex<HashMap<String, (Instant, PlaybackSession)>>> = OnceLock::new();

/// A session for `video_id` resolved on this phone. `fresh` skips the one held
/// from earlier, for a player whose session stopped working.
pub async fn resolve(video_id: &str, fresh: bool) -> Result<PlaybackSession, String> {
    let sessions = SESSIONS.get_or_init(Default::default);
    if !fresh
        && let Some((resolved, session)) = sessions
            .lock()
            .ok()
            .and_then(|held| held.get(video_id).cloned())
        && resolved.elapsed() < SESSION_LIFETIME
    {
        return Ok(session);
    }

    let (sender, receiver) = tokio::sync::oneshot::channel();
    let id = video_id.to_string();
    // yt-dlp blocks for seconds; it gets a thread of its own, never the UI's.
    std::thread::Builder::new()
        .name("tawny-local-resolve".into())
        .spawn(move || {
            let _ = sender.send(call_plugin(&id));
        })
        .map_err(|error| format!("Could not start resolving: {error}"))?;
    let answer = tokio::time::timeout(RESOLVE_TIMEOUT, receiver)
        .await
        .map_err(|_| "Resolving on this device took too long".to_string())?
        .map_err(|_| "Resolving on this device stopped".to_string())??;

    let source = match serde_json::from_str::<Answer>(&answer)
        .map_err(|error| format!("The device resolver answered something unreadable: {error}"))?
    {
        Answer::Source(source) => source,
        Answer::Error(error) => return Err(error),
    };
    let session = through_relay(video_id, source)?;
    if let Ok(mut held) = sessions.lock() {
        held.retain(|_, (resolved, _)| resolved.elapsed() < SESSION_LIFETIME);
        held.insert(video_id.to_string(), (Instant::now(), session.clone()));
    }
    Ok(session)
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum Answer {
    Source(PlaybackSource),
    Error(String),
}

/// Point every track at the local relay, which fetches it with the headers it
/// was resolved with. The player then sees an ordinary header-less source.
fn through_relay(video_id: &str, mut source: PlaybackSource) -> Result<PlaybackSession, String> {
    let relay = local_relay::relay()?;
    for track in &mut source.tracks {
        let headers = std::mem::take(&mut track.request_headers);
        track.url = relay.register(std::mem::take(&mut track.url), headers);
    }
    if let Some(first) = source.tracks.first() {
        source.url = first.url.clone();
    }
    source.request_headers.clear();
    Ok(PlaybackSession {
        primary: source,
        alternatives: Vec::new(),
        fallback_url: format!(
            "https://www.youtube-nocookie.com/embed/{video_id}?enablejsapi=1&autoplay=1&playsinline=1"
        ),
    })
}

/// The Kotlin plugin, constructed once with the Application.
struct Plugin {
    instance: GlobalRef,
    vm: JavaVM,
}

static PLUGIN: OnceLock<Result<Plugin, String>> = OnceLock::new();

fn call_plugin(video_id: &str) -> Result<String, String> {
    let plugin = PLUGIN
        .get_or_init(construct)
        .as_ref()
        .map_err(Clone::clone)?;
    let mut env = plugin
        .vm
        .attach_current_thread()
        .map_err(|error| format!("Could not reach the device resolver: {error}"))?;
    let argument = JObject::from(
        env.new_string(video_id)
            .map_err(|error| format!("Could not pass the video to the resolver: {error}"))?,
    );
    let value = env
        .call_method(
            plugin.instance.as_obj(),
            "resolveFromRust",
            "(Ljava/lang/String;)Ljava/lang/String;",
            &[JValue::Object(&argument)],
        )
        .and_then(|value| value.l())
        .map_err(|error| format!("The device resolver failed: {error}"))?;
    let text = JString::from(value);
    let answer = env
        .get_string(&text)
        .map_err(|error| format!("The device resolver answered unreadable text: {error}"))?;
    Ok(answer.into())
}

/// Constructs the plugin through the app's own class loader: from a thread
/// attached from native code, `FindClass` only sees the system's classes.
///
/// `ndk_context` holds the Application, not the Activity, and that is what the
/// plugin is built with: it only needs the application context, and a cached
/// plugin holding an Activity would outlive activity recreation (rotation,
/// dark mode). Anything that needs the Activity itself goes through
/// `manganis::android::with_activity`.
fn construct() -> Result<Plugin, String> {
    let android = ndk_context::android_context();
    let vm = unsafe { JavaVM::from_raw(android.vm().cast()) }
        .map_err(|error| format!("Could not reach the Android VM: {error}"))?;
    let mut env = vm
        .attach_current_thread_permanently()
        .map_err(|error| format!("Could not attach to the Android VM: {error}"))?;
    let application = unsafe { JObject::from_raw(android.context().cast()) };
    let loader = env
        .call_method(
            &application,
            "getClassLoader",
            "()Ljava/lang/ClassLoader;",
            &[],
        )
        .and_then(|value| value.l())
        .map_err(|error| format!("Could not find the app's class loader: {error}"))?;
    let name = JObject::from(
        env.new_string(PLUGIN_CLASS)
            .map_err(|error| format!("Could not name the resolver class: {error}"))?,
    );
    let class = env
        .call_method(
            loader,
            "loadClass",
            "(Ljava/lang/String;)Ljava/lang/Class;",
            &[JValue::Object(&name)],
        )
        .and_then(|value| value.l())
        .map_err(|error| format!("The device resolver is not in this build: {error}"))?;
    let instance = env
        .new_object(
            JClass::from(class),
            "(Landroid/content/Context;)V",
            &[JValue::Object(&application)],
        )
        .map_err(|error| format!("Could not start the device resolver: {error}"))?;
    let instance = env
        .new_global_ref(instance)
        .map_err(|error| format!("Could not keep the device resolver: {error}"))?;
    // The Application reference is the host's, not this wrapper's to release.
    std::mem::forget(application);
    drop(env);
    Ok(Plugin { instance, vm })
}
