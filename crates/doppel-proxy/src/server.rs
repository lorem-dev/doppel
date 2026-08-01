//! The proxy listener and the per-request pipeline.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::HeaderMap;
use axum::response::Response;
use axum::routing::any;
use doppel_core::RuntimeHolder;

use crate::fault::{OsSampler, Sampler, decide};
use crate::resolve::resolve;
use crate::upstream::{error_response, forward};

/// Everything a request handler needs. Cheap to clone.
#[derive(Clone)]
pub struct ProxyState {
    pub holder: Arc<RuntimeHolder>,
    pub sampler: Arc<dyn Sampler>,
}

impl ProxyState {
    #[must_use]
    pub fn new(holder: Arc<RuntimeHolder>) -> Self {
        Self {
            holder,
            sampler: Arc::new(OsSampler),
        }
    }
}

/// Every method and every path go to one handler: this is a proxy, not an app.
pub fn router(state: ProxyState) -> Router {
    Router::new().fallback(any(handle)).with_state(state)
}

/// Bind and serve until `shutdown` resolves.
pub async fn serve(
    state: ProxyState,
    listener: tokio::net::TcpListener,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    let app = router(state).into_make_service_with_connect_info::<SocketAddr>();
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await
}

/// Pipeline order is fixed by the spec: resolve, loss, latency, forward. Phase 2
/// inserts mock matching between latency and forwarding.
async fn handle(
    State(state): State<ProxyState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    request: Request,
) -> Response {
    let started = std::time::Instant::now();
    let runtime = state.holder.load();
    let request_id = request_id(request.headers());
    let method = request.method().clone();
    let path = request.uri().path().to_owned();

    let proxy = match resolve(&runtime, request.headers()) {
        Ok(proxy) => proxy,
        Err(err) => {
            let response = error_response(&err);
            tracing::info!(
                request_id,
                proxy = "",
                method = %method,
                path,
                status = response.status().as_u16(),
                duration_ms = started.elapsed().as_millis(),
                loss_injected = false,
                latency_injected_ms = 0u128,
                error_code = err.code.as_str(),
                "request rejected"
            );
            return response;
        }
    };

    let faults = decide(
        proxy.loss.as_ref(),
        proxy.latency.as_ref(),
        state.sampler.as_ref(),
    );

    if let Some(status) = faults.loss_status {
        let response = Response::builder()
            .status(status)
            .body(axum::body::Body::empty())
            .expect("status came from a validated config");
        tracing::info!(
            request_id,
            proxy = proxy.name,
            method = %method,
            path,
            status,
            duration_ms = started.elapsed().as_millis(),
            loss_injected = true,
            latency_injected_ms = 0u128,
            "request dropped"
        );
        return response;
    }

    let latency_ms = match faults.latency {
        Some(delay) => {
            tokio::time::sleep(delay).await;
            delay.as_millis()
        }
        None => 0,
    };

    match forward(&runtime.client, proxy, request, Some(peer.ip())).await {
        Ok((response, outcome)) => {
            tracing::info!(
                request_id,
                proxy = proxy.name,
                method = %method,
                path,
                status = response.status().as_u16(),
                duration_ms = started.elapsed().as_millis(),
                upstream_status = outcome.status,
                upstream_duration_ms = outcome.duration.as_millis(),
                loss_injected = false,
                latency_injected_ms = latency_ms,
                "request proxied"
            );
            response
        }
        Err(err) => {
            let response = error_response(&err);
            // `forward` validates the request path before it ever opens a
            // connection (see `join_upstream`), so a 4xx here means the
            // request was rejected on the way in, not that the upstream
            // misbehaved or was unreachable. Log that distinction: a client
            // mistake is not an operational upstream failure, and paging
            // someone for it would be wrong. Every field below still mirrors
            // the success and loss branches so a log consumer sees one shape.
            if err.status() < 500 {
                tracing::info!(
                    request_id,
                    proxy = proxy.name,
                    method = %method,
                    path,
                    status = response.status().as_u16(),
                    duration_ms = started.elapsed().as_millis(),
                    loss_injected = false,
                    latency_injected_ms = latency_ms,
                    error_code = err.code.as_str(),
                    "request rejected"
                );
            } else {
                tracing::warn!(
                    request_id,
                    proxy = proxy.name,
                    method = %method,
                    path,
                    status = response.status().as_u16(),
                    duration_ms = started.elapsed().as_millis(),
                    loss_injected = false,
                    latency_injected_ms = latency_ms,
                    error_code = err.code.as_str(),
                    "upstream failed"
                );
            }
            response
        }
    }
}

