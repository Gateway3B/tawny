use crate::models::{
    Account, ChannelDetails, ChannelMediaPage, CommentsPage, Credentials, FeedPage, FeedQuery,
    FeedRefreshResult, GroupSuggestion, LibraryChanges, LibraryExport, LibraryImport,
    PlaybackFailure, PlaybackFailureOrigin, PlaybackSession, Playlist, PlaylistContents,
    PlaylistPreview, SearchResults, SponsorSegment, SubscriptionChange, SubscriptionGroup, Video,
    VideoDetails, VideoProgress, Viewer,
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

/// What a client is told when a server function fails.
///
/// A [`Refusal`](crate::server::Refusal) or an
/// [`AuthError`](crate::auth::AuthError) was written for the viewer and passes
/// through. Anything else is the server's own failure: its text can carry
/// database errors, provider responses, URLs or record ids, so the client gets
/// a generic sentence and the log gets a code, never the text (see
/// `telemetry.rs`).
#[cfg(feature = "server")]
fn server_error(error: anyhow::Error) -> ServerFnError {
    ServerFnError::new(client_message(&error))
}

#[cfg(feature = "server")]
fn client_message(error: &anyhow::Error) -> String {
    if let Some(refusal) = error.downcast_ref::<crate::server::Refusal>() {
        return refusal.to_string();
    }
    if let Some(refusal) = error.downcast_ref::<crate::auth::AuthError>() {
        return refusal.message();
    }
    crate::telemetry::diagnostic(crate::telemetry::Diagnostic::ServerFunctionFailed);
    // Lower case: clients show it after "Could not save that: ".
    GENERIC_FAILURE.into()
}

#[cfg(feature = "server")]
const GENERIC_FAILURE: &str = "something went wrong on the server; try again";

#[cfg(all(test, feature = "server"))]
mod tests {
    use super::*;

    #[test]
    fn a_refusal_reaches_the_viewer_and_an_internal_failure_does_not() {
        let refusal = anyhow::Error::new(crate::server::Refusal("a group needs a name"));
        assert_eq!(client_message(&refusal), "a group needs a name");
        // Context added on the way up does not hide it.
        assert_eq!(
            client_message(&refusal.context("rename group")),
            "a group needs a name"
        );
        assert_eq!(
            client_message(&crate::auth::AuthError::InvalidLogin.into()),
            crate::auth::AuthError::InvalidLogin.message()
        );
        let internal = anyhow::anyhow!("query failed for app_user:secret at ws://surrealdb:8000");
        assert_eq!(client_message(&internal), GENERIC_FAILURE);
    }
}
