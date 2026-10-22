-- Data repairs that must land before SurrealKit syncs the schema. SurrealKit
-- rewrites every DEFINE to OVERWRITE, so a changed schema file rebuilds its
-- indexes, and a UNIQUE index rebuild fails the whole startup on any row that
-- slipped past it. Each statement is idempotent and a no-op on a fresh
-- database, where the tables do not exist yet.

-- Some development databases hold two rows per (owner, channel_id) that the
-- unique index never saw. Keep the most recently written answer. WITH NOINDEX
-- because the stale index hides the duplicates from indexed lookups.
IF (INFO FOR DB).tables.user_subscription != NONE {
    -- Bind SELECT before iterating: the remote 3.2.4 protocol cannot execute
    -- this parenthesized SELECT directly as a FOR iterator on an empty table.
    LET $groups = (
        SELECT owner, channel_id, array::sort::desc(array::group([updated_at, id])) AS rows
        FROM user_subscription WITH NOINDEX
        GROUP BY owner, channel_id
    );
    FOR $group IN $groups {
        IF array::len($group.rows) > 1 {
            DELETE array::slice($group.rows, 1).map(|$row| $row[1]);
        };
    };
};

-- Sessions used to name their account by its full record id
-- ("app_user:k3j2h1"); g3-auth names it by the key alone. Rewrite the old
-- form so devices signed in before the change stay signed in, instead of
-- each minting a new guest and appearing to lose its library. The id is the
-- only "app_user:" in a session's stored JSON.
IF (INFO FOR DB).tables.sessions != NONE {
    UPDATE sessions SET sessionstore = string::replace(sessionstore, "app_user:", "")
        WHERE string::contains(sessionstore, "app_user:");
};
