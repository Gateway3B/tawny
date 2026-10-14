DEFINE TABLE IF NOT EXISTS channel TYPE NORMAL SCHEMAFULL PERMISSIONS NONE;

DEFINE FIELD IF NOT EXISTS channel_id ON channel TYPE string;
DEFINE FIELD IF NOT EXISTS name ON channel TYPE string;
DEFINE FIELD IF NOT EXISTS handle ON channel TYPE string;
DEFINE FIELD IF NOT EXISTS avatar_url ON channel TYPE option<string>;
DEFINE FIELD IF NOT EXISTS subscriber_count ON channel TYPE string DEFAULT "";
DEFINE FIELD IF NOT EXISTS subscriber_count_verified_at ON channel TYPE option<datetime>;
DEFINE FIELD IF NOT EXISTS subscribed ON channel TYPE bool DEFAULT false;
DEFINE FIELD IF NOT EXISTS subscription_content ON channel TYPE string DEFAULT "all";
DEFINE FIELD IF NOT EXISTS description ON channel TYPE string DEFAULT "";
DEFINE FIELD IF NOT EXISTS banner_url ON channel TYPE option<string>;
DEFINE FIELD IF NOT EXISTS last_polled_at ON channel TYPE option<datetime>;
DEFINE FIELD IF NOT EXISTS last_full_refresh_at ON channel TYPE option<datetime>;
DEFINE FIELD IF NOT EXISTS last_websub_at ON channel TYPE option<datetime>;
DEFINE FIELD IF NOT EXISTS websub_status ON channel TYPE option<string>;
DEFINE FIELD IF NOT EXISTS websub_requested_at ON channel TYPE option<datetime>;

DEFINE INDEX IF NOT EXISTS channel_external_id_idx ON channel FIELDS channel_id UNIQUE;
