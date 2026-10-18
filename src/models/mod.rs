use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use surrealdb_types::SurrealValue;

mod channel;
mod content;
mod filters;
mod library;
mod playlist;
mod settings;
mod video;
// The seed catalog and demo viewer: only the server seeds with them.
mod account;
#[cfg(feature = "server")]
mod demo;
mod playback;
mod sponsor;

pub use account::*;
pub use channel::*;
pub use content::*;
#[cfg(feature = "server")]
pub use demo::*;
pub use filters::*;
pub use library::*;
pub use playback::*;
pub use playlist::*;
pub use settings::*;
pub use sponsor::*;
pub use video::*;

#[cfg(test)]
mod tests {
    use super::*;

    fn caption(language_code: &str, auto_generated: bool) -> CaptionTrack {
        CaptionTrack {
            label: language_code.into(),
            language_code: language_code.into(),
            mime_type: "text/vtt".into(),
            url: String::new(),
            auto_generated,
        }
    }

    #[test]
    fn captions_open_in_the_preferred_language_not_the_first_listed() {
        // YouTube lists tracks alphabetically, so Arabic leads.
        let tracks = [
            caption("ar", false),
            caption("en", true),
            caption("en-GB", false),
            caption("fr", false),
        ];
        // A written track beats the automatic one, a region still counts.
        assert_eq!(preferred_caption_index(&tracks, "en"), Some(2));
        assert_eq!(preferred_caption_index(&tracks, "EN-us"), Some(2));
        assert_eq!(preferred_caption_index(&tracks, "fr"), Some(3));
        // Only the automatic track speaks it: better than another language.
        assert_eq!(preferred_caption_index(&tracks[..2], "en"), Some(1));
        // Nothing in the language: the first, as before.
        assert_eq!(preferred_caption_index(&tracks, "ja"), Some(0));
        assert_eq!(preferred_caption_index(&[], "en"), None);
    }

    #[test]
    fn settings_saved_before_the_caption_language_existed_prefer_english() {
        let mut saved = serde_json::to_value(AppSettings::default()).expect("settings serialize");
        saved
            .as_object_mut()
            .expect("settings are an object")
            .remove("preferred_caption_language");
        let settings: AppSettings =
            serde_json::from_value(saved).expect("older settings still load");
        assert_eq!(settings.preferred_caption_language, "en");
    }

    fn video(id: &str, title: &str, published_at: &str, duration_seconds: u64) -> Video {
        Video {
            id: id.into(),
            title: title.into(),
            channel_id: "channel".into(),
            channel_name: "Channel".into(),
            thumbnail_url: String::new(),
            published_at: published_at.into(),
            duration_seconds,
            view_count: String::new(),
            progress_seconds: 0,
            watched: false,
            is_live: false,
            is_short: false,
            audio_only: false,
            channel_avatar_url: None,
        }
    }

    fn ids(videos: &[Video]) -> Vec<&str> {
        videos.iter().map(|video| video.id.as_str()).collect()
    }

    fn sample() -> Vec<Video> {
        vec![
            video("b", "Beta", "2024-03-01T00:00:00Z", 600),
            video("a", "alpha", "2024-01-01T00:00:00Z", 90),
            video("c", "Gamma", "2024-02-01T00:00:00Z", 3600),
        ]
    }

    #[test]
    fn added_keeps_the_playlists_own_order() {
        let mut videos = sample();
        PlaylistSort::Added.apply(&mut videos, false);
        assert_eq!(ids(&videos), ["b", "a", "c"]);
    }

    #[test]
    fn added_reversed_walks_the_playlist_backwards() {
        let mut videos = sample();
        PlaylistSort::Added.apply(&mut videos, true);
        assert_eq!(ids(&videos), ["c", "a", "b"]);
    }

    #[test]
    fn published_sorts_newest_first_when_descending() {
        let mut videos = sample();
        PlaylistSort::Published.apply(&mut videos, true);
        assert_eq!(ids(&videos), ["b", "c", "a"]);
        let mut videos = sample();
        PlaylistSort::Published.apply(&mut videos, false);
        assert_eq!(ids(&videos), ["a", "c", "b"]);
    }

