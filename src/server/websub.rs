use super::*;

impl AppServerState {
    pub(crate) async fn request_websub_subscription(
        &self,
        channel_id: &str,
        subscribe: bool,
    ) -> Result<bool> {
        let Some(callback) = self.websub_callback_url.as_deref() else {
            return Ok(false);
        };
        if !channel_id.starts_with("UC") || channel_id.starts_with("UC-tawny") {
            return Ok(false);
        }
        let topic = format!("https://www.youtube.com/xml/feeds/videos.xml?channel_id={channel_id}");
        let mode = if subscribe {
            "subscribe"
        } else {
            "unsubscribe"
        };
        let form = [
            ("hub.callback", callback.to_string()),
            ("hub.topic", topic),
            ("hub.verify", "async".to_string()),
            ("hub.mode", mode.to_string()),
            ("hub.verify_token", self.websub_secret.clone()),
            ("hub.secret", self.websub_secret.clone()),
            ("hub.lease_seconds", (10 * 24 * 60 * 60).to_string()),
        ];
        self.http
            .post("https://pubsubhubbub.appspot.com/subscribe")
            .form(&form)
            .send()
            .await
            .context("request YouTube WebSub subscription")?
            .error_for_status()
            .context("YouTube WebSub hub rejected subscription")?;
        self.db
            .query(
                "UPDATE channel SET websub_status = $status, websub_requested_at = time::now() WHERE channel_id = $channel_id",
            )
            .bind(("channel_id", channel_id.to_string()))
            .bind(("status", format!("{mode}-requested")))
            .await?
            .check()?;
        Ok(true)
    }

    pub(crate) async fn renew_websub_subscriptions(&self) -> Result<usize> {
        use futures_util::{StreamExt, stream};

        let channels: Vec<DbSubscriptionState> = self
            .db
            .query(
                "SELECT channel_id, subscribed FROM channel WHERE subscribed = true AND (websub_requested_at = NONE OR websub_requested_at < time::now() - 3d)",
            )
            .await?
            .take(0)?;
        let results = stream::iter(channels.into_iter().map(|channel| async move {
            self.request_websub_subscription(&channel.channel_id, true)
                .await
        }))
        .buffer_unordered(WEBSUB_RENEW_CONCURRENCY)
        .collect::<Vec<_>>()
        .await;
        Ok(results
            .into_iter()
            .filter(|result| result.as_ref().is_ok_and(|requested| *requested))
            .count())
    }

    pub(crate) async fn ingest_websub_notification(&self, xml: &str) -> Result<usize> {
        let entries = parse_youtube_feed(xml)?;
        let Some(channel_id) = entries
            .iter()
            .find_map(|entry| (!entry.channel_id.is_empty()).then(|| entry.channel_id.clone()))
        else {
            return Ok(0);
        };
        if !self.channel_is_subscribed(&channel_id).await? {
            return Ok(0);
        }
        // Catalog only: a WebSub push belongs to the instance, not to a viewer,
        // and all it needs from here is the channel's cached metadata.
        let channel = self
            .catalog_channel(&channel_id)
            .await?
            .ok_or_else(|| anyhow!("WebSub channel is not cached"))?;

        let mut hints = Vec::with_capacity(entries.len());
        for entry in &entries {
            let video = feed_entry_video(entry, &channel);
            self.upsert_feed_hint(&video, entry.published.clone())
                .await?;
            hints.push(video);
        }
        self.db
            .query(
                "UPDATE channel SET last_websub_at = time::now(), last_polled_at = time::now() WHERE channel_id = $channel_id",
            )
            .bind(("channel_id", channel.id))
            .await?
            .check()?;

        use futures_util::{StreamExt, stream};
        let _ = stream::iter(hints.into_iter().map(|video| async move {
            let enriched = self.enrich_video_player(video).await;
            self.upsert_feed_video(&enriched, video_sort_key(&enriched))
                .await
        }))
        .buffer_unordered(4)
        .collect::<Vec<_>>()
        .await;
        Ok(entries.len())
    }

    pub async fn poll_subscriptions_once(&self) -> Result<usize> {
        // The poller has no viewer; it only wants the count.
        Ok(self.refresh_feed("").await?.imported)
    }
}

