//! Runtime telemetry accepts only the fields deliberately emitted here. No
//! dependency events, surrounding span fields or environment-selected verbosity.
use axum::{
    extract::{MatchedPath, Request},
    middleware::Next,
    response::Response,
};
use std::time::Instant;
use tracing_subscriber::{EnvFilter, fmt::MakeWriter};

const TARGET: &str = "tawny::telemetry";

fn subscriber<W>(writer: W) -> impl tracing::Subscriber + Send + Sync
where
    W: for<'writer> MakeWriter<'writer> + Send + Sync + 'static,
{
    tracing_subscriber::fmt()
        .json()
        .with_writer(writer)
        // A caller-controlled RUST_LOG must not turn SDK payloads or database
        // diagnostics into retained personal data. Debug with synthetic local
        // fixtures when richer diagnostics are needed.
        .with_env_filter(EnvFilter::new("off,tawny::telemetry=info"))
        .with_current_span(false)
        .with_span_list(false)
        .with_target(false)
        .with_ansi(false)
        .finish()
}

pub(crate) fn init() {
    tracing::subscriber::set_global_default(subscriber(std::io::stderr))
        .unwrap_or_else(|_| panic!("Failed to install the runtime logger."));
    // Default panic payloads may contain provider responses or query values.
    std::panic::set_hook(Box::new(|_| diagnostic(Diagnostic::RuntimePanic)));
}

fn completed(route: &str, status: u16, duration_ms: u64) {
    tracing::info!(target: TARGET, route, status, duration_ms, "request_complete");
}

pub(crate) async fn request_complete(request: Request, next: Next) -> Response {
    // MatchedPath is the router's source-defined pattern, never the requested
    // URL. In particular it cannot include query strings or route parameters.
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map(|value| value.as_str().to_owned())
        .unwrap_or_else(|| "unmatched".into());
    let started = Instant::now();
    let response = next.run(request).await;
    completed(
        &route,
        response.status().as_u16(),
        started.elapsed().as_millis().min(u64::MAX as u128) as u64,
    );
    response
}

/// Diagnostics cannot carry account IDs, viewing choices, URLs or raw errors.
#[derive(Clone, Copy)]
pub(crate) enum Diagnostic {
    DatabaseUnavailable,
    BotguardUnavailable,
    PoProviderRejected,
    PoProviderUnavailable,
    PoProviderTimeout,
    ChannelInterestUnavailable,
    PlaybackTailRetry,
    PlaybackTracksResolved,
    PlaybackHlsFallback,
    PlaybackExtractionFailed,
    PlaybackUnavailable,
    MetadataListUnavailable,
    MetadataExtractFailed,
    MetadataCountFailed,
    MetadataStoreFailed,
    WebsubIngestionFailed,
    WebsubRenewalFailed,
    WebsubSecretUnsaved,
    SubscriptionPollFailed,
    ExtractorBinaryUnavailable,
    ExtractorConfigurationInvalid,
    ExtractorShortsFailed,
    ExtractorFormatsFailed,
    /// A server function failed with an internal error; the client was told
    /// only that something went wrong.
    ServerFunctionFailed,
    /// Best effort, retried by the poller: a new follow's first fetch.
    FirstFetchFailed,
    /// Best effort: storing a video a WebSub notification announced.
    WebsubHintFailed,
    /// Best effort: recording a WebSub lease the hub just verified.
    WebsubStatusUnsaved,
    RuntimePanic,
}

impl Diagnostic {
    fn code(self) -> &'static str {
        match self {
            Self::DatabaseUnavailable => "database_unavailable",
            Self::BotguardUnavailable => "botguard_unavailable",
            Self::PoProviderRejected => "po_provider_rejected",
            Self::PoProviderUnavailable => "po_provider_unavailable",
            Self::PoProviderTimeout => "po_provider_timeout",
            Self::ChannelInterestUnavailable => "channel_interest_unavailable",
            Self::PlaybackTailRetry => "playback_tail_retry",
            Self::PlaybackTracksResolved => "playback_tracks_resolved",
            Self::PlaybackHlsFallback => "playback_hls_fallback",
            Self::PlaybackExtractionFailed => "playback_extraction_failed",
            Self::PlaybackUnavailable => "playback_unavailable",
            Self::MetadataListUnavailable => "metadata_list_unavailable",
            Self::MetadataExtractFailed => "metadata_extract_failed",
            Self::MetadataCountFailed => "metadata_count_failed",
            Self::MetadataStoreFailed => "metadata_store_failed",
            Self::WebsubIngestionFailed => "websub_ingestion_failed",
            Self::WebsubRenewalFailed => "websub_renewal_failed",
            Self::WebsubSecretUnsaved => "websub_secret_unsaved",
            Self::SubscriptionPollFailed => "subscription_poll_failed",
            Self::ExtractorBinaryUnavailable => "extractor_binary_unavailable",
            Self::ExtractorConfigurationInvalid => "extractor_configuration_invalid",
            Self::ExtractorShortsFailed => "extractor_shorts_failed",
            Self::ExtractorFormatsFailed => "extractor_formats_failed",
            Self::ServerFunctionFailed => "server_function_failed",
            Self::FirstFetchFailed => "first_fetch_failed",
            Self::WebsubHintFailed => "websub_hint_failed",
            Self::WebsubStatusUnsaved => "websub_status_unsaved",
            Self::RuntimePanic => "runtime_panic",
        }
    }
}

