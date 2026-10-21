//! Which cached reads a change affects.
//!
//! After a mutation, the client refetches only the reads whose answers it
//! could have changed. Each variant names what was written; [`invalidate`]
//! maps it to the reads built from it. When a read starts depending on
//! something new, add it here.

use crate::api::{
    get_channel_details, get_feed_page, get_playlist, get_playlist_previews, get_video_details,
    get_videos, get_viewer,
};
use g3_cache::invalidate_cached;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DataChange {
    /// Follows, or which uploads a follow wants: the viewer, the feed, and the
    /// follow button on a channel or video.
    Subscriptions,
    /// Subscription groups: the viewer and the feed's group filter.
    Groups,
    /// A playlist created, renamed, deleted or its videos changed.
    Playlists,
    /// The queue. Its videos are a `get_videos` read keyed by the queue's ids,
    /// so a changed queue is a new key and needs nothing more.
    Queue,
    History,
    /// Videos marked watched or unwatched on another device: every list that
    /// shows it or hides watched videos. This device's own marks are applied
    /// to its cached lists directly, and need no refetch.
    Watched,
}

pub fn invalidate(change: DataChange) {
    match change {
        DataChange::Subscriptions => {
            invalidate_cached(get_viewer);
            invalidate_cached(get_feed_page);
            invalidate_cached(get_channel_details);
            invalidate_cached(get_video_details);
        }
        DataChange::Groups => {
            invalidate_cached(get_viewer);
            invalidate_cached(get_feed_page);
        }
        DataChange::Playlists => {
            invalidate_cached(get_viewer);
            invalidate_cached(get_playlist);
            invalidate_cached(get_playlist_previews);
        }
        DataChange::Queue | DataChange::History => invalidate_cached(get_viewer),
        DataChange::Watched => {
            invalidate_cached(get_feed_page);
            invalidate_cached(get_playlist);
            invalidate_cached(get_videos);
        }
    }
}
