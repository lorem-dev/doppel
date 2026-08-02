//! The proxy listener and the per-request pipeline.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::HeaderMap;
use axum::response::Response;
use axum::routing::any;
use doppel_core::{CompiledMock, CompiledProxy, Error, ErrorCode, MockBody, RuntimeHolder};
use doppel_render::{Renderer, Variables};

use crate::fault::{OsSampler, Sampler, decide, fires};
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
    // `started.elapsed()` below, wherever it is read, is read once this
    // function has a `Response` value to hand back to axum -- which wraps
    // the response body as a still-unread stream -- not once that stream has
    // actually finished writing to the client. Every `duration_ms` this
    // handler logs therefore stops at the same point `UpstreamOutcome`'s
    // duration does (see its doc comment in `upstream.rs`), and for the same
    // reason.
    let started = std::time::Instant::now();
    let runtime = state.holder.load();
    let request_id = request_id(request.headers());
    let method = request.method().clone();
    let path = request.uri().path().to_owned();

    let proxy = match resolve(&runtime, request.headers()) {
        Ok(proxy) => proxy,
        Err(err) => {
            let response = with_request_id(error_response(&err), &request_id);
            // `upstream_contacted` is a bool, not an `Option`-typed null: the
            // `tracing::Value` impl for `Option<T>` records nothing at all
            // when the value is `None`, so the JSON layer omits a null field
            // rather than emitting `null` for it -- verified against this
            // workspace's pinned `tracing-subscriber` 0.3.23, which never
            // shows the key when `None` is recorded. A null-based scheme
            // therefore cannot express "no upstream was involved" in the
            // rendered log line at all. A boolean is also simply the better
            // design here regardless of that library quirk: a null status
            // still invites a consumer to plot it, where a boolean says what
            // actually happened. `upstream_status` and `upstream_duration_ms`
            // are emitted only when `upstream_contacted` is true, which is
            // also the only case where `forward` actually produced them.
            tracing::info!(
                request_id,
                proxy = "",
                method = %method,
                path,
                status = response.status().as_u16(),
                duration_ms = started.elapsed().as_millis(),
                upstream_contacted = false,
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
        let response = with_request_id(
            Response::builder()
                .status(status)
                .body(axum::body::Body::empty())
                .expect("status came from a validated config"),
            &request_id,
        );
        tracing::info!(
            request_id,
            proxy = proxy.name,
            method = %method,
            path,
            status,
            duration_ms = started.elapsed().as_millis(),
            upstream_contacted = false,
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

    // Mock matching sits here, between latency and forwarding, because the
    // spec fixes that order: faults are a property of the proxy and apply
    // before an endpoint is chosen, while a mock replaces the endpoint.
    //
    // The `replace` roll is what makes a matched mock optional -- a proxy can
    // serve a mock some of the time and the real backend the rest -- so a
    // mock that matches but loses the roll falls through to `forward` below
    // with its request untouched. That is why `serve_mock`, which consumes
    // the request, is only reached inside the winning branch.
    if let Some((mock, vars)) = crate::mock::match_mock(proxy, &method, &path) {
        let replace = mock.replace.unwrap_or(proxy.replace);
        if fires(replace, state.sampler.as_ref()) {
            let outcome =
                serve_mock(&runtime.config.templates.dir, proxy, mock, vars, request).await;
            let (response, error_code) = match outcome {
                Ok(response) => (response, None),
                Err(err) => (error_response(&err), Some(err.code.as_str())),
            };
            let response = with_request_id(response, &request_id);
            // `upstream_contacted` is false here and the two upstream fields
            // are absent: a served mock never reaches the upstream, which is
            // precisely the distinction that field exists to record.
            tracing::info!(
                request_id,
                proxy = proxy.name,
                mock = mock.name,
                method = %method,
                path,
                status = response.status().as_u16(),
                duration_ms = started.elapsed().as_millis(),
                upstream_contacted = false,
                loss_injected = false,
                latency_injected_ms = latency_ms,
                error_code,
                "request mocked"
            );
            return response;
        }
    }

    match forward(
        &runtime.client,
        proxy,
        request,
        Some(peer.ip()),
        &runtime.resolve_headers,
        &request_id,
    )
    .await
    {
        Ok((response, outcome)) => {
            tracing::info!(
                request_id,
                proxy = proxy.name,
                method = %method,
                path,
                status = response.status().as_u16(),
                duration_ms = started.elapsed().as_millis(),
                upstream_contacted = true,
                upstream_status = outcome.status,
                upstream_duration_ms = outcome.duration.as_millis(),
                loss_injected = false,
                latency_injected_ms = latency_ms,
                "request proxied"
            );
            response
        }
        Err(err) => {
            let response = with_request_id(error_response(&err), &request_id);
            // `forward` validates the request path before it ever opens a
            // connection (see `join_upstream`), so a 4xx here (always
            // `InvalidRequestPath` today) means the request was rejected on
            // the way in: `upstream_contacted` is false, and there is no
            // status or duration to report, because `forward` returned
            // before ever building an `UpstreamOutcome`. A 5xx here
            // (`UpstreamTimeout`/`UpstreamError`) means a connection really
            // was attempted and failed -- `upstream_contacted` is true, but
            // there is still no status or duration to report, since a failed
            // attempt never produced an `UpstreamOutcome` either. A client
            // mistake is not an operational upstream failure, and paging
            // someone for it would be wrong, so the two cases log at
            // different levels; `err.status() >= 500` is the same test
            // already used to pick the level, so it is reused rather than
            // re-derived.
            //
            // `tracing::event!` requires its level to be a compile-time
            // constant -- each callsite bakes one fixed `Level` into its
            // static metadata for the fast filtering path -- so a runtime
            // `let level = if .. { Level::INFO } else { Level::WARN }` does
            // not compile. This local macro keeps the field list written
            // exactly once while still letting each arm supply its own
            // literal level and message, so the next field added here only
            // has to be added once.
            macro_rules! log_forward_error {
                ($level:expr, $message:literal) => {
                    tracing::event!(
                        $level,
                        request_id,
                        proxy = proxy.name,
                        method = %method,
                        path,
                        status = response.status().as_u16(),
                        duration_ms = started.elapsed().as_millis(),
                        upstream_contacted = err.status() >= 500,
                        loss_injected = false,
                        latency_injected_ms = latency_ms,
                        error_code = err.code.as_str(),
                        $message
                    )
                };
            }
            if err.status() < 500 {
                log_forward_error!(tracing::Level::INFO, "request rejected");
            } else {
                log_forward_error!(tracing::Level::WARN, "upstream failed");
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

/// Set `X-Request-ID` on every response the client receives, not just a
/// successfully forwarded one -- otherwise the id logged for a rejected or
/// dropped request would never reach the client that could quote it back.
/// `forward`'s own success path already sets this on its way back from the
/// upstream; this covers the branches that never call `forward` at all.
fn with_request_id(mut response: Response, request_id: &str) -> Response {
    if let Ok(value) = axum::http::HeaderValue::from_str(request_id) {
        response.headers_mut().insert("x-request-id", value);
    }
    response
}

/// Renders a matched, roll-winning mock into its response. Order, per spec
/// section 3 and the task brief: bind headers, then query, then body (only
/// if the mock declares body selectors, buffering no more than
/// `proxy.body_limit` -- phase 1's streaming stays untouched for every other
/// mock and every unmatched request); read the template file if the body is
/// `MockBody::Template`; render the body and every response header; return
/// the mock's status.
async fn serve_mock(
    templates_dir: &std::path::Path,
    proxy: &CompiledProxy,
    mock: &CompiledMock,
    mut vars: Variables,
    request: Request,
) -> Result<Response, Error> {
    crate::mock::bind_headers(mock, request.headers(), &mut vars);
    crate::mock::bind_query(mock, request.uri().query(), &mut vars)?;

    if !mock.body_vars.is_empty() {
        // Buffering is deferred to exactly here: only a mock that declares a
        // body selector ever causes the request body to be read into memory
        // at all (spec section 6 and section 10).
        let limit = usize::try_from(proxy.body_limit).unwrap_or(usize::MAX);
        let bytes = axum::body::to_bytes(request.into_body(), limit)
            .await
            .map_err(|err| {
                Error::new(
                    ErrorCode::UploadTooLarge,
                    format!(
                        "request body exceeds proxy `{}`'s body_limit of {} bytes: {err}",
                        proxy.name, proxy.body_limit
                    ),
                )
            })?;
        let root = doppel_render::parse_body(&bytes)?;
        crate::mock::bind_body(mock, &root, &mut vars)?;
    }

    let renderer = Renderer::new();
    let body: Vec<u8> = match &mock.body {
        MockBody::None => Vec::new(),
        MockBody::Text(template) => renderer.render_str(template, &vars)?.into_bytes(),
        MockBody::Json(template) => renderer.render_json(template, &vars)?.into_bytes(),
        MockBody::Template(file) => {
            let contents = read_template(templates_dir, &proxy.name, file)?;
            renderer.render_str(&contents, &vars)?.into_bytes()
        }
    };

    let mut builder = Response::builder().status(mock.status);
    for (name, template) in &mock.headers {
        let value = renderer.render_str(template, &vars)?;
        builder = builder.header(name.as_str(), value);
    }

    builder.body(axum::body::Body::from(body)).map_err(|err| {
        Error::new(
            ErrorCode::TemplateRenderError,
            format!("mock `{}` produced an invalid response: {err}", mock.name),
        )
    })
}

/// Reads `<templates_dir>/<proxy>/<file>` for `MockBody::Template`. Per spec
/// section 8, this happens per request rather than at compile time, because
/// phase 3 uploads templates at runtime and a reload must pick up whatever is
/// on disk *at that moment*.
///
/// The mock's file name already passed `sanitize` at config validation (rule
/// V31), but the proxy name never goes through that check -- V6 only rejects
/// a *duplicate* proxy name, not an unsafe one -- so both are sanitized again
/// here, defensively, before either becomes a path component. A name that
/// fails this check cannot name a real template underneath `templates_dir`
/// either way, so it is reported the same as a genuinely missing file rather
/// than inventing a new error code for it.
fn read_template(
    templates_dir: &std::path::Path,
    proxy_name: &str,
    file: &str,
) -> Result<String, Error> {
    let not_found = || {
        Error::new(
            ErrorCode::TemplateNotFound,
            format!("template `{file}` not found for proxy `{proxy_name}`"),
        )
    };
    let safe_proxy = doppel_core::store::name::sanitize(proxy_name).map_err(|_| not_found())?;
    let safe_file = doppel_core::store::name::sanitize(file).map_err(|_| not_found())?;
    let path = templates_dir.join(safe_proxy).join(safe_file);
    std::fs::read_to_string(&path).map_err(|_| not_found())
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

    /// Builds config text pointing the proxy's upstream at `base`.
    fn config_pointing_at(base: &str, extra: &str) -> String {
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
    url: "{base}"
    resolve:
      type: default
{extra}
"#
        )
    }

    /// The upstream points at loopback port 1, where nothing listens. Any test
    /// that passes without a real upstream therefore proves the upstream was
    /// never contacted.
    fn config_with(extra: &str) -> String {
        config_pointing_at("http://127.0.0.1:1/", extra)
    }

    /// Spawns a tiny real upstream that answers every request with 200 OK,
    /// for the one branch (a successful forward) that actually needs one.
    async fn spawn_ok_upstream() -> String {
        let app = axum::Router::new().fallback(axum::routing::any(|| async { "ok" }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        format!("http://{addr}/")
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

    // -- Log level distinction and log schema -----------------------------
    //
    // A minimal `tracing::Subscriber`, set as the *thread-local* default via
    // `set_default` (not `set_global_default`). `#[tokio::test]` runs on a
    // single-threaded current-thread runtime, so the subscriber stays active
    // across every await point in the request under test without ever
    // touching global state another test could observe.
    //
    // It records, per event: the level, the message, every field name
    // *declared* at that callsite (`Metadata::fields()`, fixed at compile
    // time from the macro invocation regardless of whether a value was ever
    // recorded for it), and every field name that actually got a value
    // (`recorded_fields`). Declared-but-not-recorded is not exercised by this
    // file's production code today: every field named in a `handle` callsite
    // is always given a value at that callsite (see `assert_full_schema`
    // below), and the `upstream_contacted` boolean -- rather than an
    // `Option`-typed `upstream_status`/`upstream_duration_ms` pair -- is
    // exactly what lets the schema stay uniform without ever needing a
    // present-but-unrecorded field. `declared_fields` is captured anyway
    // because it is the only way to see the full, fixed key set a callsite
    // promises regardless of which branch of `handle` produced the event,
    // which `assert_full_schema` relies on.

    #[derive(Debug, Clone)]
    struct CapturedEvent {
        level: tracing::Level,
        message: String,
        declared_fields: Vec<String>,
        recorded_fields: std::collections::BTreeMap<String, String>,
    }

    struct RecordFields {
        message: String,
        recorded_fields: std::collections::BTreeMap<String, String>,
    }

    impl tracing::field::Visit for RecordFields {
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            let text = format!("{value:?}");
            if field.name() == "message" {
                self.message = text;
            } else {
                self.recorded_fields.insert(field.name().to_owned(), text);
            }
        }
    }

    struct Capture {
        events: Arc<std::sync::Mutex<Vec<CapturedEvent>>>,
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
            let mut visitor = RecordFields {
                message: String::new(),
                recorded_fields: std::collections::BTreeMap::new(),
            };
            event.record(&mut visitor);
            let declared_fields = event
                .metadata()
                .fields()
                .iter()
                .map(|field| field.name().to_owned())
                .collect();
            self.events.lock().unwrap().push(CapturedEvent {
                level: *event.metadata().level(),
                message: visitor.message,
                declared_fields,
                recorded_fields: visitor.recorded_fields,
            });
        }

        fn enter(&self, _span: &tracing::span::Id) {}

        fn exit(&self, _span: &tracing::span::Id) {}
    }

    async fn run_captured(
        state: ProxyState,
        request: Request<Body>,
    ) -> (axum::response::Response, Vec<CapturedEvent>) {
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

    /// The nine fields every completion log carries on every branch, per the
    /// design spec's logging section -- `upstream_contacted` included,
    /// `upstream_status`/`upstream_duration_ms` excluded, since those two are
    /// present only when `upstream_contacted` is true (see the doc comment on
    /// the resolve-error branch in `handle` for why this is a bool and not a
    /// null-valued pair of fields).
    const REQUIRED_FIELDS: [&str; 9] = [
        "request_id",
        "proxy",
        "method",
        "path",
        "status",
        "duration_ms",
        "upstream_contacted",
        "loss_injected",
        "latency_injected_ms",
    ];

    fn assert_full_schema(event: &CapturedEvent) {
        for field in REQUIRED_FIELDS {
            assert!(
                event.declared_fields.iter().any(|name| name == field),
                "missing field `{field}` in {event:?}"
            );
        }
    }

    fn assert_upstream_contacted(event: &CapturedEvent, expected: bool) {
        assert_eq!(
            event.recorded_fields.get("upstream_contacted"),
            Some(&expected.to_string()),
            "wrong upstream_contacted in {event:?}"
        );
    }

    /// Asserts `upstream_status`/`upstream_duration_ms` are absent outright --
    /// not present-and-null, since `tracing` cannot express that -- which is
    /// the correct shape whenever no upstream was actually contacted, or a
    /// connection was attempted but never produced an `UpstreamOutcome`.
    fn assert_upstream_fields_absent(event: &CapturedEvent) {
        assert!(
            !event
                .declared_fields
                .iter()
                .any(|name| name == "upstream_status"),
            "upstream_status should be absent, not declared, in {event:?}"
        );
        assert!(
            !event
                .declared_fields
                .iter()
                .any(|name| name == "upstream_duration_ms"),
            "upstream_duration_ms should be absent, not declared, in {event:?}"
        );
    }

    fn assert_upstream_fields_present(event: &CapturedEvent) {
        assert!(
            event.recorded_fields.contains_key("upstream_status"),
            "upstream_status should be recorded when the upstream was contacted, got {event:?}"
        );
        assert!(
            event.recorded_fields.contains_key("upstream_duration_ms"),
            "upstream_duration_ms should be recorded when the upstream was contacted, got {event:?}"
        );
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
        let event = &events[0];
        assert_eq!(event.level, tracing::Level::INFO, "got {event:?}");
        assert!(event.message.contains("rejected"), "got {event:?}");
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
        let event = &events[0];
        assert_eq!(event.level, tracing::Level::WARN, "got {event:?}");
        assert!(event.message.contains("failed"), "got {event:?}");
    }

    #[tokio::test]
    async fn resolve_error_branch_has_the_full_schema_with_upstream_contacted_false() {
        let text = config_with("").replace(
            "      type: default",
            "      type: header\n      header: X-Proxy-Name",
        );
        let (response, events) = run_captured(state(&text, vec![]), get("/anything")).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(events.len(), 1, "{events:?}");
        assert_full_schema(&events[0]);
        assert_upstream_contacted(&events[0], false);
        assert_upstream_fields_absent(&events[0]);
    }

    #[tokio::test]
    async fn loss_branch_has_the_full_schema_with_upstream_contacted_false() {
        let text = config_with("    loss:\n      percentage: 1.0\n      status: 503");
        let (response, events) = run_captured(state(&text, vec![0.0]), get("/anything")).await;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(events.len(), 1, "{events:?}");
        assert_full_schema(&events[0]);
        assert_upstream_contacted(&events[0], false);
        assert_upstream_fields_absent(&events[0]);
    }

    #[tokio::test]
    async fn forward_error_branch_has_the_full_schema_with_upstream_contacted_true() {
        // Connection to loopback port 1 is refused: a genuine upstream
        // failure, so `upstream_contacted` is true even though `forward`
        // never got far enough to produce a status or duration to report.
        let (response, events) =
            run_captured(state(&config_with(""), vec![]), get("/anything")).await;
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        assert_eq!(events.len(), 1, "{events:?}");
        assert_full_schema(&events[0]);
        assert_upstream_contacted(&events[0], true);
        assert_upstream_fields_absent(&events[0]);
    }

    #[tokio::test]
    async fn client_rejected_path_has_the_full_schema_with_upstream_contacted_false() {
        // `/../secret` fails `join_upstream`'s path validation before any
        // connection is attempted, so unlike the genuine failure above,
        // `upstream_contacted` is false here even though both are logged
        // from the same `Err(err)` arm in `handle`.
        let (response, events) =
            run_captured(state(&config_with(""), vec![]), get("/../secret")).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(events.len(), 1, "{events:?}");
        assert_full_schema(&events[0]);
        assert_upstream_contacted(&events[0], false);
        assert_upstream_fields_absent(&events[0]);
    }

    #[tokio::test]
    async fn forward_success_branch_has_the_full_schema_with_upstream_contacted_true() {
        let base = spawn_ok_upstream().await;
        let text = config_pointing_at(&base, "");
        let (response, events) = run_captured(state(&text, vec![]), get("/anything")).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(events.len(), 1, "{events:?}");
        let event = &events[0];
        assert_full_schema(event);
        assert_upstream_contacted(event, true);
        assert_upstream_fields_present(event);
    }

    #[tokio::test]
    async fn loss_and_latency_together_skip_the_latency_delay_entirely() {
        // Pipeline order is resolve, then loss, then latency, then forward.
        // With loss configured to always fire, latency must never even be
        // sampled, let alone slept on -- `decide` already pins this at the
        // unit level (`fault::tests::loss_short_circuits_latency`); this
        // pins the same claim through the whole pipeline. A single sampler
        // draw is supplied: if latency were sampled too, `SequenceSampler`
        // would panic on exhaustion rather than silently letting a slow test
        // pass.
        let text = config_with(
            "    loss:\n      percentage: 1.0\n      status: 503\n    latency:\n      \
             percentage: 1.0\n      min: 2.0\n      max: 2.0",
        );
        let started = Instant::now();
        let response = send(state(&text, vec![0.0]), get("/anything")).await;
        let elapsed = started.elapsed();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(
            elapsed < Duration::from_millis(500),
            "expected the 2s configured latency to be skipped entirely, took {elapsed:?}"
        );
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

    // -- X-Request-ID reaches the client on every branch, not just forward --

    fn get_with_request_id(uri: &str, id: &str) -> Request<Body> {
        let mut request = get(uri);
        request
            .headers_mut()
            .insert("x-request-id", id.parse().unwrap());
        request
    }

    #[tokio::test]
    async fn client_supplied_request_id_is_echoed_back_on_a_successful_forward() {
        let base = spawn_ok_upstream().await;
        let text = config_pointing_at(&base, "");
        let response = send(
            state(&text, vec![]),
            get_with_request_id("/anything", "caller-id-1"),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get("x-request-id").unwrap(),
            "caller-id-1"
        );
    }

    #[tokio::test]
    async fn client_supplied_request_id_is_echoed_back_when_resolution_fails() {
        let text = config_with("").replace(
            "      type: default",
            "      type: header\n      header: X-Proxy-Name",
        );
        let response = send(
            state(&text, vec![]),
            get_with_request_id("/anything", "caller-id-2"),
        )
        .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            response.headers().get("x-request-id").unwrap(),
            "caller-id-2"
        );
    }

    #[tokio::test]
    async fn client_supplied_request_id_is_echoed_back_when_loss_drops_the_request() {
        let text = config_with("    loss:\n      percentage: 1.0\n      status: 503");
        let response = send(
            state(&text, vec![0.0]),
            get_with_request_id("/anything", "caller-id-3"),
        )
        .await;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            response.headers().get("x-request-id").unwrap(),
            "caller-id-3"
        );
    }

    #[tokio::test]
    async fn client_supplied_request_id_is_echoed_back_when_forwarding_fails() {
        let response = send(
            state(&config_with(""), vec![]),
            get_with_request_id("/anything", "caller-id-4"),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        assert_eq!(
            response.headers().get("x-request-id").unwrap(),
            "caller-id-4"
        );
    }

    #[tokio::test]
    async fn a_generated_request_id_is_returned_when_the_client_sent_none() {
        let response = send(state(&config_with(""), vec![]), get("/anything")).await;
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        let returned = response
            .headers()
            .get("x-request-id")
            .expect("a request id must be generated and returned even without one from the client")
            .to_str()
            .unwrap();
        assert_eq!(returned.len(), 32);
        assert!(returned.chars().all(|c| c.is_ascii_hexdigit()));
    }

    // -- Mock pipeline integration (phase 2 task 6) -----------------------

    mod mocks {
        use super::*;

        async fn body_string(response: axum::response::Response) -> String {
            let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
                .await
                .unwrap();
            String::from_utf8(bytes.to_vec()).unwrap()
        }

        /// `oneshot` bypasses `axum::serve`, so tests insert connect info
        /// themselves, same as `get` above.
        fn post(uri: &str, body: &'static [u8]) -> Request<Body> {
            let mut request = Request::builder()
                .method("POST")
                .uri(uri)
                .body(Body::from(body))
                .unwrap();
            request.extensions_mut().insert(axum::extract::ConnectInfo(
                "127.0.0.1:54321".parse::<std::net::SocketAddr>().unwrap(),
            ));
            request
        }

        /// A config text pointing at the dead upstream (see `config_with`),
        /// with a `templates.dir` an individual test controls, so a template
        /// test can put (or withhold) a real file underneath it.
        fn config_with_templates(dir: &std::path::Path, extra: &str) -> String {
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
templates:
  dir: "{dir}"
proxies:
  - name: p1
    type: http
    url: "http://127.0.0.1:1/"
    resolve:
      type: default
{extra}
"#,
                dir = dir.display(),
            )
        }

        /// A fresh, per-call directory under the OS temp dir -- an atomic
        /// counter alongside the process id keeps concurrently running tests
        /// in this binary from colliding on the same path.
        fn temp_templates_dir() -> std::path::PathBuf {
            static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!(
                "doppel-proxy-test-templates-{}-{id}",
                std::process::id()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            dir
        }

        #[tokio::test]
        async fn a_matched_mock_serves_without_contacting_the_upstream() {
            // The upstream is loopback port 1 (see `config_with`), where
            // nothing listens: passing at all, rather than timing out or
            // getting a connection-refused error, proves the mock served the
            // response and the upstream was never contacted.
            let extra = r#"    mocks:
      - name: m1
        request:
          method: GET
          url: /widgets/
        response:
          status: 200
          body: 'hello'
"#;
            let response = send(state(&config_with(extra), vec![0.0]), get("/widgets/")).await;
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(body_string(response).await, "hello");
        }

        #[tokio::test]
        async fn a_mock_that_loses_the_replace_roll_reaches_the_upstream() {
            let extra = r#"    replace: 0.3
    mocks:
      - name: m1
        request:
          method: GET
          url: /widgets/
        response:
          status: 200
          body: 'hello'
"#;
            // Draw above the replace threshold, so the roll is lost and the
            // dead upstream is contacted, producing 502 -- the same proof by
            // dead upstream `loss_that_does_not_fire_falls_through_to_the_upstream`
            // above uses.
            let response = send(state(&config_with(extra), vec![0.9]), get("/widgets/")).await;
            assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        }

        #[tokio::test]
        async fn a_body_extracting_mock_renders_from_the_body() {
            let extra = r#"    mocks:
      - name: m1
        request:
          method: POST
          url: /widgets/
          body:
            itemName: .name
        response:
          status: 201
          json: '{"item": "{{ itemName }}"}'
"#;
            let response = send(
                state(&config_with(extra), vec![0.0]),
                post("/widgets/", br#"{"name": "widget-1"}"#),
            )
            .await;
            assert_eq!(response.status(), StatusCode::CREATED);
            let body = json_body(response).await;
            assert_eq!(body["item"], "widget-1");
        }

        #[tokio::test]
        async fn a_body_over_the_limit_yields_413_upload_too_large() {
            let extra = r#"    body_limit: 10
    mocks:
      - name: m1
        request:
          method: POST
          url: /widgets/
          body:
            itemName: .name
        response:
          status: 201
          json: '{"item": "{{ itemName }}"}'
"#;
            let response = send(
                state(&config_with(extra), vec![0.0]),
                post(
                    "/widgets/",
                    br#"{"name": "a name much longer than ten bytes"}"#,
                ),
            )
            .await;
            assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
            let body = json_body(response).await;
            assert_eq!(body["code"], "UPLOAD_TOO_LARGE");
        }

        #[tokio::test]
        async fn a_mock_without_body_selectors_does_not_buffer_or_parse_the_body() {
            // `body_limit` is tiny and the body sent is both far larger than
            // it and not valid JSON. If the body were buffered at all, this
            // would fail as either UPLOAD_TOO_LARGE or BODY_EXTRACTION_ERROR;
            // getting the mock's own plain 200 back instead is the only
            // externally observable trace that buffering would leave, and
            // this is the strongest assertion honestly available from
            // outside the request path -- it is not a memory-instrumentation
            // proof that no allocation occurred, only a behavioural one that
            // neither failure mode a real buffer read would trip ever fires.
            let extra = r#"    body_limit: 5
    mocks:
      - name: m1
        request:
          method: POST
          url: /widgets/
        response:
          status: 200
          body: 'hello'
"#;
            let response = send(
                state(&config_with(extra), vec![0.0]),
                post(
                    "/widgets/",
                    b"this is not json and is much longer than five bytes",
                ),
            )
            .await;
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(body_string(response).await, "hello");
        }

        #[tokio::test]
        async fn an_undefined_template_variable_yields_template_render_error() {
            let extra = r#"    mocks:
      - name: m1
        request:
          method: GET
          url: /widgets/
        response:
          status: 200
          json: '{"x": "{{ missing }}"}'
"#;
            let response = send(state(&config_with(extra), vec![0.0]), get("/widgets/")).await;
            assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
            let body = json_body(response).await;
            assert_eq!(body["code"], "TEMPLATE_RENDER_ERROR");
        }

        #[tokio::test]
        async fn a_missing_template_file_yields_template_not_found() {
            let dir = temp_templates_dir();
            std::fs::create_dir_all(dir.join("p1")).unwrap();
            let extra = r#"    mocks:
      - name: m1
        request:
          method: GET
          url: /widgets/
        response:
          status: 200
          template: missing.j2
"#;
            let response = send(
                state(&config_with_templates(&dir, extra), vec![0.0]),
                get("/widgets/"),
            )
            .await;
            assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
            let body = json_body(response).await;
            assert_eq!(body["code"], "TEMPLATE_NOT_FOUND");
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[tokio::test]
        async fn a_non_json_body_with_a_body_selector_yields_body_extraction_error() {
            let extra = r#"    mocks:
      - name: m1
        request:
          method: POST
          url: /widgets/
          body:
            itemName: .name
        response:
          status: 201
          json: '{"item": "{{ itemName }}"}'
"#;
            let response = send(
                state(&config_with(extra), vec![0.0]),
                post("/widgets/", b"not json"),
            )
            .await;
            assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
            let body = json_body(response).await;
            assert_eq!(body["code"], "BODY_EXTRACTION_ERROR");
        }

        #[tokio::test]
        async fn response_header_templates_render() {
            let extra = r#"    mocks:
      - name: m1
        request:
          method: GET
          url: /widgets/(?P<resourceId>[0-9]+)/
        response:
          status: 200
          headers:
            X-Resource-ID: "{{ resourceId }}"
"#;
            let response = send(state(&config_with(extra), vec![0.0]), get("/widgets/42/")).await;
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers().get("x-resource-id").unwrap(), "42");
        }

        #[tokio::test]
        async fn the_log_line_carries_the_mock_name_with_upstream_contacted_false() {
            let extra = r#"    mocks:
      - name: my-mock
        request:
          method: GET
          url: /widgets/
        response:
          status: 200
          body: 'hello'
"#;
            let (response, events) =
                run_captured(state(&config_with(extra), vec![0.0]), get("/widgets/")).await;
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(events.len(), 1, "{events:?}");
            let event = &events[0];
            assert_upstream_contacted(event, false);
            assert!(
                event
                    .recorded_fields
                    .get("mock")
                    .is_some_and(|value| value.contains("my-mock")),
                "expected the mock name in {event:?}"
            );
        }
    }
}
