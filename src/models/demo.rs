use super::*;

#[cfg_attr(not(feature = "server"), allow(dead_code))]
/// The seed catalog: a few videos and their channels, which the server writes
/// on first start, and the playlists the default swipes save to. A new
/// account follows none of the channels, so its feed starts empty.
#[derive(Clone, Debug, PartialEq)]
pub struct DemoLibrary {
    pub channels: Vec<Channel>,
    pub videos: Vec<Video>,
    pub playlists: Vec<Playlist>,
    pub queue: Vec<String>,
}

#[cfg_attr(not(feature = "server"), allow(dead_code))]
/// What a new account starts with: the default playlists and a queue, and no
/// follows or groups.
pub fn demo_viewer() -> Viewer {
    let demo = DemoLibrary::demo();
    Viewer {
        subscriptions: Vec::new(),
        playlists: demo.playlists,
        subscription_groups: Vec::new(),
        queue: demo.queue,
        history: Vec::new(),
    }
}

#[cfg_attr(not(feature = "server"), allow(dead_code))]
impl DemoLibrary {
    pub fn demo() -> Self {
        let channels = vec![
            Channel {
                id: "UC-tawny-byte".into(),
                name: "Byte Sized".into(),
                handle: "@bytesized".into(),
                avatar_url: None,
                subscriber_count: "482K".into(),
                subscribed: true,
                description: "Practical Rust and software architecture.".into(),
                banner_url: None,
                subscription_content: SubscriptionContent::All,
            },
            Channel {
                id: "UC-tawny-signal".into(),
                name: "Signal Path".into(),
                handle: "@signalpath".into(),
                avatar_url: None,
                subscriber_count: "1.2M".into(),
                subscribed: true,
                description: "Signals, space, and ambitious experiments.".into(),
                banner_url: None,
                subscription_content: SubscriptionContent::All,
            },
            Channel {
                id: "UC-tawny-field".into(),
                name: "Field Notes".into(),
                handle: "@fieldnotes".into(),
                avatar_url: None,
                subscriber_count: "206K".into(),
                subscribed: true,
                description: "Thoughtful stories from outdoors.".into(),
                banner_url: None,
                subscription_content: SubscriptionContent::All,
            },
            Channel {
                id: "UC-tawny-slow".into(),
                name: "Slow Living Lab".into(),
                handle: "@slowlivinglab".into(),
                avatar_url: None,
                subscriber_count: "734K".into(),
                subscribed: true,
                description: "Make technology feel quieter.".into(),
                banner_url: None,
                subscription_content: SubscriptionContent::All,
            },
        ];

        let videos = vec![
            Video {
                id: "aqz-KE-bpKQ".into(),
                title: "I rebuilt my tiny studio around one quiet idea".into(),
                channel_id: channels[3].id.clone(),
                channel_name: channels[3].name.clone(),
                thumbnail_url: "https://picsum.photos/seed/tawny-studio/1280/720".into(),
                published_at: "18 minutes ago".into(),
                duration_seconds: 1128,
                view_count: "42K views".into(),
                progress_seconds: 0,
                watched: false,
                is_live: false,
                is_short: false,
                audio_only: false,
                channel_avatar_url: None,
            },
            Video {
                id: "M7lc1UVf-VE".into(),
                title: "Rust UI architecture that still feels simple".into(),
                channel_id: channels[0].id.clone(),
                channel_name: channels[0].name.clone(),
                thumbnail_url: "https://picsum.photos/seed/tawny-rust/1280/720".into(),
                published_at: "2 hours ago".into(),
                duration_seconds: 1642,
                view_count: "91K views".into(),
                progress_seconds: 420,
                watched: false,
                is_live: false,
                is_short: false,
                audio_only: false,
                channel_avatar_url: None,
            },
            Video {
                id: "ysz5S6PUM-U".into(),
                title: "The surprising physics of a perfect trail".into(),
                channel_id: channels[2].id.clone(),
                channel_name: channels[2].name.clone(),
                thumbnail_url: "https://picsum.photos/seed/tawny-trail/1280/720".into(),
                published_at: "Yesterday".into(),
                duration_seconds: 812,
                view_count: "318K views".into(),
                progress_seconds: 0,
                watched: false,
                is_live: false,
                is_short: false,
                audio_only: false,
                channel_avatar_url: None,
            },
            Video {
                id: "jNQXAC9IVRw".into(),
                title: "Live: decoding the signal from deep space".into(),
                channel_id: channels[1].id.clone(),
                channel_name: channels[1].name.clone(),
                thumbnail_url: "https://picsum.photos/seed/tawny-space/1280/720".into(),
                published_at: "Streaming now".into(),
                duration_seconds: 0,
                view_count: "8.4K watching".into(),
                progress_seconds: 0,
                watched: false,
                is_live: true,
                is_short: false,
                audio_only: false,
                channel_avatar_url: None,
            },
            Video {
                id: "dQw4w9WgXcQ".into(),
                title: "Five shortcuts I wish I knew before switching editors".into(),
                channel_id: channels[0].id.clone(),
                channel_name: channels[0].name.clone(),
                thumbnail_url: "https://picsum.photos/seed/tawny-editor/1280/720".into(),
                published_at: "3 days ago".into(),
                duration_seconds: 625,
                view_count: "126K views".into(),
                progress_seconds: 625,
                watched: true,
                is_live: false,
                is_short: false,
                audio_only: false,
                channel_avatar_url: None,
            },
            Video {
                id: "ScMzIvxBSi4".into(),
                title: "A week in the high desert, without notifications".into(),
                channel_id: channels[2].id.clone(),
                channel_name: channels[2].name.clone(),
                thumbnail_url: "https://picsum.photos/seed/tawny-desert/1280/720".into(),
                published_at: "5 days ago".into(),
                duration_seconds: 2210,
                view_count: "604K views".into(),
                progress_seconds: 0,
                watched: false,
                is_live: false,
                is_short: false,
                audio_only: false,
                channel_avatar_url: None,
            },
        ];

        let playlists = vec![
            Playlist {
                id: "watch-later".into(),
                name: "Watch later".into(),
                video_ids: vec![videos[1].id.clone(), videos[2].id.clone()],
            },
            Playlist {
                id: "deep-dives".into(),
                name: "Deep dives".into(),
                video_ids: vec![videos[3].id.clone(), videos[5].id.clone()],
            },
            Playlist {
                id: "weekend-queue".into(),
                name: "Weekend queue".into(),
                video_ids: vec![videos[0].id.clone()],
            },
        ];

        Self {
            channels,
            videos,
            playlists,
            queue: vec!["M7lc1UVf-VE".into(), "ysz5S6PUM-U".into()],
        }
    }
}
