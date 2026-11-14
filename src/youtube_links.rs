//! Opening YouTube links in Tawny: pasted into search, or handed to the app by
//! Android (a tapped link, or Share from another app).
//!
//! Videos and `/channel/UC…` links are read here, on the device, so they open
//! at once and offline. A channel named only by its handle or a vanity path
//! has to be looked up, which the server does ([`resolve_channel_url`]).

use crate::{api::resolve_channel_url, app::Route, state::AppState};
use dioxus::prelude::*;
use g3_route_transitions::animated_navigate;
use g3_ui::Color;

/// What a YouTube link points at, as far as Tawny can show it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum YoutubeLink {
    Video(String),
    Channel(String),
    /// A channel named by `@handle`, `/c/…` or `/user/…`: the whole URL, for
    /// the server to resolve.
    ChannelPage(String),
}

/// The YouTube link in `text`, if there is one.
///
/// Shared text is often a sentence around the link ("Watch this
/// https://youtu.be/…"), so every word is tried, not only the whole string.
/// Words without a scheme count too: people paste `youtu.be/…` by hand.
pub fn parse_youtube_link(text: &str) -> Option<YoutubeLink> {
    text.split_whitespace().find_map(parse_url)
}

fn parse_url(word: &str) -> Option<YoutubeLink> {
    let word = word.trim_matches(|character: char| {
        matches!(character, '<' | '>' | '(' | ')' | '"' | '\'' | ',')
    });
    let rest = without_scheme(word).unwrap_or(word);
    let (host, rest) = rest.split_once('/').unwrap_or((rest, ""));
    let host = host.to_ascii_lowercase();
    let (path, query) = match rest.split_once('?') {
        Some((path, query)) => (path, query),
        None => (rest, ""),
    };
    let path = path.split('#').next().unwrap_or_default();
    let mut segments = path.split('/').filter(|segment| !segment.is_empty());
    let first = segments.next();
    let second = segments.next();

    if host == "youtu.be" || host == "www.youtu.be" {
        return first.filter(|id| is_video_id(id)).map(video);
    }
    let host = host.strip_prefix("www.").unwrap_or(&host);
    let host = host
        .strip_prefix("m.")
        .or_else(|| host.strip_prefix("music."))
        .unwrap_or(host);
    if host != "youtube.com" && host != "youtube-nocookie.com" {
        return None;
    }
    match (first?, second) {
        ("watch", _) => query_value(query, "v")
            .filter(|id| is_video_id(id))
            .map(video),
        ("shorts" | "live" | "embed" | "v" | "e", Some(id)) if is_video_id(id) => Some(video(id)),
        ("channel", Some(id)) if is_channel_id(id) => Some(YoutubeLink::Channel(id.to_string())),
        ("c" | "user", Some(_)) => Some(channel_page(word)),
        (handle, _) if handle.len() > 1 && handle.starts_with('@') => Some(channel_page(word)),
        _ => None,
    }
}

fn video(id: &str) -> YoutubeLink {
    YoutubeLink::Video(id.to_string())
}

fn channel_page(word: &str) -> YoutubeLink {
    // rustypipe wants a lower-case scheme; a hand-typed link may have none.
    YoutubeLink::ChannelPage(format!("https://{}", without_scheme(word).unwrap_or(word)))
}

/// `word` after its `http://` or `https://`, in any case: phones upper-case
/// the first letter of a pasted line.
fn without_scheme(word: &str) -> Option<&str> {
    ["https://", "http://"].into_iter().find_map(|scheme| {
        word.get(..scheme.len())
            .filter(|prefix| prefix.eq_ignore_ascii_case(scheme))
            .map(|_| &word[scheme.len()..])
    })
}

fn query_value<'a>(query: &'a str, key: &str) -> Option<&'a str> {
    query
        .split(['&', '#'])
        .filter_map(|pair| pair.split_once('='))
        .find(|(name, _)| *name == key)
        .map(|(_, value)| value)
}

/// Video ids are 11 URL-safe base64 characters. Checking the shape keeps a
/// path like `/watch/later` from opening as a video.
fn is_video_id(value: &str) -> bool {
    value.len() == 11 && value.chars().all(is_id_character)
}

fn is_channel_id(value: &str) -> bool {
    value.len() == 24 && value.starts_with("UC") && value.chars().all(is_id_character)
}

fn is_id_character(character: char) -> bool {
    character.is_ascii_alphanumeric() || matches!(character, '-' | '_')
}