pub(crate) fn diagnostic(code: Diagnostic) {
    tracing::info!(target: TARGET, code = code.code(), "service_event");
}

/// How many of a batch of best-effort jobs failed, when any did.
///
/// The jobs' results used to be dropped with `let _ =`, which hid a run of
/// failures entirely. A count and a code, never the errors. At info, not
/// debug: the subscriber above keeps only info and up from this target.
pub(crate) fn best_effort_failures(code: Diagnostic, failed: usize) {
    if failed > 0 {
        tracing::info!(target: TARGET, code = code.code(), failed, "best_effort_failed");
    }
}

pub(crate) fn playback_resolved(
    sources: usize,
    extract: std::time::Duration,
    ranges: std::time::Duration,
    verify: std::time::Duration,
    total: std::time::Duration,
) {
    fn millis(value: std::time::Duration) -> u64 {
        value.as_millis().min(u64::MAX as u128) as u64
    }
    tracing::info!(target: TARGET, sources, extract_ms = millis(extract), ranges_ms = millis(ranges),
        verify_ms = millis(verify), total_ms = millis(total), "playback_resolved");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::Write,
        sync::{Arc, Mutex},
        time::Duration,
    };

    #[derive(Clone, Default)]
    struct Capture(Arc<Mutex<Vec<u8>>>);

    impl Write for Capture {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'writer> MakeWriter<'writer> for Capture {
        type Writer = Capture;

        fn make_writer(&'writer self) -> Self::Writer {
            self.clone()
        }
    }

    #[test]
    fn service_diagnostics_and_playback_timing_have_a_small_fixed_field_set() {
        let capture = Capture::default();
        tracing::subscriber::with_default(subscriber(capture.clone()), || {
            diagnostic(Diagnostic::PlaybackExtractionFailed);
            diagnostic(Diagnostic::RuntimePanic);
            best_effort_failures(Diagnostic::FirstFetchFailed, 0);
            best_effort_failures(Diagnostic::FirstFetchFailed, 2);
            playback_resolved(
                2,
                Duration::from_millis(10),
                Duration::from_millis(20),
                Duration::from_millis(30),
                Duration::from_millis(40),
            );
        });
        let output = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
        let fields = output
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap()["fields"].clone())
            .collect::<Vec<_>>();
        assert_eq!(
            fields,
            vec![
                serde_json::json!({"message":"service_event", "code":"playback_extraction_failed"}),
                serde_json::json!({"message":"service_event", "code":"runtime_panic"}),
                serde_json::json!({"message":"best_effort_failed", "code":"first_fetch_failed", "failed":2}),
                serde_json::json!({"message":"playback_resolved", "sources":2, "extract_ms":10, "ranges_ms":20, "verify_ms":30, "total_ms":40}),
            ]
        );
    }

    #[test]
    fn sdk_events_and_surrounding_private_span_fields_never_enter_telemetry() {
        let capture = Capture::default();
        tracing::subscriber::with_default(subscriber(capture.clone()), || {
            tracing::error!(target: "surrealdb", query = "query-canary", "error-canary");
            tracing::warn!(target: "reqwest", token = "token-canary", "provider-canary");
            let span = tracing::info_span!(target: TARGET, "private-canary", user = "user-canary", cookie = "cookie-canary");
            let _entered = span.enter();
            completed("/api/v1/profile/{id}", 200, 12);
        });
        let output = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
        assert!(!output.contains("canary"), "{output}");
        let lines = output.lines().collect::<Vec<_>>();
        assert_eq!(lines.len(), 1);
        let event: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(
            event["fields"],
            serde_json::json!({"message":"request_complete", "route":"/api/v1/profile/{id}", "status":200, "duration_ms":12})
        );
        assert!(event.get("span").is_none());
        assert!(event.get("spans").is_none());
    }

    #[tokio::test]
    async fn request_events_use_patterns_without_parameter_query_or_header_values() {
        use axum::{Router, middleware::from_fn, routing::get};
        use tracing::instrument::WithSubscriber;
        let capture = Capture::default();
        let dispatch = tracing::Dispatch::new(subscriber(capture.clone()));
        let app = Router::new()
            .route("/api/v1/profile/{id}", get(|| async { "ok" }))
            .layer(from_fn(move |request: Request, next: Next| {
                request_complete(request, next).with_subscriber(dispatch.clone())
            }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut connection = tokio::net::TcpStream::connect(address).await.unwrap();
        connection.write_all(format!("GET /api/v1/profile/person-canary?email=email-canary HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer token-canary\r\nCookie: session=cookie-canary\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
        let mut response = Vec::new();
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            connection.read_to_end(&mut response),
        )
        .await
        .unwrap()
        .unwrap();
        let response = String::from_utf8(response).unwrap();
        assert!(response.starts_with("HTTP/1.1 200"));
        assert!(response.ends_with("ok"));
        task.abort();
        let output = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
        assert!(!output.contains("canary"), "{output}");
        let event: serde_json::Value = serde_json::from_str(output.trim()).unwrap();
        assert_eq!(event["fields"]["route"], "/api/v1/profile/{id}");
        assert_eq!(event["fields"]["status"], 200);
    }
}
