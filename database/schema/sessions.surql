-- Match the reviewed g3-auth account-tracked adapter. Untracked writers must
-- be stopped before migration; the event rejects a rollback to their format.
DEFINE TABLE IF NOT EXISTS sessions TYPE NORMAL SCHEMAFULL PERMISSIONS NONE;
DEFINE FIELD IF NOT EXISTS sessionstore ON sessions TYPE string;
DEFINE FIELD IF NOT EXISTS sessionexpires ON sessions TYPE string;
DEFINE FIELD IF NOT EXISTS sessionid ON sessions TYPE string;
DEFINE FIELD IF NOT EXISTS session_account ON sessions TYPE option<record<app_user>>;
DEFINE FIELD IF NOT EXISTS account_indexed ON sessions TYPE option<bool> DEFAULT false;
DEFINE INDEX IF NOT EXISTS sessions_sessionid_idx ON sessions FIELDS sessionid UNIQUE;
DEFINE INDEX IF NOT EXISTS sessions_account_idx ON sessions FIELDS session_account;
DEFINE INDEX IF NOT EXISTS sessions_unindexed_idx ON sessions FIELDS account_indexed;
DEFINE EVENT IF NOT EXISTS tracked_session_v1 ON TABLE sessions WHEN $event IN ['CREATE', 'UPDATE'] THEN {
    IF $after.account_indexed != true { THROW 'Account-tracked sessions are required'; };
};

-- Minimal, private, non-reused identity fence; never a public catalog record.
DEFINE TABLE IF NOT EXISTS auth_session_subject TYPE NORMAL SCHEMAFULL PERMISSIONS NONE;
DEFINE FIELD IF NOT EXISTS account ON auth_session_subject TYPE record<app_user>;
DEFINE FIELD IF NOT EXISTS retired ON auth_session_subject TYPE bool DEFAULT false;
DEFINE FIELD IF NOT EXISTS touched_at ON auth_session_subject TYPE datetime DEFAULT time::now();
DEFINE INDEX IF NOT EXISTS session_subject_account_idx ON auth_session_subject FIELDS account UNIQUE;

-- Account, library and session removal commit together. Do not report success
-- while a legacy payload has not been assigned to its exact account.
DEFINE FUNCTION IF NOT EXISTS fn::retire_account_sessions($account: record<app_user>) {
    IF array::len(SELECT VALUE id FROM sessions WHERE account_indexed != true LIMIT 1) != 0 {
        THROW 'Session migration is pending';
    };
    -- The SQL representation matches the typed key used by g3-auth.
    LET $fence = type::record('auth_session_subject', <string>$account);
    UPSERT $fence SET account = $account, retired = true, touched_at = time::now();
    DELETE sessions WHERE session_account = $account;
    RETURN true;
} PERMISSIONS NONE;