#[derive(Debug, Deserialize)]
pub struct WebSubVerification {
    #[serde(rename = "hub.mode")]
    pub(crate) mode: String,
    #[serde(rename = "hub.topic")]
    pub(crate) topic: String,
    #[serde(rename = "hub.challenge")]
    pub(crate) challenge: String,
    #[serde(rename = "hub.verify_token")]
    pub(crate) verify_token: Option<String>,
    #[serde(rename = "hub.lease_seconds")]
    pub(crate) lease_seconds: Option<u64>,
}

pub(crate) fn websub_channel_from_topic(topic: &str) -> Option<String> {
    let url = reqwest::Url::parse(topic).ok()?;
    if url.host_str()? != "www.youtube.com" || url.path() != "/xml/feeds/videos.xml" {
        return None;
    }
    url.query_pairs()
        .find_map(|(key, value)| (key == "channel_id").then(|| value.into_owned()))
        .filter(|id| id.starts_with("UC"))
}

pub async fn youtube_websub_verify(
    Extension(state): Extension<AppServerState>,
    Query(verification): Query<WebSubVerification>,
) -> Response {
    if verification.mode != "subscribe" && verification.mode != "unsubscribe" {
        return proxy_error(StatusCode::BAD_REQUEST, "Unsupported WebSub mode");
    }
    if verification
        .verify_token
        .as_deref()
        .is_some_and(|token| token != state.websub_secret)
    {
        return proxy_error(StatusCode::FORBIDDEN, "Invalid WebSub verification token");
    }
    let Some(channel_id) = websub_channel_from_topic(&verification.topic) else {
        return proxy_error(StatusCode::BAD_REQUEST, "Invalid YouTube WebSub topic");
    };
    let subscribed = state
        .channel_is_subscribed(&channel_id)
        .await
        .unwrap_or(false);
    if verification.mode == "subscribe" && !subscribed {
        return proxy_error(StatusCode::NOT_FOUND, "Channel is not subscribed");
    }
    let status = if verification.mode == "subscribe" {
        format!("active:{}s", verification.lease_seconds.unwrap_or_default())
    } else {
        "inactive".into()
    };
    let _ = state
        .db
        .query("UPDATE channel SET websub_status = $status WHERE channel_id = $channel_id")
        .bind(("channel_id", channel_id))
        .bind(("status", status))
        .await;
    let mut response = Response::new(Body::from(verification.challenge));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    response
}

pub async fn youtube_websub_notification(
    Extension(state): Extension<AppServerState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let signature = headers
        .get("x-hub-signature")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("sha1="))
        .and_then(|value| hex::decode(value).ok());
    let Some(signature) = signature else {
        return proxy_error(StatusCode::UNAUTHORIZED, "Missing WebSub signature");
    };
    let Ok(mut mac) = Hmac::<Sha1>::new_from_slice(state.websub_secret.as_bytes()) else {
        return proxy_error(StatusCode::INTERNAL_SERVER_ERROR, "Invalid WebSub secret");
    };
    mac.update(&body);
    if mac.verify_slice(&signature).is_err() {
        return proxy_error(StatusCode::UNAUTHORIZED, "Invalid WebSub signature");
    }
    let Ok(xml) = String::from_utf8(body.to_vec()) else {
        return proxy_error(StatusCode::BAD_REQUEST, "WebSub payload is not UTF-8");
    };
    tokio::spawn(async move {
        if state.ingest_websub_notification(&xml).await.is_err() {
            crate::telemetry::diagnostic(crate::telemetry::Diagnostic::WebsubIngestionFailed);
        }
    });

    let mut response = Response::new(Body::empty());
    *response.status_mut() = StatusCode::NO_CONTENT;
    response
}

pub fn spawn_subscription_poller(state: AppServerState) {
    tokio::spawn(async move {
        let renewal_state = state.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(3 * 24 * 60 * 60));
            loop {
                interval.tick().await;
                if renewal_state.renew_websub_subscriptions().await.is_err() {
                    crate::telemetry::diagnostic(crate::telemetry::Diagnostic::WebsubRenewalFailed);
                }
            }
        });

        let mut interval = tokio::time::interval(Duration::from_secs(15 * 60));
        loop {
            interval.tick().await;
            // Also catches channels imported before the import path learned to
            // do this, a batch per tick so a large library does not arrive at
            // YouTube as one burst.
            state
                .backfill_channel_metadata(None, CHANNEL_METADATA_BATCH)
                .await;
            if state.poll_subscriptions_once().await.is_err() {
                crate::telemetry::diagnostic(crate::telemetry::Diagnostic::SubscriptionPollFailed);
            }
        }
    });
}