/// Go to what `link` points at. A channel page is looked up first; if that
/// fails the reader is told, and stays where they are.
pub fn open_youtube_link(app_state: AppState, link: YoutubeLink) {
    spawn(async move {
        let route = match link {
            YoutubeLink::Video(id) => Route::VideoDetail { id },
            YoutubeLink::Channel(id) => Route::ChannelDetail { id },
            YoutubeLink::ChannelPage(url) => match resolve_channel_url(url).await {
                Ok(id) => Route::ChannelDetail { id },
                Err(_) => {
                    app_state.show_toast("Could not find that channel", Color::Warning);
                    return;
                }
            },
        };
        animated_navigate(route).await;
    });
}

/// Android hands Tawny the YouTube links it is asked to open: a tapped link,
/// once Tawny is allowed to open youtube.com, or a video shared to it from
/// another app. Mounted inside the router so it can navigate.
#[component]
pub fn YoutubeLinkReceiver() -> Element {
    #[cfg(target_os = "android")]
    {
        let app_state = use_context::<AppState>();
        let mut plugins = use_context::<g3_native_plugins::NativePlugins>();
        use_future(move || async move {
            let _ = plugins.deep_links.write().prepare();
            // Polled, because the plugin queues rather than calls back: a link
            // can arrive before anything here could receive it. One JNI call
            // a second is nothing beside the WebView.
            loop {
                while let Ok(Some(text)) = plugins.deep_links.write().take_link() {
                    match parse_youtube_link(&text) {
                        Some(link) => open_youtube_link(app_state, link),
                        None => app_state.show_toast(
                            "Tawny can open YouTube videos and channels",
                            Color::Warning,
                        ),
                    }
                }
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
        });
    }

    rsx! {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn video_id(text: &str) -> Option<String> {
        match parse_youtube_link(text)? {
            YoutubeLink::Video(id) => Some(id),
            _ => None,
        }
    }

    #[test]
    fn reads_every_video_url_shape() {
        for url in [
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
            "https://youtube.com/watch?feature=share&v=dQw4w9WgXcQ&t=42",
            "http://m.youtube.com/watch?v=dQw4w9WgXcQ#comments",
            "https://music.youtube.com/watch?v=dQw4w9WgXcQ&list=RDAMVM",
            "https://youtu.be/dQw4w9WgXcQ?si=abcdef",
            "youtu.be/dQw4w9WgXcQ",
            "www.youtube.com/shorts/dQw4w9WgXcQ",
            "https://www.youtube.com/live/dQw4w9WgXcQ?feature=shared",
            "https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ",
            "HTTPS://WWW.YOUTUBE.COM/watch?v=dQw4w9WgXcQ",
        ] {
            assert_eq!(video_id(url).as_deref(), Some("dQw4w9WgXcQ"), "{url}");
        }
    }

    #[test]
    fn finds_the_link_inside_shared_text() {
        assert_eq!(
            video_id("Watch this! (https://youtu.be/dQw4w9WgXcQ) so good").as_deref(),
            Some("dQw4w9WgXcQ"),
        );
    }

    #[test]
    fn reads_channels() {
        assert_eq!(
            parse_youtube_link("https://www.youtube.com/channel/UC2DjFE7Xf11URZqWBigcVOQ/videos"),
            Some(YoutubeLink::Channel("UC2DjFE7Xf11URZqWBigcVOQ".into())),
        );
        assert_eq!(
            parse_youtube_link("youtube.com/@somebody/videos"),
            Some(YoutubeLink::ChannelPage(
                "https://youtube.com/@somebody/videos".into()
            )),
        );
        assert_eq!(
            parse_youtube_link("https://www.youtube.com/c/Vanity"),
            Some(YoutubeLink::ChannelPage(
                "https://www.youtube.com/c/Vanity".into()
            )),
        );
    }

    #[test]
    fn leaves_searches_and_other_sites_alone() {
        for text in [
            "youtube",
            "how to bake bread",
            "youtube.com",
            "https://www.youtube.com/watch?v=short",
            "https://www.youtube.com/watch/later",
            "https://www.youtube.com/playlist?list=PL123",
            "https://notyoutube.com/watch?v=dQw4w9WgXcQ",
            "https://vimeo.com/dQw4w9WgXcQ",
            "@",
        ] {
            assert_eq!(parse_youtube_link(text), None, "{text}");
        }
    }
}
