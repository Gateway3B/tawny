use crate::models::{
    Account, ChannelDetails, ChannelMediaPage, CommentsPage, Credentials, FeedPage, FeedQuery,
    FeedRefreshResult, GroupSuggestion, LibraryChanges, PlaybackFailure, PlaybackFailureOrigin,
    PlaybackSession, Playlist, PlaylistContents, PlaylistPreview, SearchResults, SponsorSegment,
    SubscriptionChange, SubscriptionGroup, Video, VideoDetails, VideoProgress, Viewer,
};
use dioxus::prelude::*;

#[cfg(feature = "server")]
use crate::models::LibraryChange;

mod account;

mod catalog;

mod feed;

mod library;

mod playback;

pub use account::*;
pub use catalog::*;
pub use feed::*;
pub use library::*;
pub use playback::*;

#[cfg(feature = "server")]
fn server_error(error: anyhow::Error) -> ServerFnError {
    ServerFnError::new(error.to_string())
}
