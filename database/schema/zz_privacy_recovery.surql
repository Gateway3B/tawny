-- A durable primary-removal request is not proof that every retained copy expired.
DEFINE TABLE IF NOT EXISTS privacy_request TYPE NORMAL SCHEMAFULL PERMISSIONS NONE;
DEFINE FIELD IF NOT EXISTS app ON privacy_request TYPE string ASSERT $value = 'tawny';
DEFINE FIELD IF NOT EXISTS request_id ON privacy_request TYPE string;
DEFINE FIELD IF NOT EXISTS subject ON privacy_request TYPE string;
DEFINE FIELD IF NOT EXISTS subject_version ON privacy_request TYPE int ASSERT $value > 0;
DEFINE FIELD IF NOT EXISTS account ON privacy_request TYPE option<record<app_user>>;
DEFINE FIELD IF NOT EXISTS state ON privacy_request TYPE 'verified' | 'recovery_recorded' | 'primary_removed' | 'primary_verified';
DEFINE FIELD IF NOT EXISTS recovery_digest ON privacy_request TYPE option<string>;
DEFINE FIELD IF NOT EXISTS created_at ON privacy_request TYPE datetime DEFAULT time::now();
DEFINE FIELD IF NOT EXISTS updated_at ON privacy_request TYPE datetime DEFAULT time::now();
DEFINE FIELD IF NOT EXISTS primary_verified_at ON privacy_request TYPE option<datetime>;
DEFINE FIELD IF NOT EXISTS next_attempt_at ON privacy_request TYPE datetime DEFAULT time::now();
DEFINE FIELD IF NOT EXISTS attempt_count ON privacy_request TYPE int DEFAULT 0 ASSERT $value >= 0 AND $value <= 20;
DEFINE FIELD IF NOT EXISTS lease_owner ON privacy_request TYPE option<string>;
DEFINE FIELD IF NOT EXISTS lease_until ON privacy_request TYPE option<datetime>;
DEFINE FIELD IF NOT EXISTS last_error_code ON privacy_request TYPE option<'ledger_unavailable' | 'primary_pending' | 'session_pending' | 'review_required' | 'lease_lost' | 'invalid_record'>;
DEFINE INDEX IF NOT EXISTS privacy_request_id ON privacy_request FIELDS request_id UNIQUE;
DEFINE INDEX IF NOT EXISTS privacy_request_subject ON privacy_request FIELDS subject UNIQUE;
DEFINE INDEX IF NOT EXISTS privacy_request_account ON privacy_request FIELDS account;
DEFINE INDEX IF NOT EXISTS privacy_request_due ON privacy_request FIELDS state, next_attempt_at;
DEFINE FIELD IF NOT EXISTS privacy_version ON auth_session_subject TYPE option<int> DEFAULT 0 ASSERT $value = NONE OR $value >= 0;

-- Older in-flight deletion code must not erase a newly queued identity before
-- its independent event is acknowledged. The account touch serializes enqueue.
DEFINE EVENT IF NOT EXISTS privacy_delete_recording_v1 ON TABLE app_user WHEN $event = 'DELETE' THEN {
    IF array::len(SELECT VALUE id FROM privacy_request WHERE account = $before.id AND (state = 'verified' OR recovery_digest = NONE) LIMIT 1) != 0 {
        THROW 'Deletion is waiting for independent recovery recording';
    };
};

-- The ledger event's identity cannot change while a worker awaits its receipt.
DEFINE EVENT IF NOT EXISTS privacy_request_identity_v1 ON TABLE privacy_request WHEN $event = 'UPDATE' THEN {
    IF $before.app != $after.app OR $before.request_id != $after.request_id OR $before.subject != $after.subject OR $before.subject_version != $after.subject_version {
        THROW 'Recovery request identity cannot be changed';
    };
    IF $before.recovery_digest != NONE AND $after.recovery_digest != $before.recovery_digest { THROW 'Recovery acknowledgment cannot be replaced'; };
    IF $after.account != $before.account AND !($after.account = NONE AND $before.state = 'primary_removed' AND $after.state = 'primary_verified') { THROW 'Request account cannot be changed'; };
    IF ($before.state = 'verified' AND $after.state NOT IN ['verified', 'recovery_recorded'])
        OR ($before.state = 'recovery_recorded' AND $after.state NOT IN ['recovery_recorded', 'primary_removed'])
        OR ($before.state = 'primary_removed' AND $after.state NOT IN ['primary_removed', 'primary_verified'])
        OR ($before.state = 'primary_verified' AND $after.state != 'primary_verified') { THROW 'Invalid recovery checkpoint'; };
    IF $after.state != 'verified' AND $after.recovery_digest = NONE { THROW 'Recovery acknowledgment is required'; };
};
