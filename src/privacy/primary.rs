//! One primary erase program shared by live deletion and recovery processing.
macro_rules! erase_primary_sql {
    () => { r#"
    LET $channels = SELECT VALUE channel_id FROM user_subscription WHERE owner = type::record($owner);
    LET $playlists = SELECT VALUE id FROM playlist WHERE owner = type::record($owner);
    DELETE playlist_item WHERE in IN $playlists;
    DELETE playlist WHERE owner = type::record($owner);
    DELETE subscription_group WHERE owner = type::record($owner);
    DELETE library_state WHERE owner = type::record($owner);
    DELETE video_progress WHERE owner = type::record($owner);
    DELETE user_subscription WHERE owner = type::record($owner);
    DELETE subscription WHERE in = type::record($owner);
    DELETE brand_legal_receipt WHERE owner = type::record($owner);
    FOR $channel_id IN $channels {
        UPDATE channel SET subscribed = count(SELECT id FROM user_subscription WHERE channel_id = $channel_id AND subscribed = true) > 0 WHERE channel_id = $channel_id;
    };
    fn::privacy_retire_sessions_v1(type::record($owner));
    DELETE type::record($owner);
    IF count(SELECT id FROM playlist_item WHERE in IN $playlists) != 0 { THROW 'Primary relation removal is pending'; };
"# };
}
pub(crate) use erase_primary_sql;
