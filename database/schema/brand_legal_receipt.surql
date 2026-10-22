-- Private, per-account evidence. Public catalog storage never contains this.
-- No consent purposes are enabled by this notice/terms pilot.
DEFINE TABLE IF NOT EXISTS brand_legal_receipt TYPE NORMAL SCHEMAFULL PERMISSIONS NONE;
DEFINE FIELD IF NOT EXISTS app ON brand_legal_receipt TYPE string ASSERT $value = 'tawny';
DEFINE FIELD IF NOT EXISTS owner ON brand_legal_receipt TYPE record<app_user>;
DEFINE FIELD IF NOT EXISTS subject ON brand_legal_receipt TYPE string;
DEFINE FIELD IF NOT EXISTS document_set_digest ON brand_legal_receipt TYPE string;
DEFINE FIELD IF NOT EXISTS action ON brand_legal_receipt TYPE string ASSERT $value IN ['acknowledged', 'terms_accepted'];
DEFINE FIELD IF NOT EXISTS purpose ON brand_legal_receipt TYPE string ASSERT $value = '';
DEFINE FIELD IF NOT EXISTS created_at ON brand_legal_receipt TYPE datetime;
DEFINE INDEX IF NOT EXISTS receipt_subject ON brand_legal_receipt FIELDS app, subject, document_set_digest, action;
DEFINE INDEX IF NOT EXISTS receipt_owner ON brand_legal_receipt FIELDS owner;
