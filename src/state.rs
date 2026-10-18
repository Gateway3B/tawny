//! App-wide state.
//!
//! The server holds this account's data; the client holds what the screens
//! last saw of it, through `g3-cache`. [`AppState::viewer`] is the account's
//! own small state (follows, playlists, groups, queue, history), read by
//! nearly every screen, and each screen reads the videos it shows itself.
//!
//! A change shows at once: it is applied to the cached reads it touches, sent
//! to the server as a "set to" mutation, and then the reads it changed are
//! refetched (see `data_change`), which also undoes the guess if the server
//! refused it.

use std::collections::HashMap;
use std::future::Future;

use crate::subscriptions_io::{ImportSummary, ParsedImport};
use crate::{
    api::{self, get_feed_page, get_playlist, get_playlist_previews, get_videos, get_viewer},
    auth::use_session_provider,
    cache::use_persistent_signal,
    data_change::{DataChange, invalidate},
    models::{
        AppSettings, AudioTrackOption, CaptionTrack, Channel, FeedFilter, HISTORY_LIMIT,
        HistoryEntry, PlaylistContents, PlaylistView, SponsorSegment, SubscriptionChange,
        SubscriptionContent, SubscriptionGroup, Video, VideoChapter, VideoList, VideoPreviewFrames,
        VideoProgress, Viewer, WATCH_LATER_PLAYLIST_ID, playlist_queue_entry,
        preferred_caption_index, queued_playlist_id,
    },
};
use dioxus::prelude::*;
use g3_cache::{Cached, update_all_cached, update_cached, use_cached};
use g3_ui::{Color, ToastOptions, Toaster};

/// What saving a video to a playlist did. A swipe that lands on a video the
/// playlist already holds has changed nothing, and saying so in the same
/// green as a save reads as though it saved again.
pub enum PlaylistSave {
    Saved(String),
    AlreadyThere(String),
}

/// A snapshot of the playlist run, resolved the same way autoplay resolves it.
pub struct PlaylistRunStatus {
    pub playlist_id: String,
    pub name: String,
    /// The arrangement the run is walking right now. A run reads the playlist's
    /// live view each time it advances, so this is also what it will use next.
    pub view: PlaylistView,
    /// Entries in the arranged order, watched filter aside.
    pub total: usize,
    /// 1-based position of the playing video, when the run contains it.
    pub position: Option<usize>,
    pub current: Option<Video>,
    /// Entries still to play after the current one.
    pub remaining: usize,
    pub up_next: Option<Video>,
}

#[derive(Clone, Copy)]
pub struct AppState {
    /// This account's own state, as last fetched. See [`Self::with_viewer`].
    pub viewer: Cached<Viewer>,
    /// The playlist the queue is running, with its videos, so autoplay can
    /// decide what is next without a round trip. Shares its cache entry with
    /// that playlist's page.
    pub run_playlist: Cached<Option<PlaylistContents>>,
    pub settings: Signal<AppSettings>,
    /// Where playback has reached, per video, since it was last sent. Ticks
    /// land here; [`Self::flush_progress`] sends them.
    pending_progress: Signal<HashMap<String, VideoProgress>>,
    /// Positions already sent but not yet shown on the cached lists. Rewriting
    /// every cached feed page and refetching the playlist previews on each
    /// five-second tick was most of the work of watching a video, so the
    /// lists catch up when playback pauses, hides or ends instead.
    unshown_progress: Signal<HashMap<String, VideoProgress>>,
    /// g3-ui's toast queue. It belongs to the `AppWrapper`, which sits below
    /// this state, so a component inside the wrapper hands it over once it
    /// mounts. Until then a notice has nowhere to show and is dropped.
    pub toaster: Signal<Option<Toaster>>,
    pub playlist_picker_video: Signal<Option<Video>>,
    pub playlist_picker_open: Signal<bool>,
    pub video_actions_video: Signal<Option<Video>>,
    /// The playlist the actions sheet was opened from, so it can offer to
    /// take the video out of it.
    pub video_actions_playlist: Signal<Option<String>>,
    pub video_actions_open: Signal<bool>,
    pub share_video: Signal<Option<Video>>,
    pub share_open: Signal<bool>,
    pub share_with_timestamp: Signal<bool>,
    pub share_timestamp_seconds: Signal<u64>,
    pub active_video: Signal<Option<Video>>,
    pub active_captions: Signal<Vec<CaptionTrack>>,
    pub selected_caption: Signal<Option<usize>>,
    pub captions_enabled: Signal<bool>,
    pub active_chapters: Signal<Vec<VideoChapter>>,
    /// Fetched separately from the rest of the metadata: which categories to ask
    /// for depends on viewer settings, so it cannot ride along with the details.
    pub active_sponsor_segments: Signal<Vec<SponsorSegment>>,
    /// Audio languages the transport can switch between, and which one is live.
    /// Empty for the ordinary single-language upload, which is why the chip that
    /// reads these only appears when there is a choice to make.
    pub active_audio_tracks: Signal<Vec<AudioTrackOption>>,
    pub selected_audio_track: Signal<Option<String>>,
    pub active_preview_frames: Signal<Option<VideoPreviewFrames>>,
    pub syncing: Signal<bool>,
    /// Counts taps on the Feed tab while the feed is already open. The feed
    /// answers each one; a count rather than a flag so two taps are two.
    pub feed_reselected: Signal<u64>,
    pub chapters_sheet_open: Signal<bool>,
    /// Videos this sitting has advanced away from, most recent last.
    ///
    /// Deliberately not on the server: it is what "previous" means during one
    /// run of the queue, not something worth carrying to another device or
    /// remembering tomorrow. History cannot answer it - playing a video moves it
    /// to the front of history, so stepping back through history immediately
    /// starts bouncing between two videos.
    pub run_back_stack: Signal<Vec<String>>,
    pub feed_filter: Signal<FeedFilter>,
    pub explore_filter: Signal<crate::models::ExploreFilter>,
    /// The search page's query and server results, kept here so leaving the
    /// page and coming back finds the same search. A reload clears them.
    pub explore_search: Signal<String>,
    pub explore_results: Signal<Option<crate::models::SearchResults>>,
    pub channel_tab: Signal<FeedFilter>,
    /// What the feed was showing, kept so leaving it and coming back finds the
    /// same filters and as many pages as were loaded, which is what lets a
    /// returning scroll position land where it was left.
    pub feed_group: Signal<crate::models::FeedGroup>,
    pub feed_duration: Signal<Option<crate::models::DurationFilter>>,
    pub feed_pages: Signal<usize>,
}