    /// Opening a video stored the watch page's "Feb 1, 2024", which used to
    /// read as 1970 and sink it to the end of Newest - leaving a run started on
    /// it with nothing after it and everything before it.
    #[test]
    fn a_written_out_date_keeps_its_place_in_newest() {
        let mut videos = sample();
        videos[2].published_at = "Feb 1, 2024".into();
        PlaylistSort::Published.apply(&mut videos, true);
        assert_eq!(ids(&videos), ["b", "c", "a"]);
        let view = PlaylistView {
            sort: PlaylistSort::Published,
            descending: true,
            ..PlaylistView::default()
        };
        assert_eq!(view.next_after(&videos, "c").as_deref(), Some("a"));
    }

    #[test]
    fn text_dates_read_through_youtube_prefixes() {
        let expected = time::Date::from_calendar_date(2013, time::Month::August, 2).ok();
        assert_eq!(text_date("Aug 2, 2013"), expected);
        assert_eq!(text_date("Premiered Aug 2, 2013"), expected);
        assert_eq!(text_date("Streamed live on August 2, 2013"), expected);
        assert_eq!(text_date("2 months ago"), None);
        assert_eq!(text_date("From YouTube"), None);
    }

    /// The feed's timestamp outranks the watch page's day, which outranks a
    /// related shelf's "3 years ago"; a finer date is never replaced by a coarser one.
    #[test]
    fn merging_details_keeps_the_finer_publish_date() {
        let feed = video("a", "A", "2024-02-01T15:30:00Z", 0);
        let mut detail = video("a", "A", "2024-02-01 0:00:00.0 +00:00:00", 0);
        detail.keep_finer_publish_date(&feed);
        assert_eq!(detail.published_at, "2024-02-01T15:30:00Z");

        let mut related = video("a", "A", "3 years ago", 0);
        related.keep_finer_publish_date(&video("a", "A", "Feb 1, 2024", 0));
        assert_eq!(related.published_at, "Feb 1, 2024");

        let mut fresh = video("a", "A", "2024-02-01T15:30:00Z", 0);
        fresh.keep_finer_publish_date(&video("a", "A", "3 years ago", 0));
        assert_eq!(fresh.published_at, "2024-02-01T15:30:00Z");
    }

    /// "3 months ago" is stored as the moment it meant, so it still reads
    /// right months later, and still loses to any real date.
    #[test]
    fn a_relative_date_is_kept_as_an_approximate_moment() {
        let mut shelf = video("a", "A", "3 months ago", 0);
        shelf.settle_relative_date();
        assert!(
            shelf.published_at.starts_with('~'),
            "{}",
            shelf.published_at
        );
        assert_eq!(shelf.published_label(), "3 months ago");

        let year_old = Video {
            published_at: Video::approximate_published_at(shelf.published_epoch() - 365 * 86_400),
            ..shelf.clone()
        };
        assert_eq!(year_old.published_label(), "1 year ago");

        let mut dated = video("a", "A", "Feb 1, 2024", 0);
        dated.keep_finer_publish_date(&shelf);
        assert_eq!(dated.published_at, "Feb 1, 2024");
        shelf.keep_finer_publish_date(&dated);
        assert_eq!(shelf.published_at, "Feb 1, 2024");

        // Only "... ago" is settled: the rest is either exact or not a guess
        // worth keeping.
        for text in ["Streaming now", "Yesterday", "Feb 1, 2024", ""] {
            let mut other = video("a", "A", text, 0);
            other.settle_relative_date();
            assert_eq!(other.published_at, text);
        }
    }

    #[test]
    fn duration_sorts_longest_first_when_descending() {
        let mut videos = sample();
        PlaylistSort::Duration.apply(&mut videos, true);
        assert_eq!(ids(&videos), ["c", "b", "a"]);
    }

