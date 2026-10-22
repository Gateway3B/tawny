-- Account, library and session removal commit together. Do not report success
-- while a legacy payload has not been assigned to its exact account.
DEFINE FUNCTION IF NOT EXISTS fn::privacy_retire_sessions_v1($account: record<app_user>) {
    IF array::len(SELECT VALUE id FROM sessions WHERE account_indexed != true LIMIT 1) != 0 {
        THROW 'Session migration is pending';
    };
    -- The SQL representation matches the typed key used by g3-auth.
    LET $fence = type::record('auth_session_subject', <string>$account);
    UPSERT $fence SET account = $account, retired = true, touched_at = time::now();
    DELETE sessions WHERE session_account = $account;
    RETURN true;
} PERMISSIONS NONE;