impl AppState {
    /// Read this account's state without copying it. Subscribes, so the
    /// caller rerenders when it changes. Before the first answer arrives it
    /// reads as empty.
    pub fn with_viewer<T>(self, read: impl FnOnce(&Viewer) -> T) -> T {
        match &*self.viewer.read() {
            Some(Ok(viewer)) => read(viewer),
            _ => read(&Viewer::default()),
        }
    }

    /// Like [`Self::with_viewer`], without subscribing: for event handlers.
    fn peek_viewer<T>(self, read: impl FnOnce(&Viewer) -> T) -> T {
        match &*self.viewer.peek() {
            Some(Ok(viewer)) => read(viewer),
            _ => read(&Viewer::default()),
        }
    }

    /// Show a change to this account's state at once. The mutation that
    /// follows, and its invalidation, reconcile it with the server.
    fn edit_viewer(self, edit: impl FnOnce(&mut Viewer)) {
        update_cached(get_viewer, (), edit);
    }

    /// Apply `update` to this video wherever a cached list holds it: feed
    /// pages, playlists, the queue and history, and the one playing.
    fn apply_to_videos(mut self, video_id: &str, update: impl Fn(&mut Video)) {
        update_all_cached(get_feed_page, |page| page.update_video(video_id, &update));
        update_all_cached(get_videos, |videos| videos.update_video(video_id, &update));
        update_all_cached(get_playlist, |contents| {
            if let Some(contents) = contents {
                contents.update_video(video_id, &update);
            }
        });
        // Previews carry counts, not videos: refetched rather than edited.
        g3_cache::invalidate_cached(get_playlist_previews);
        let active = self.active_video.peek().clone();
        if let Some(mut active) = active
            && active.id == video_id
        {
            update(&mut active);
            self.active_video.set(Some(active));
        }
    }

    /// Take newly resolved catalog facts (lengths, live and Shorts state)
    /// into every cached list, keeping this viewer's progress.
    ///
    /// Feed pages are refetched too: a video with a length can now enter a
    /// length filter it was left out of.
    pub fn merge_video_metadata(self, fresh: Vec<Video>) {
        if fresh.is_empty() {
            return;
        }
        update_all_cached(get_videos, |videos| videos.merge_metadata(&fresh));
        update_all_cached(get_playlist, |contents| {
            if let Some(contents) = contents {
                contents.merge_metadata(&fresh);
            }
        });
        g3_cache::invalidate_cached(get_feed_page);
    }

