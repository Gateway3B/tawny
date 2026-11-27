//! Where playback reached, kept on this device as well as on the server.
//!
//! A downloaded video is mostly watched with no connection, and every
//! five-second tick then failed to reach the server and said so in a toast.
//! Positions now land here first, in local storage, and the ones the server
//! has not taken yet are sent again with the next tick that can reach it. The
//! player also resumes from here, so a video left offline picks up where it
//! was even though the server never heard.

use crate::models::VideoProgress;
use serde::{Deserialize, Serialize};

/// How many videos' positions are kept. A position matters for the videos
/// someone is part way through, not for everything they ever opened.
const LIMIT: usize = 300;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LocalProgress {
    /// The latest position of each video, most recently recorded last.
    positions: Vec<VideoProgress>,
    /// Videos whose latest position the server has not taken yet.
    unsent: Vec<String>,
}

impl LocalProgress {
    /// Keeps `progress` as the video's latest position, owed to the server.
    pub fn record(&mut self, progress: VideoProgress) {
        let id = progress.video_id.clone();
        self.positions.retain(|kept| kept.video_id != id);
        self.positions.push(progress);
        if self.positions.len() > LIMIT {
            let dropped = self.positions.remove(0);
            self.unsent.retain(|unsent| *unsent != dropped.video_id);
        }
        if !self.unsent.contains(&id) {
            self.unsent.push(id);
        }
    }

    /// This device's latest position for `video_id`.
    pub fn get(&self, video_id: &str) -> Option<&VideoProgress> {
        self.positions.iter().find(|kept| kept.video_id == video_id)
    }

    /// Whether this device holds a position for `video_id` that the server
    /// has not seen, so the server's copy is older than this one.
    pub fn is_unsent(&self, video_id: &str) -> bool {
        self.unsent.iter().any(|unsent| unsent == video_id)
    }

    /// Every position still owed to the server.
    pub fn unsent(&self) -> Vec<VideoProgress> {
        self.unsent
            .iter()
            .filter_map(|id| self.get(id).cloned())
            .collect()
    }

    /// The server took `sent`. A video that has moved on since it was sent
    /// is still owed its newer position.
    pub fn mark_sent(&mut self, sent: &[VideoProgress]) {
        let positions = &self.positions;
        self.unsent.retain(|id| {
            let latest = positions.iter().find(|kept| kept.video_id == *id);
            !sent.iter().any(|entry| Some(entry) == latest)
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(video_id: &str, seconds: u64) -> VideoProgress {
        VideoProgress {
            video_id: video_id.into(),
            watched: false,
            progress_seconds: seconds,
            audio_only: false,
        }
    }

    #[test]
    fn a_position_is_kept_and_owed_until_the_server_takes_it() {
        let mut local = LocalProgress::default();
        local.record(at("a", 10));
        local.record(at("a", 15));
        assert_eq!(local.get("a").map(|kept| kept.progress_seconds), Some(15));
        assert_eq!(local.unsent(), vec![at("a", 15)]);
        local.mark_sent(&[at("a", 15)]);
        assert!(!local.is_unsent("a"));
        // Still remembered for resuming.
        assert_eq!(local.get("a").map(|kept| kept.progress_seconds), Some(15));
    }

    #[test]
    fn a_position_that_moved_on_while_sending_is_still_owed() {
        let mut local = LocalProgress::default();
        local.record(at("a", 10));
        let sending = local.unsent();
        local.record(at("a", 20));
        local.mark_sent(&sending);
        assert_eq!(local.unsent(), vec![at("a", 20)]);
    }

    #[test]
    fn only_the_most_recent_videos_are_kept() {
        let mut local = LocalProgress::default();
        for index in 0..LIMIT + 5 {
            local.record(at(&format!("v{index}"), 1));
        }
        assert!(local.get("v0").is_none());
        assert!(!local.is_unsent("v0"));
        assert!(local.get(&format!("v{}", LIMIT + 4)).is_some());
        assert_eq!(local.unsent().len(), LIMIT);
    }
}