    /// Case is not order: "alpha" belongs before "Beta", not after "Gamma".
    #[test]
    fn title_ignores_case() {
        let mut videos = sample();
        PlaylistSort::Title.apply(&mut videos, false);
        assert_eq!(ids(&videos), ["a", "b", "c"]);
    }

    /// Every chip is reachable in both directions, and the two directions never
    /// read the same - the label is the only place the direction is shown.
    #[test]
    fn every_sort_labels_both_directions_distinctly() {
        for sort in PlaylistSort::ALL {
            assert_ne!(sort.label(true), sort.label(false), "{sort:?}");
        }
    }

    fn run_order(view: &PlaylistView, videos: Vec<Video>) -> Vec<Video> {
        let mut videos = videos;
        view.arrange(&mut videos, &AppSettings::default());
        videos
    }

    #[test]
    fn a_run_walks_the_arrangement_on_screen() {
        let view = PlaylistView {
            sort: PlaylistSort::Title,
            ..PlaylistView::default()
        };
        let order = run_order(&view, sample());
        assert_eq!(ids(&order), ["a", "b", "c"]);
        assert_eq!(view.next_after(&order, "a").as_deref(), Some("b"));
        assert_eq!(view.previous_before(&order, "b").as_deref(), Some("a"));
        assert_eq!(view.next_after(&order, "c"), None, "the last entry ends it");
        assert_eq!(
            view.previous_before(&order, "a"),
            None,
            "nothing before the first"
        );
    }

    /// The regression the split between `arrange` and `shows` exists to prevent.
    /// Finishing a video is what marks it watched, so an order that had already
    /// dropped watched videos could not say where the run was - and every advance
    /// would hand back the top of the playlist forever.
    #[test]
    fn finishing_a_video_does_not_send_an_unwatched_run_back_to_the_top() {
        let view = PlaylistView {
            only_unwatched: true,
            ..PlaylistView::default()
        };
        let mut videos = sample();
        videos[0].watched = true;
        let order = run_order(&view, videos);
        assert_eq!(ids(&order), ["b", "a", "c"], "playlist order, watched kept");
        assert_eq!(
            view.next_after(&order, "b").as_deref(),
            Some("a"),
            "advances past the video that just finished"
        );
    }

    #[test]
    fn a_run_skips_over_what_the_filters_hide() {
        let view = PlaylistView {
            only_unwatched: true,
            ..PlaylistView::default()
        };
        let mut videos = sample();
        videos[1].watched = true;
        let order = run_order(&view, videos);
        assert_eq!(view.next_after(&order, "b").as_deref(), Some("c"));
        assert_eq!(view.previous_before(&order, "c").as_deref(), Some("b"));
    }

    #[test]
    fn a_run_starts_at_the_top_for_a_video_it_does_not_hold() {
        let view = PlaylistView::default();
        let order = run_order(&view, sample());
        assert_eq!(view.next_after(&order, "unrelated").as_deref(), Some("b"));
        assert_eq!(view.previous_before(&order, "unrelated"), None);
    }

    #[test]
    fn a_kind_filter_narrows_what_the_run_can_reach() {
        let mut videos = sample();
        videos[1].is_short = true;
        let shorts = PlaylistView {
            kind: PlaylistKind::Shorts,
            ..PlaylistView::default()
        };
        assert_eq!(ids(&run_order(&shorts, videos.clone())), ["a"]);
        let long_form = PlaylistView {
            kind: PlaylistKind::Videos,
            ..PlaylistView::default()
        };
        assert_eq!(ids(&run_order(&long_form, videos)), ["b", "c"]);
    }

    #[test]
    fn a_default_view_hides_nothing() {
        assert!(!PlaylistView::default().is_filtered());
    }

    #[test]
    fn a_queue_marker_is_told_apart_from_a_video_id() {
        let entry = playlist_queue_entry("watch-later");
        assert_eq!(queued_playlist_id(&entry), Some("watch-later"));
        assert_eq!(
            queued_playlist_id("dQw4w9WgXcQ"),
            None,
            "a video id is never mistaken for a run"
        );
    }
}