    /// Send a mutation, then refetch what it changed, whether or not it
    /// succeeded. A failure is said aloud: the guess shown in the meantime is
    /// about to be undone by the refetch.
    fn send<T: 'static>(
        self,
        change: DataChange,
        mutation: impl Future<Output = Result<T>> + 'static,
    ) {
        spawn(async move {
            let result = mutation.await;
            invalidate(change);
            if let Err(error) = result {
                self.show_toast(
                    format!(
                        "Could not save that: {}",
                        crate::auth::readable(&error.to_string())
                    ),
                    Color::Danger,
                );
            }
        });
    }

    pub fn settings(self) -> AppSettings {
        (self.settings)()
    }

    pub fn playlist_picker_video(self) -> Option<Video> {
        (self.playlist_picker_video)()
    }

    pub fn syncing(self) -> bool {
        (self.syncing)()
    }

    pub fn active_video(self) -> Option<Video> {
        (self.active_video)()
    }

    pub fn show_toast(self, message: impl Into<String>, color: Color) {
        if let Some(toaster) = *self.toaster.peek() {
            // Most notices confirm an action that can be repeated at once -
            // a swipe, a toggle - so the latest replaces rather than queues.
            toaster.show(ToastOptions::new(message).color(color).replace());
        }
    }

    pub fn play(mut self, video: Video) {
        if self
            .active_video()
            .as_ref()
            .is_none_or(|active| active.id != video.id)
        {
            // The outgoing video keeps whatever position its last tick recorded.
            // This is what stops the server from being up to half a minute
            // behind when a video is swapped.
            self.flush_progress();
            self.active_captions.set(Vec::new());
            self.selected_caption.set(None);
            self.captions_enabled.set(false);
            self.active_chapters.set(Vec::new());
            self.active_sponsor_segments.set(Vec::new());
            self.active_preview_frames.set(None);
            self.active_audio_tracks.set(Vec::new());
            self.selected_audio_track.set(None);
        }
        self.active_video.set(Some(video));
    }

    /// Fill in what is known about the video that is playing.
    ///
    /// Only ever adds. The same player is handed two descriptions of a video:
    /// the card it was opened from, which carries no chapters, captions or
    /// preview frames, and the one the details request returns, which carries
    /// all three. They arrive in either order and more than once, so applying
    /// an empty field on top of a full one would strip the timeline of its
    /// divisions and its segment colours part way through a video. Changing
    /// video is what clears this - see [`Self::play`] and
    /// [`Self::stop_playback`].
    pub fn set_player_metadata(
        mut self,
        captions: Vec<CaptionTrack>,
        chapters: Vec<VideoChapter>,
        preview_frames: Option<VideoPreviewFrames>,
    ) {
        if !captions.is_empty() {
            if (self.selected_caption)().is_none() {
                let language = self.settings.peek().preferred_caption_language.clone();
                self.selected_caption
                    .set(preferred_caption_index(&captions, &language));
            }
            self.active_captions.set(captions);
        }
        if !chapters.is_empty() {
            self.active_chapters.set(chapters);
        }
        if preview_frames.is_some() {
            self.active_preview_frames.set(preview_frames);
        }
    }

    /// Choose the playing video's captions by the viewer's language again,
    /// after the setting changes, so the choice shows without a new video.
    pub fn select_preferred_caption(mut self) {
        let language = self.settings.peek().preferred_caption_language.clone();
        let index = preferred_caption_index(&self.active_captions.peek(), &language);
        if index.is_some() {
            self.selected_caption.set(index);
        }
    }

    pub fn stop_playback(mut self) {
        self.flush_progress();
        self.active_video.set(None);
        self.active_captions.set(Vec::new());
        self.selected_caption.set(None);
        self.captions_enabled.set(false);
        self.active_chapters.set(Vec::new());
        self.active_sponsor_segments.set(Vec::new());
        self.active_preview_frames.set(None);
        self.active_audio_tracks.set(Vec::new());
        self.selected_audio_track.set(None);
    }

    /// Discard account state without saving playback progress to a deleted
    /// account. The persistent read cache is cleared by the caller.
    pub fn forget_account(mut self) {
        self.pending_progress.write().clear();
        self.unshown_progress.write().clear();
        self.stop_playback();
        self.run_back_stack.write().clear();
        self.explore_results.set(None);
        self.playlist_picker_open.set(false);
        self.playlist_picker_video.set(None);
        self.video_actions_open.set(false);
        self.video_actions_video.set(None);
        self.video_actions_playlist.set(None);
        self.share_open.set(false);
        self.share_video.set(None);
    }

    /// Remember where playback has reached.
    ///
    /// Called on a timer while a video plays, so it only updates the playing
    /// copy and the pending map; [`Self::flush_progress`] sends it, at the
    /// points where losing the position would actually matter. A whole second
    /// of change is the smallest step worth a re-render.
    pub fn record_progress(mut self, video_id: &str, seconds: u64) -> bool {
        let Some(mut active) = self.active_video.peek().clone() else {
            return false;
        };
        if active.id != video_id || active.progress_seconds == seconds {
            return false;
        }
        active.progress_seconds = seconds;
        // Treated as watched once it is effectively over, which is what the
        // card's progress bar and the "hide watched" filter both key on.
        if active.is_effectively_over(seconds) {
            active.watched = true;
        }
        self.pending_progress.write().insert(
            active.id.clone(),
            VideoProgress {
                video_id: active.id.clone(),
                watched: active.watched,
                progress_seconds: active.progress_seconds,
                audio_only: active.audio_only,
            },
        );
        self.active_video.set(Some(active));
        true
    }

    /// Send the remembered positions to the server, and show them on every
    /// cached list holding those videos.
    pub fn flush_progress(self) {
        self.sync_progress(true);
    }

    /// Send the remembered positions to the server. With `show_in_lists`, the
    /// cached lists holding those videos are updated too, along with any
    /// positions sent earlier that they have not seen; without it, the
    /// periodic tick costs one request and no cache writes.
    pub fn sync_progress(mut self, show_in_lists: bool) {
        let pending = std::mem::take(&mut *self.pending_progress.write());
        let mut to_show = HashMap::new();
        if show_in_lists {
            to_show = std::mem::take(&mut *self.unshown_progress.write());
            to_show.extend(pending.clone());
        } else {
            self.unshown_progress.write().extend(pending.clone());
        }
        for entry in to_show.into_values() {
            self.apply_to_videos(&entry.video_id.clone(), move |video| {
                video.progress_seconds = entry.progress_seconds;
                video.watched = entry.watched;
            });
        }
        if pending.is_empty() {
            return;
        }
        let progress = pending.into_values().collect::<Vec<_>>();
        // No invalidation: the lists already show what was sent.
        spawn(async move {
            if api::save_progress(progress).await.is_err() {
                self.show_toast("Could not save your place in that video", Color::Warning);
            }
        });
    }

    fn save_one_progress(self, video: &Video) {
        let progress = vec![VideoProgress {
            video_id: video.id.clone(),
            watched: video.watched,
            progress_seconds: video.progress_seconds,
            audio_only: video.audio_only,
        }];
        spawn(async move {
            if api::save_progress(progress).await.is_err() {
                self.show_toast("Could not save that", Color::Warning);
            }
        });
    }

    /// Play this video without its video stream, and remember that choice for
    /// this video specifically.
    pub fn set_audio_only(self, video: &Video, audio_only: bool) -> bool {
        if video.audio_only == audio_only {
            return false;
        }
        self.apply_to_videos(&video.id, move |video| video.audio_only = audio_only);
        self.save_one_progress(&Video {
            audio_only,
            ..video.clone()
        });
        true
    }

    pub fn mark_watched(self, video: &Video, watched: bool) {
        let progress_seconds = if watched { video.duration_seconds } else { 0 };
        self.apply_to_videos(&video.id, move |video| {
            video.watched = watched;
            video.progress_seconds = progress_seconds;
        });
        self.save_one_progress(&Video {
            watched,
            progress_seconds,
            ..video.clone()
        });
    }

    // -----------------------------------------------------------------------
    // Playlists
    // -----------------------------------------------------------------------

    pub fn add_to_playlist(self, video: &Video, playlist_id: &str) -> Option<PlaylistSave> {
        let playlist = self.peek_viewer(|viewer| viewer.playlist(playlist_id).cloned())?;
        if playlist.video_ids.iter().any(|id| id == &video.id) {
            return Some(PlaylistSave::AlreadyThere(playlist.name));
        }
        let video_id = video.id.clone();
        self.edit_viewer(|viewer| {
            if let Some(playlist) = viewer.playlists.iter_mut().find(|p| p.id == playlist_id) {
                playlist.video_ids.insert(0, video_id.clone());
            }
        });
        let added = video.clone();
        update_cached(get_playlist, (playlist_id.to_string(),), |contents| {
            if let Some(contents) = contents {
                contents.playlist.video_ids.insert(0, added.id.clone());
                contents.videos.insert(0, added);
            }
        });
        self.send(
            DataChange::Playlists,
            api::set_in_playlist(playlist_id.to_string(), video.id.clone(), true),
        );
        Some(PlaylistSave::Saved(playlist.name))
    }

    pub fn remove_from_playlist(self, video_id: &str, playlist_id: &str) -> bool {
        let present = self.peek_viewer(|viewer| {
            viewer
                .playlist(playlist_id)
                .is_some_and(|playlist| playlist.video_ids.iter().any(|id| id == video_id))
        });
        if !present {
            return false;
        }
        self.edit_viewer(|viewer| {
            if let Some(playlist) = viewer.playlists.iter_mut().find(|p| p.id == playlist_id) {
                playlist.video_ids.retain(|id| id != video_id);
            }
        });
        update_cached(get_playlist, (playlist_id.to_string(),), |contents| {
            if let Some(contents) = contents {
                contents.playlist.video_ids.retain(|id| id != video_id);
                contents.videos.retain(|video| video.id != video_id);
            }
        });
        self.send(
            DataChange::Playlists,
            api::set_in_playlist(playlist_id.to_string(), video_id.to_string(), false),
        );
        true
    }

    /// Create a playlist, returning its id. The id is made here so the
    /// playlist can be used before the server has answered.
    pub fn create_playlist(self, name: String) -> String {
        let id = format!("pl-{}", random_suffix());
        let playlist = crate::models::Playlist {
            id: id.clone(),
            name: name.clone(),
            video_ids: Vec::new(),
        };
        self.edit_viewer(|viewer| viewer.playlists.push(playlist));
        self.send(DataChange::Playlists, api::save_playlist(id.clone(), name));
        id
    }

    /// Create a playlist that already holds `video`, returning its id.
    ///
    /// One edit and one request chain rather than [`Self::create_playlist`]
    /// followed by [`Self::add_to_playlist`]: that second call looks the
    /// playlist up in the cached viewer, which a created playlist reaches only
    /// after the cache has settled, so the video was silently left out.
    pub fn create_playlist_with(self, name: String, video: &Video) -> String {
        let id = format!("pl-{}", random_suffix());
        let playlist = crate::models::Playlist {
            id: id.clone(),
            name: name.clone(),
            video_ids: vec![video.id.clone()],
        };
        self.edit_viewer(|viewer| viewer.playlists.push(playlist));
        let (created, video_id) = (id.clone(), video.id.clone());
        self.send(DataChange::Playlists, async move {
            api::save_playlist(created.clone(), name).await?;
            api::set_in_playlist(created, video_id, true).await?;
            Ok(())
        });
        id
    }

    /// Rename a playlist, everywhere it is shown at once.
    pub fn rename_playlist(self, playlist_id: &str, name: String) {
        let name = name.trim().to_string();
        if name.is_empty() {
            return;
        }
        self.edit_viewer(|viewer| {
            if let Some(playlist) = viewer.playlists.iter_mut().find(|p| p.id == playlist_id) {
                playlist.name = name.clone();
            }
        });
        update_cached(get_playlist, (playlist_id.to_string(),), |contents| {
            if let Some(contents) = contents {
                contents.playlist.name = name.clone();
            }
        });
        update_cached(get_playlist_previews, (), |previews| {
            if let Some(preview) = previews.iter_mut().find(|p| p.id == playlist_id) {
                preview.name = name.clone();
            }
        });
        self.send(
            DataChange::Playlists,
            api::save_playlist(playlist_id.to_string(), name),
        );
    }

    /// Put the playlists in the order given. Anything not named keeps its
    /// place after the ones that are.
    pub fn reorder_playlists(self, order: Vec<String>) {
        let rank = |id: &str| order.iter().position(|ordered| ordered == id);
        self.edit_viewer(|viewer| {
            viewer
                .playlists
                .sort_by_key(|p| rank(&p.id).unwrap_or(usize::MAX))
        });
        update_cached(get_playlist_previews, (), |previews| {
            previews.sort_by_key(|p| rank(&p.id).unwrap_or(usize::MAX));
        });
        self.send(DataChange::Playlists, api::reorder_playlists(order.clone()));
    }

    pub fn delete_playlist(mut self, playlist_id: &str) -> Option<String> {
        let name =
            self.peek_viewer(|viewer| viewer.playlist(playlist_id).map(|p| p.name.clone()))?;
        self.edit_viewer(|viewer| {
            viewer
                .playlists
                .retain(|playlist| playlist.id != playlist_id);
            viewer
                .queue
                .retain(|entry| queued_playlist_id(entry) != Some(playlist_id));
        });
        let mut settings = self.settings.write();
        if settings.swipe_right_playlist_id == playlist_id {
            settings.swipe_right_playlist_id = WATCH_LATER_PLAYLIST_ID.into();
        }
        if settings.swipe_left_playlist_id == playlist_id {
            settings.swipe_left_playlist_id = WATCH_LATER_PLAYLIST_ID.into();
        }
        settings.playlist_views.remove(playlist_id);
        drop(settings);
        let id = playlist_id.to_string();
        self.send(DataChange::Playlists, async move {
            api::delete_playlist(id).await?;
            invalidate(DataChange::Queue);
            Ok(())
        });
        Some(name)
    }

    /// Drop every already-watched video from a playlist. The server decides
    /// which, since it knows what every device has watched; the count comes
    /// back in a toast.
    pub fn remove_watched_from_playlist(self, playlist_id: &str) {
        let id = playlist_id.to_string();
        spawn(async move {
            let result = api::remove_watched_from_playlist(id).await;
            invalidate(DataChange::Playlists);
            match result {
                Ok(0) => self.show_toast("Nothing watched to remove", Color::Neutral),
                Ok(removed) => self.show_toast(
                    format!(
                        "Removed {removed} watched {}",
                        if removed == 1 { "video" } else { "videos" }
                    ),
                    Color::Success,
                ),
                Err(_) => self.show_toast("Could not remove watched videos", Color::Danger),
            }
        });
    }

    // -----------------------------------------------------------------------
    // Subscriptions and groups
    // -----------------------------------------------------------------------

    /// Whether this account follows the channel.
    pub fn follows(self, channel_id: &str) -> bool {
        self.with_viewer(|viewer| viewer.follows(channel_id).is_some())
    }

    /// Follow or unfollow a channel. Returns the new answer.
    pub fn toggle_subscription_for(self, channel: &Channel) -> bool {
        let current = self.peek_viewer(|viewer| viewer.follows(&channel.id).cloned());
        let subscribed = current.is_none();
        let content = current
            .map(|channel| channel.subscription_content)
            .unwrap_or(channel.subscription_content);
        self.set_subscriptions(vec![SubscriptionChange {
            channel: channel.clone(),
            subscribed,
            content,
        }]);
        subscribed
    }

    /// Narrow (or widen) which of a channel's uploads reach the feed. Leaves
    /// the subscription itself alone: this is a filter, not an unsubscribe.
    pub fn set_subscription_content(self, channel_id: &str, content: SubscriptionContent) -> bool {
        let Some(channel) = self.peek_viewer(|viewer| viewer.follows(channel_id).cloned()) else {
            return false;
        };
        if channel.subscription_content == content {
            return false;
        }
        self.set_subscriptions(vec![SubscriptionChange {
            channel,
            subscribed: true,
            content,
        }]);
        true
    }

    fn set_subscriptions(self, changes: Vec<SubscriptionChange>) {
        let shown = changes.clone();
        self.edit_viewer(move |viewer| {
            for change in shown {
                viewer
                    .subscriptions
                    .retain(|channel| channel.id != change.channel.id);
                if change.subscribed {
                    viewer.subscriptions.push(Channel {
                        subscribed: true,
                        subscription_content: change.content,
                        ..change.channel
                    });
                }
            }
            viewer
                .subscriptions
                .sort_by_key(|channel| channel.name.to_lowercase());
        });
        self.send(DataChange::Subscriptions, api::set_subscriptions(changes));
    }

    /// Apply a parsed subscription export.
    ///
    /// Channels already followed are counted, not rewritten. Unknown channels
    /// are sent as stubs carrying only the id and name from the export: the
    /// server adds them to the catalog, and the next refresh fills in the
    /// avatar, handle and uploads.
    pub fn import_subscriptions(self, parsed: ParsedImport) -> ImportSummary {
        let mut summary = ImportSummary::default();
        let viewer = self.peek_viewer(Clone::clone);
        let mut changes = Vec::new();
        for entry in parsed.subscriptions {
            // Handle-only records cannot be matched or fetched offline. They
            // are counted rather than silently dropped so the toast can say so.
            if entry.channel_id.is_empty() {
                summary.unresolved += 1;
                continue;
            }
            if viewer.follows(&entry.channel_id).is_some() {
                summary.already_subscribed += 1;
                continue;
            }
            summary.subscribed += 1;
            changes.push(SubscriptionChange {
                channel: Channel {
                    id: entry.channel_id,
                    name: entry.name,
                    handle: entry.handle,
                    avatar_url: None,
                    subscriber_count: String::new(),
                    subscribed: true,
                    description: String::new(),
                    banner_url: None,
                    subscription_content: entry.content,
                },
                subscribed: true,
                content: entry.content,
            });
        }

        // Groups only ever arrive from Tawny's own export. Matching on name
        // keeps a re-import from stacking duplicates of the same group.
        let mut groups = Vec::new();
        for group in parsed.groups {
            match viewer
                .subscription_groups
                .iter()
                .find(|existing| existing.name.eq_ignore_ascii_case(&group.name))
            {
                Some(existing) => {
                    let mut merged = existing.clone();
                    for channel_id in group.channel_ids {
                        if !merged.channel_ids.contains(&channel_id) {
                            merged.channel_ids.push(channel_id);
                        }
                    }
                    if merged != *existing {
                        groups.push(merged);
                    }
                }
                None => {
                    summary.groups += 1;
                    groups.push(group);
                }
            }
        }

        if !changes.is_empty() {
            self.set_subscriptions(changes);
        }
        if !groups.is_empty() {
            self.save_groups(groups);
        }
        summary
    }

    fn save_groups(self, groups: Vec<SubscriptionGroup>) {
        let shown = groups.clone();
        self.edit_viewer(move |viewer| {
            for group in shown {
                match viewer
                    .subscription_groups
                    .iter_mut()
                    .find(|existing| existing.id == group.id)
                {
                    Some(existing) => *existing = group,
                    None => viewer.subscription_groups.push(group),
                }
            }
        });
        self.send(DataChange::Groups, api::save_groups(groups));
    }

    pub fn create_subscription_group(self, name: String) -> String {
        let slug = name
            .to_lowercase()
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() {
                    character
                } else {
                    '-'
                }
            })
            .collect::<String>()
            .trim_matches('-')
            .to_string();
        let suffix = random_suffix();
        let id = if slug.is_empty() {
            format!("group-{suffix}")
        } else {
            format!("{slug}-{suffix}")
        };
        self.save_groups(vec![SubscriptionGroup {
            id: id.clone(),
            name,
            channel_ids: Vec::new(),
        }]);
        id
    }

    pub fn delete_subscription_group(self, group_id: &str) -> Option<String> {
        let name = self.peek_viewer(|viewer| {
            viewer
                .subscription_groups
                .iter()
                .find(|group| group.id == group_id)
                .map(|group| group.name.clone())
        })?;
        self.edit_viewer(|viewer| {
            viewer
                .subscription_groups
                .retain(|group| group.id != group_id)
        });
        self.send(DataChange::Groups, api::delete_group(group_id.to_string()));
        Some(name)
    }

    /// Put a channel in a group or take it out. Returns whether it is in the
    /// group now.
    pub fn toggle_channel_in_group(self, group_id: &str, channel_id: &str) -> Option<bool> {
        let included = self.peek_viewer(|viewer| {
            viewer
                .subscription_groups
                .iter()
                .find(|group| group.id == group_id)
                .map(|group| group.channel_ids.iter().any(|id| id == channel_id))
        })?;
        let member = !included;
        self.edit_viewer(|viewer| {
            if let Some(group) = viewer
                .subscription_groups
                .iter_mut()
                .find(|group| group.id == group_id)
            {
                group.channel_ids.retain(|id| id != channel_id);
                if member {
                    group.channel_ids.push(channel_id.to_string());
                }
            }
        });
        self.send(
            DataChange::Groups,
            api::set_in_group(group_id.to_string(), channel_id.to_string(), member),
        );
        Some(member)
    }

    // -----------------------------------------------------------------------
    // Swipes and sharing
    // -----------------------------------------------------------------------

    /// Run whichever action a swipe direction is configured for.
    ///
    /// Centralised so the gesture, the desktop buttons, and the settings page
    /// all agree on what a direction means.
    pub fn run_swipe_action(self, video: &Video, start_side: bool) {
        use crate::models::SwipeActionKind;

        let settings = self.settings();
        let (kind, playlist_id) = if start_side {
            (
                settings.swipe_right_action,
                settings.swipe_right_playlist_id,
            )
        } else {
            (settings.swipe_left_action, settings.swipe_left_playlist_id)
        };
        match kind {
            SwipeActionKind::AddToPlaylist => match self.add_to_playlist(video, &playlist_id) {
                Some(PlaylistSave::Saved(name)) => {
                    self.show_toast(format!("Added to {name}"), Color::Success)
                }
                Some(PlaylistSave::AlreadyThere(name)) => {
                    self.show_toast(format!("Already in {name}"), Color::Warning)
                }
                None => {}
            },
            SwipeActionKind::AddToQueue => {
                let message = self.add_to_queue(&video.id, false);
                self.show_toast(message, Color::Success);
            }
            SwipeActionKind::PlayNext => {
                let message = self.add_to_queue(&video.id, true);
                self.show_toast(message, Color::Success);
            }
            SwipeActionKind::MarkWatched => {
                self.mark_watched(video, true);
                self.show_toast("Marked watched", Color::Neutral);
            }
            SwipeActionKind::Share => self.open_share(video.clone()),
        }
    }

    pub fn open_share(mut self, video: Video) {
        self.share_timestamp_seconds.set(0);
        self.share_with_timestamp.set(false);
        self.share_video.set(Some(video.clone()));
        self.share_open.set(true);

        if self
            .active_video()
            .as_ref()
            .is_some_and(|active| active.id == video.id)
        {
            spawn(async move {
                let mut eval = document::eval(
                    "dioxus.send(Math.max(0, Math.floor(document.querySelector('#tawny-player video')?.currentTime || 0)));",
                );
                if let Ok(seconds) = eval.recv::<u64>().await {
                    self.share_timestamp_seconds.set(seconds);
                }
            });
        }
    }

    /// Which action a swipe on this side is set to, so a card can show the
    /// icon for what it will actually do.
    pub fn swipe_action_kind(self, start_side: bool) -> crate::models::SwipeActionKind {
        let settings = self.settings();
        if start_side {
            settings.swipe_right_action
        } else {
            settings.swipe_left_action
        }
    }

    /// What a swipe on this side says it will do, named in full: a playlist
    /// action carries the playlist's own name, and the rest speak for
    /// themselves. This is the accessible name.
    pub fn swipe_action_label(self, start_side: bool) -> String {
        use crate::models::SwipeActionKind;

        let settings = self.settings();
        let (kind, playlist_id) = if start_side {
            (
                settings.swipe_right_action,
                settings.swipe_right_playlist_id,
            )
        } else {
            (settings.swipe_left_action, settings.swipe_left_playlist_id)
        };
        match kind {
            SwipeActionKind::AddToPlaylist => {
                match self
                    .with_viewer(|viewer| viewer.playlist(&playlist_id).map(|p| p.name.clone()))
                {
                    Some(name) => format!("Add to {name}"),
                    None => SwipeActionKind::AddToPlaylist.label().to_string(),
                }
            }
            other => other.label().to_string(),
        }
    }

    /// How this playlist is currently arranged, defaults included.
    pub fn playlist_view(self, playlist_id: &str) -> PlaylistView {
        self.settings
            .read()
            .playlist_views
            .get(playlist_id)
            .cloned()
            .unwrap_or_default()
    }

    /// Remember how a playlist is arranged, so a run resolved later walks the
    /// list the viewer was actually looking at.
    ///
    /// A view holding nothing but defaults is removed rather than stored: the map
    /// is keyed by playlist id and would otherwise accumulate an entry for every
    /// playlist ever opened.
    pub fn set_playlist_view(mut self, playlist_id: &str, view: PlaylistView) {
        let mut settings = self.settings.write();
        if view == PlaylistView::default() {
            settings.playlist_views.remove(playlist_id);
        } else {
            settings
                .playlist_views
                .insert(playlist_id.to_string(), view);
        }
    }

    // -----------------------------------------------------------------------
    // Runs: the queue, and a playlist playing through it
    // -----------------------------------------------------------------------

    /// The running playlist's videos in the order its page is showing them,
    /// with the watched filter *not* applied - see [`PlaylistView::arrange`].
    /// Empty when `playlist_id` is not the one running.
    pub fn playlist_run_order(self, playlist_id: &str) -> Vec<Video> {
        let settings = self.settings();
        let view = self.playlist_view(playlist_id);
        let mut videos = match &*self.run_playlist.peek() {
            Some(Ok(Some(contents))) if contents.playlist.id == playlist_id => {
                contents.videos.clone()
            }
            _ => Vec::new(),
        };
        view.arrange(&mut videos, &settings);
        videos
    }

    fn next_in_playlist_run(self, playlist_id: &str, current_video_id: &str) -> Option<String> {
        self.playlist_view(playlist_id)
            .next_after(&self.playlist_run_order(playlist_id), current_video_id)
    }

    fn previous_in_playlist_run(self, playlist_id: &str, current_video_id: &str) -> Option<String> {
        self.playlist_view(playlist_id)
            .previous_before(&self.playlist_run_order(playlist_id), current_video_id)
    }

    /// What the run would hand over next, plus the playlist markers it walked
    /// past on the way.
    ///
    /// Resolved in queue order, so a video queued by hand still plays before a
    /// playlist sitting behind it. A marker the run has reached the end of - or
    /// one whose playlist has been deleted - resolves to nothing and is reported
    /// as spent so the caller can drop it.
    fn resolve_next_in_run(self, finished_video_id: &str) -> (Option<String>, Vec<String>) {
        let queue = self.peek_viewer(|viewer| viewer.queue.clone());
        let mut spent_markers = Vec::new();
        for entry in queue {
            match queued_playlist_id(&entry) {
                Some(playlist_id) => {
                    match self.next_in_playlist_run(playlist_id, finished_video_id) {
                        Some(video_id) => return (Some(video_id), spent_markers),
                        None => spent_markers.push(entry),
                    }
                }
                None if entry != finished_video_id => return (Some(entry), spent_markers),
                None => {}
            }
        }
        (None, spent_markers)
    }

    /// What the run would play after `current_video_id`, without consuming it.
    ///
    /// Pass an empty id to ask what the run starts with.
    pub fn next_in_run(self, current_video_id: &str) -> Option<String> {
        // Subscribes: a control showing "next" follows the queue.
        let _ = self.viewer.read();
        let _ = self.run_playlist.read();
        self.resolve_next_in_run(current_video_id).0
    }

    /// Whether there is anywhere forward to go from this video.
    pub fn has_next_in_run(self, current_video_id: &str) -> bool {
        self.next_in_run(current_video_id).is_some()
    }

    /// Take the next video in the run, skipping the one that just finished, and
    /// remember the one being left so [`Self::step_back_in_run`] can return to it.
    ///
    /// Removing the video as it is handed over is what stops autoplay looping: an
    /// entry that stays queued would be chosen again the moment it ends. A
    /// playlist marker is the exception - it is the run, not a place in it, so it
    /// stays until the playlist is spent.
    pub fn take_next_queued(self, finished_video_id: &str) -> Option<String> {
        let (next, spent_markers) = self.resolve_next_in_run(finished_video_id);
        let video_id = next?;
        self.edit_queue(|queue| {
            queue.retain(|entry| {
                entry != finished_video_id && entry != &video_id && !spent_markers.contains(entry)
            });
        });
        self.push_back_stack(finished_video_id);
        Some(video_id)
    }

    /// Whether there is a video to go back to without leaving the run.
    ///
    /// Either somewhere this run has already been, or - on the first video of a
    /// playlist run, where the stack is still empty - the entry before it in the
    /// playlist's own order.
    pub fn can_step_back_in_run(self, current_video_id: &str) -> bool {
        if !(self.run_back_stack)().is_empty() {
            return true;
        }
        let _ = self.run_playlist.read();
        self.queued_playlist_run()
            .and_then(|playlist_id| self.previous_in_playlist_run(&playlist_id, current_video_id))
            .is_some()
    }

    /// The playlist the queue is running, as (id, name), when it still exists.
    pub fn running_playlist(self) -> Option<(String, String)> {
        let playlist_id = self.queued_playlist_run()?;
        self.with_viewer(|viewer| {
            viewer
                .playlist(&playlist_id)
                .map(|playlist| (playlist.id.clone(), playlist.name.clone()))
        })
    }

    /// Where the playlist run stands, for the queue page to explain it.
    ///
    /// Positions count the arranged order *without* the watched filter, so the
    /// number does not jump back to 1 every time finishing a video hides it.
    /// `remaining` is what the run will actually still play.
    pub fn playlist_run_status(self) -> Option<PlaylistRunStatus> {
        let (playlist_id, name) = self.running_playlist()?;
        let _ = self.run_playlist.read();
        let view = self.playlist_view(&playlist_id);
        let order = self.playlist_run_order(&playlist_id);
        let current_id = self
            .active_video()
            .map(|video| video.id)
            .unwrap_or_default();
        let index = order.iter().position(|video| video.id == current_id);
        let ahead = index.map_or(&order[..], |index| &order[index + 1..]);
        let remaining = ahead.iter().filter(|video| view.shows(video)).count();
        let up_next = view
            .next_after(&order, &current_id)
            .and_then(|id| order.iter().find(|video| video.id == id).cloned());
        Some(PlaylistRunStatus {
            playlist_id,
            name,
            total: order.len(),
            position: index.map(|index| index + 1),
            current: index.map(|index| order[index].clone()),
            remaining,
            up_next,
            view,
        })
    }

    /// The name of the playlist the queue is running, for controls that would
    /// otherwise only be able to say "queue".
    pub fn running_playlist_name(self) -> Option<String> {
        self.running_playlist().map(|(_, name)| name)
    }

    /// The playlist the queue is currently running, if any.
    fn queued_playlist_run(self) -> Option<String> {
        self.with_viewer(|viewer| {
            viewer
                .queue
                .iter()
                .find_map(|entry| queued_playlist_id(entry).map(str::to_string))
        })
    }

    /// Return to the video this run last advanced away from, or to the previous
    /// entry of the playlist being run when there is no history yet.
    ///
    /// The video being left goes back to the front of the queue, so the run is
    /// exactly where it was: pressing next again plays it, rather than skipping
    /// whatever the viewer stepped back past. A playlist run needs no such
    /// bookmark - it finds its place from whatever is playing - so stepping back
    /// into one leaves the queue alone.
    pub fn step_back_in_run(mut self, current_video_id: &str) -> Option<String> {
        let Some(previous) = self.run_back_stack.write().pop() else {
            let playlist_id = self.queued_playlist_run()?;
            return self.previous_in_playlist_run(&playlist_id, current_video_id);
        };
        if previous == current_video_id {
            return None;
        }
        self.edit_queue(|queue| {
            queue.retain(|id| id != current_video_id);
            queue.insert(0, current_video_id.to_string());
        });
        Some(previous)
    }

    /// Hand the queue a playlist to run, replacing any playlist already running.
    ///
    /// Goes to the front: pressing play on a playlist is a request to watch it
    /// now, not after whatever was queued by hand last week. Only one playlist
    /// runs at a time, because two markers would interleave in an order neither
    /// playlist's page could show.
    pub fn start_playlist_run(self, playlist_id: &str) {
        let marker = playlist_queue_entry(playlist_id);
        self.edit_queue(|queue| {
            queue.retain(|entry| queued_playlist_id(entry).is_none());
            queue.insert(0, marker);
        });
    }

    /// Stop running a playlist, leaving anything queued by hand in place.
    pub fn clear_playlist_run(self) -> bool {
        let running = self.peek_viewer(|viewer| {
            viewer
                .queue
                .iter()
                .any(|entry| queued_playlist_id(entry).is_some())
        });
        if running {
            self.edit_queue(|queue| queue.retain(|entry| queued_playlist_id(entry).is_none()));
        }
        running
    }

    fn push_back_stack(mut self, video_id: &str) {
        let mut stack = self.run_back_stack.write();
        if stack.last().is_some_and(|last| last == video_id) {
            return;
        }
        stack.push(video_id.to_string());
        // One sitting's worth. The stack is only ever walked from the end.
        if stack.len() > 50 {
            stack.remove(0);
        }
    }

    /// Change the queue: shown at once, and the whole list sent as the answer.
    fn edit_queue(self, edit: impl FnOnce(&mut Vec<String>)) {
        let mut queue = self.peek_viewer(|viewer| viewer.queue.clone());
        edit(&mut queue);
        let shown = queue.clone();
        self.edit_viewer(move |viewer| viewer.queue = shown);
        self.send(DataChange::Queue, api::set_queue(queue));
    }

    /// Append a whole run of videos to the queue, in the order given.
    ///
    /// Ids already in the queue are moved rather than duplicated, because the
    /// run is what the viewer just chose to watch, in the order they chose it;
    /// a stale copy earlier in the queue would play them out of that order.
    pub fn queue_run(self, video_ids: &[String]) -> usize {
        if video_ids.is_empty() {
            return 0;
        }
        self.edit_queue(|queue| {
            queue.retain(|id| !video_ids.contains(id));
            queue.extend(video_ids.iter().cloned());
        });
        video_ids.len()
    }

    pub fn add_to_queue(self, video_id: &str, play_next: bool) -> String {
        self.edit_queue(|queue| {
            queue.retain(|id| id != video_id);
            if play_next {
                queue.insert(0, video_id.to_string());
            } else {
                queue.push(video_id.to_string());
            }
        });
        if play_next {
            "Playing next".into()
        } else {
            "Added to queue".into()
        }
    }

    pub fn remove_from_queue(self, video_id: &str) -> bool {
        let queued = self.peek_viewer(|viewer| viewer.queue.iter().any(|id| id == video_id));
        if queued {
            self.edit_queue(|queue| queue.retain(|id| id != video_id));
        }
        queued
    }

    pub fn clear_queue(self) {
        self.edit_queue(Vec::clear);
    }

    pub fn record_history(self, video_id: &str) {
        // Reopening the video already at the top is the usual case - expanding
        // the mini player - and needs nothing sent.
        let already_first = self.peek_viewer(|viewer| {
            viewer
                .history
                .first()
                .is_some_and(|entry| entry.video_id == video_id)
        });
        if already_first {
            return;
        }
        let entry = HistoryEntry {
            video_id: video_id.to_string(),
            played_at: "Just now".into(),
        };
        self.edit_viewer(move |viewer| {
            viewer.history.retain(|old| old.video_id != entry.video_id);
            viewer.history.insert(0, entry);
            viewer.history.truncate(HISTORY_LIMIT);
        });
        self.send(
            DataChange::History,
            api::record_history(video_id.to_string()),
        );
    }

    pub fn clear_history(self) {
        self.edit_viewer(|viewer| viewer.history.clear());
        self.send(DataChange::History, api::clear_history());
    }
}

