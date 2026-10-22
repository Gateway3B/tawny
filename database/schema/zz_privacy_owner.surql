-- Every private write and account removal must conflict on the account row.
-- A session checked before deletion cannot authorize a later orphaned write.
DEFINE FIELD IF NOT EXISTS privacy_revision ON app_user TYPE option<int> DEFAULT 0 ASSERT $value = NONE OR $value >= 0;

DEFINE FUNCTION IF NOT EXISTS fn::privacy_active_owner_v1($owner: record<app_user>) {
    LET $active = (UPDATE $owner SET privacy_revision = (privacy_revision ?? 0) + 1 RETURN VALUE id);
    IF array::len($active) != 1 { THROW 'Account is unavailable'; };
    RETURN true;
};

DEFINE FUNCTION IF NOT EXISTS fn::privacy_owned_write_v1($before: option<object>, $after: object, $event: string) {
    IF $event = 'UPDATE' AND $before.owner != NONE AND $after.owner != $before.owner {
        THROW 'Account ownership cannot be changed';
    };
    -- Legacy unowned catalog/demo rows are not assigned to an arbitrary user.
    IF $after.owner != NONE { fn::privacy_active_owner_v1($after.owner); };
    RETURN true;
};

DEFINE EVENT IF NOT EXISTS privacy_owner_write_v1 ON TABLE playlist WHEN $event IN ['CREATE', 'UPDATE'] THEN fn::privacy_owned_write_v1($before, $after, $event);
DEFINE EVENT IF NOT EXISTS privacy_owner_write_v1 ON TABLE library_state WHEN $event IN ['CREATE', 'UPDATE'] THEN fn::privacy_owned_write_v1($before, $after, $event);
DEFINE EVENT IF NOT EXISTS privacy_owner_write_v1 ON TABLE subscription_group WHEN $event IN ['CREATE', 'UPDATE'] THEN fn::privacy_owned_write_v1($before, $after, $event);
DEFINE EVENT IF NOT EXISTS privacy_owner_write_v1 ON TABLE user_subscription WHEN $event IN ['CREATE', 'UPDATE'] THEN fn::privacy_owned_write_v1($before, $after, $event);
DEFINE EVENT IF NOT EXISTS privacy_owner_write_v1 ON TABLE video_progress WHEN $event IN ['CREATE', 'UPDATE'] THEN fn::privacy_owned_write_v1($before, $after, $event);
DEFINE EVENT IF NOT EXISTS privacy_owner_write_v1 ON TABLE brand_legal_receipt WHEN $event IN ['CREATE', 'UPDATE'] THEN fn::privacy_owned_write_v1($before, $after, $event);

DEFINE EVENT IF NOT EXISTS privacy_subscription_write_v1 ON TABLE subscription WHEN $event IN ['CREATE', 'UPDATE'] THEN {
    IF $event = 'UPDATE' AND ($before.in != $after.in OR $before.out != $after.out) {
        THROW 'Subscription endpoints cannot be changed';
    };
    fn::privacy_active_owner_v1($after.in);
};

DEFINE EVENT IF NOT EXISTS privacy_playlist_item_write_v1 ON TABLE playlist_item WHEN $event IN ['CREATE', 'UPDATE'] THEN {
    IF $event = 'UPDATE' AND ($before.in != $after.in OR $before.out != $after.out) {
        THROW 'Playlist item endpoints cannot be changed';
    };
    LET $playlist = (SELECT id, owner FROM ONLY $after.in);
    IF $playlist = NONE { THROW 'Playlist is unavailable'; };
    IF $playlist.owner != NONE { fn::privacy_active_owner_v1($playlist.owner); };
};