/// Reuse an incoming `X-Request-ID` so one request can be followed across
/// services; generate one otherwise. A header that is not valid ASCII is
/// replaced rather than propagated, since it cannot be logged faithfully.
#[must_use]
pub fn request_id(headers: &HeaderMap) -> String {
    headers
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| format!("{:032x}", rand::random::<u128>()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use doppel_core::config::load_from_str;
    use doppel_core::store::Revision;
    use doppel_core::{Runtime, RuntimeHolder};
    use std::sync::Arc;
    use std::time::{Duration, Instant};
    use tower::ServiceExt;

    /// The upstream points at loopback port 1, where nothing listens. Any test
    /// that passes without a real upstream therefore proves the upstream was
    /// never contacted.
    fn config_with(extra: &str) -> String {
        format!(
            r#"
server:
  host: "127.0.0.1"
  port: 8080
admin:
  host: "127.0.0.1"
  port: 8081
  tokens: []
  access: {{}}
  upload:
    limit: 1M
proxies:
  - name: p1
    type: http
    url: "http://127.0.0.1:1/"
    resolve:
      type: default
{extra}
"#
        )
    }

    fn state(text: &str, samples: Vec<f64>) -> ProxyState {
        let config = Arc::new(load_from_str(text).unwrap());
        let runtime = Runtime::compile(config, Revision(1)).unwrap();
        ProxyState {
            holder: Arc::new(RuntimeHolder::new(runtime)),
            sampler: Arc::new(crate::fault::SequenceSampler::new(samples)),
        }
    }

    async fn send(state: ProxyState, request: Request<Body>) -> axum::response::Response {
        router(state).oneshot(request).await.unwrap()
    }

    /// `oneshot` bypasses `axum::serve`, so nothing populates connect info and
    /// the `ConnectInfo` extractor in the handler would fail with a 500. Tests
    /// therefore insert it themselves.
    fn get(uri: &str) -> Request<Body> {
        let mut request = Request::builder().uri(uri).body(Body::empty()).unwrap();
        request.extensions_mut().insert(axum::extract::ConnectInfo(
            "127.0.0.1:54321".parse::<std::net::SocketAddr>().unwrap(),
        ));
        request
    }

    async fn json_body(response: axum::response::Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn loss_returns_the_configured_status_without_contacting_the_upstream() {
        let text = config_with("    loss:\n      percentage: 1.0\n      status: 503");
        let response = send(state(&text, vec![0.0]), get("/anything")).await;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn loss_that_does_not_fire_falls_through_to_the_upstream() {
        let text = config_with("    loss:\n      percentage: 0.5\n      status: 503");
        // Draw above the threshold, so loss does not fire and the dead upstream
        // is contacted, producing 502.
        let response = send(state(&text, vec![0.9]), get("/anything")).await;
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    }

    #[tokio::test]
    async fn latency_delays_the_response_by_at_least_the_minimum() {
        let text =
            config_with("    latency:\n      percentage: 1.0\n      min: 0.15\n      max: 0.15");
        let started = Instant::now();
        let response = send(state(&text, vec![0.0, 0.0]), get("/anything")).await;
        assert!(
            started.elapsed() >= Duration::from_millis(150),
            "{:?}",
            started.elapsed()
        );
        // Still reaches the dead upstream afterwards.
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    }

    #[tokio::test]
    async fn unresolvable_request_returns_the_error_envelope() {
        let text = config_with("").replace(
            "      type: default",
            "      type: header\n      header: X-Proxy-Name",
        );
        let response = send(state(&text, vec![]), get("/anything")).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            response.headers().get("content-type").unwrap(),
            "application/json"
        );
        let body = json_body(response).await;
        assert_eq!(body["status"], "error");
        assert_eq!(body["code"], "PROXY_NOT_RESOLVED");
        assert!(
            body["message"]
                .as_str()
                .unwrap()
                .contains("no default proxy")
        );
    }

    #[tokio::test]
    async fn upstream_failure_returns_the_error_envelope() {
        let response = send(state(&config_with(""), vec![]), get("/anything")).await;
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        let body = json_body(response).await;
        assert_eq!(body["code"], "UPSTREAM_ERROR");
    }

    // -- Log level distinction: client rejection vs. genuine upstream failure --
    //
    // A minimal `tracing::Subscriber` recording only level and message, set as
    // the *thread-local* default via `set_default` (not `set_global_default`).
    // `#[tokio::test]` runs on a single-threaded current-thread runtime, so the
    // subscriber stays active across every await point in the request under
    // test without ever touching global state another test could observe.

    struct RecordMessage(String);

    impl tracing::field::Visit for RecordMessage {
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            if field.name() == "message" {
                self.0 = format!("{value:?}");
            }
        }
    }

    struct Capture {
        events: Arc<std::sync::Mutex<Vec<(tracing::Level, String)>>>,
    }

    impl tracing::Subscriber for Capture {
        fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
            // Limit to this crate's own events; reqwest/hyper emit their own
            // trace/debug noise while reaching the dead upstream, which is
            // not what this test is about.
            metadata.target().starts_with("doppel_proxy")
        }

        fn new_span(&self, _span: &tracing::span::Attributes<'_>) -> tracing::span::Id {
            tracing::span::Id::from_u64(1)
        }

        fn record(&self, _span: &tracing::span::Id, _values: &tracing::span::Record<'_>) {}

        fn record_follows_from(&self, _span: &tracing::span::Id, _follows: &tracing::span::Id) {}

        fn event(&self, event: &tracing::Event<'_>) {
            let mut visitor = RecordMessage(String::new());
            event.record(&mut visitor);
            self.events
                .lock()
                .unwrap()
                .push((*event.metadata().level(), visitor.0));
        }

        fn enter(&self, _span: &tracing::span::Id) {}

        fn exit(&self, _span: &tracing::span::Id) {}
    }

    async fn run_captured(
        state: ProxyState,
        request: Request<Body>,
    ) -> (axum::response::Response, Vec<(tracing::Level, String)>) {
        let events = Arc::new(std::sync::Mutex::new(Vec::new()));
        let capture = Capture {
            events: events.clone(),
        };
        let guard = tracing::subscriber::set_default(capture);
        let response = send(state, request).await;
        drop(guard);
        let events = events.lock().unwrap().clone();
        (response, events)
    }

    #[tokio::test]
    async fn a_client_rejected_path_is_logged_at_info_not_warn() {
        // `/../secret` fails `join_upstream`'s path validation before any
        // connection is attempted: a 400, never contacting the dead upstream.
        let (response, events) =
            run_captured(state(&config_with(""), vec![]), get("/../secret")).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            events.len(),
            1,
            "expected exactly one log line, got {events:?}"
        );
        let (level, message) = &events[0];
        assert_eq!(*level, tracing::Level::INFO, "got {events:?}");
        assert!(message.contains("rejected"), "got {events:?}");
    }

    #[tokio::test]
    async fn a_genuine_upstream_failure_is_logged_at_warn_not_info() {
        // Connection to loopback port 1 is refused: the upstream really was
        // contacted and really did fail, unlike the client-rejection case above.
        let (response, events) =
            run_captured(state(&config_with(""), vec![]), get("/anything")).await;
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        assert_eq!(
            events.len(),
            1,
            "expected exactly one log line, got {events:?}"
        );
        let (level, message) = &events[0];
        assert_eq!(*level, tracing::Level::WARN, "got {events:?}");
        assert!(message.contains("failed"), "got {events:?}");
    }

    #[test]
    fn request_id_prefers_the_incoming_header() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert("x-request-id", "abc-123".parse().unwrap());
        assert_eq!(request_id(&headers), "abc-123");
    }

    #[test]
    fn request_id_is_generated_when_absent() {
        let generated = request_id(&axum::http::HeaderMap::new());
        assert_eq!(
            generated.len(),
            32,
            "expected a 128 bit hex id, got `{generated}`"
        );
        assert!(generated.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(generated, request_id(&axum::http::HeaderMap::new()));
    }

    #[test]
    fn a_non_ascii_request_id_is_replaced_rather_than_propagated() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            "x-request-id",
            axum::http::HeaderValue::from_bytes(&[0xff, 0xfe]).unwrap(),
        );
        assert_eq!(request_id(&headers).len(), 32);
    }
}