/// A short random id suffix, for a playlist or group made on this device
/// before the server has seen it.
fn random_suffix() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = web_time::SystemTime::now()
        .duration_since(web_time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    let mixed = nanos ^ COUNTER.fetch_add(1, Ordering::Relaxed).rotate_left(40);
    format!("{mixed:x}")
}

/// The localStorage key the whole library used to live under. Emptied on
/// start, so a device upgraded from that design gets its storage back.
#[cfg(not(feature = "server"))]
const RETIRED_LIBRARY_KEY: &str = "tawny-library-v1";

#[component]
pub fn AppStateProvider(children: Element) -> Element {
    // Before anything reads: `use_cached` persists nothing until the cache
    // knows whose data it holds.
    g3_cache::use_client_cache(g3_cache::CacheConfig::new("tawny"));
    // Every read is scoped to an account and refused without one. A first
    // launch refetches once its guest account exists (see `session`).
    let session = use_session_provider();
    let settings = use_persistent_signal("tawny-settings-v1", 250, AppSettings::default);

    // Whose data the device cache holds. Not told "nobody" while the session
    // is still being looked up: that empties the cache, so every launch would
    // start cold. Only a sign-out, after someone was known, says so.
    let account = session.account;
    let mut owner_known = use_signal(|| false);
    use_effect(move || {
        let owner = account.read().as_ref().map(|account| account.id.clone());
        match owner {
            Some(owner) => {
                owner_known.set(true);
                spawn(g3_cache::set_cache_owner(Some(owner)));
            }
            None if *owner_known.peek() => {
                spawn(g3_cache::set_cache_owner(None));
            }
            None => {}
        }
    });

    #[cfg(not(feature = "server"))]
    use_hook(|| {
        spawn(async {
            let _ = document::eval(&format!(
                "window.localStorage.removeItem({RETIRED_LIBRARY_KEY:?});"
            ))
            .await;
        });
    });

    let viewer = use_cached(get_viewer, ());

    let running = use_memo(move || match &*viewer.read() {
        Some(Ok(viewer)) => viewer
            .queue
            .iter()
            .find_map(|entry| queued_playlist_id(entry).map(str::to_string))
            .unwrap_or_default(),
        _ => String::new(),
    });
    // Under `get_playlist`'s own key, so it shares an entry with the playlist
    // page; with no run, answered here instead of asking the server about an
    // empty id.
    let run_key = g3_cache::CacheKey::of(&get_playlist, &(running(),));
    let run_playlist = g3_cache::use_cached_key(run_key, move || {
        let playlist_id = running.peek().clone();
        async move {
            if playlist_id.is_empty() {
                Ok(None)
            } else {
                get_playlist(playlist_id).await
            }
        }
    });

    use_context_provider(|| AppState {
        viewer,
        run_playlist,
        settings,
        pending_progress: Signal::new(HashMap::new()),
        unshown_progress: Signal::new(HashMap::new()),
        toaster: Signal::new(None),
        playlist_picker_video: Signal::new(None),
        playlist_picker_open: Signal::new(false),
        video_actions_video: Signal::new(None),
        video_actions_playlist: Signal::new(None),
        video_actions_open: Signal::new(false),
        share_video: Signal::new(None),
        share_open: Signal::new(false),
        share_with_timestamp: Signal::new(false),
        share_timestamp_seconds: Signal::new(0),
        active_video: Signal::new(None),
        active_captions: Signal::new(Vec::new()),
        selected_caption: Signal::new(None),
        captions_enabled: Signal::new(false),
        active_chapters: Signal::new(Vec::new()),
        active_sponsor_segments: Signal::new(Vec::new()),
        active_audio_tracks: Signal::new(Vec::new()),
        selected_audio_track: Signal::new(None),
        active_preview_frames: Signal::new(None),
        syncing: Signal::new(false),
        feed_reselected: Signal::new(0),
        chapters_sheet_open: Signal::new(false),
        run_back_stack: Signal::new(Vec::new()),
        feed_filter: Signal::new(FeedFilter::All),
        explore_filter: Signal::new(crate::models::ExploreFilter::All),
        explore_search: Signal::new(String::new()),
        explore_results: Signal::new(None),
        channel_tab: Signal::new(FeedFilter::All),
        feed_group: Signal::new(crate::models::FeedGroup::All),
        feed_duration: Signal::new(None),
        feed_pages: Signal::new(1),
    });

    rsx! { {children} }
}
