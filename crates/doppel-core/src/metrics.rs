//! The metrics Doppel exports, and the only place their names and labels are
//! written.
//!
//! Every recording goes through a function here rather than through a
//! `metrics::histogram!` at the call site. That is what makes the label sets
//! reviewable in one screen, and it is why no function below accepts a
//! request path: the cardinality rule is enforced by the signatures, not by
//! remembering it at five call sites.

use std::collections::BTreeSet;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use metrics_exporter_prometheus::{BuildError, Matcher, PrometheusBuilder, PrometheusHandle};

pub const UPSTREAM_DURATION: &str = "doppel_upstream_request_duration_seconds";
pub const PROXY_DURATION: &str = "doppel_proxy_request_duration_seconds";
pub const ADMIN_DURATION: &str = "doppel_admin_request_duration_seconds";
pub const LOSS_TOTAL: &str = "doppel_loss_total";
pub const LATENCY_INJECTED_TOTAL: &str = "doppel_latency_injected_total";
pub const MOCK_HITS_TOTAL: &str = "doppel_mock_hits_total";
pub const LAST_ERROR: &str = "doppel_proxy_last_error_timestamp_seconds";
pub const BUILD_INFO: &str = "doppel_build_info";
pub const DASHBOARD_INFO: &str = "doppel_dashboard_info";
pub const PROXY_MOCKS: &str = "doppel_proxy_mocks";

/// Recorded when a request reached the proxy listener but matched no proxy.
///
/// Rule V35 refuses an empty proxy name, so this cannot collide with a real
/// one. Dropping these requests from the metric instead would make the proxy
/// histogram silently disagree with the total traffic the process handled,
/// which is the kind of gap an operator finds during an incident.
pub const UNRESOLVED: &str = "";

/// Recorded when a request reached the admin listener and matched no route.
///
/// The same reasoning as `UNRESOLVED`, and the same value: no route template is
/// empty, so this cannot collide with one.
pub const UNMATCHED: &str = "";

/// Latency buckets, in seconds.
///
/// Explicit because the Prometheus exporter renders a histogram as a summary
/// with quantiles unless buckets are configured, and quantiles cannot be
/// aggregated across instances -- averaging a p99 is meaningless. The
/// requirement asks for buckets, and buckets are also the only form that
/// survives more than one replica.
const BUCKETS: &[f64] = &[
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 15.0, 30.0, 60.0,
];

/// The same ladder, cut at five seconds, for the admin API.
///
/// The upper reaches of the proxy ladder are there because a proxy is asked to be
/// slow on purpose -- `latency` injection goes to whatever the configuration says,
/// and a 60-second bucket is a real measurement of a fault. Nothing on the admin
/// listener has any business taking longer than five seconds: it reads a document,
/// writes a document, or renders an exposition.
const ADMIN_BUCKETS: &[f64] = &[0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0];

/// Proxy names whose mock gauge has been set, so a proxy that leaves the
/// configuration can be zeroed rather than left at its last value.
///
/// The exporter has no way to delete a series: an unset gauge keeps reporting
/// whatever it last held, which for a deleted proxy would be a mock count for
/// something that no longer exists. Zero is the honest reading, and this is the
/// smallest thing that knows which names to zero.
static GAUGED_PROXIES: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

/// Build a recorder without touching the global one.
///
/// Tests use this with `metrics::set_default_local_recorder`, so they observe
/// only their own recordings; a global recorder is process-wide and would
/// make every test see every other test's counters.
pub fn build() -> Result<metrics_exporter_prometheus::PrometheusRecorder, BuildError> {
    Ok(PrometheusBuilder::new()
        .set_buckets(BUCKETS)?
        .set_buckets_for_metric(Matcher::Full(ADMIN_DURATION.to_owned()), ADMIN_BUCKETS)?
        .build_recorder())
}

/// Why the process-wide recorder could not be installed.
#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error("cannot build the metrics recorder: {0}")]
    Build(#[from] BuildError),
    /// Reported rather than ignored. A second install means two handles
    /// exist and `/metrics` would render from whichever one the caller
    /// happens to hold, which looks like metrics silently going flat.
    #[error("a metrics recorder is already installed in this process")]
    AlreadyInstalled,
}

/// Install the process-wide recorder and return the handle `/metrics` renders
/// from. Called once, by `serve`.
pub fn install() -> Result<PrometheusHandle, InstallError> {
    let recorder = build()?;
    let handle = recorder.handle();
    metrics::set_global_recorder(recorder).map_err(|_| InstallError::AlreadyInstalled)?;
    Ok(handle)
}

/// One upstream exchange: the time `forward` measured, by proxy, method and
/// status.
pub fn record_upstream(proxy: &str, method: &str, status: u16, elapsed: Duration) {
    metrics::histogram!(
        UPSTREAM_DURATION,
        "proxy" => proxy.to_owned(),
        "method" => crate::method::method_label(method),
        "status" => status.to_string(),
    )
    .record(elapsed.as_secs_f64());
}

/// How a request through the proxy listener was answered, beyond its status.
///
/// Three booleans rather than one enum: they are not exclusive. A mock can answer
/// a request the proxy would otherwise have dropped, and an upstream error can
/// arrive after a latency delay. Each is `1` or `0` in a label, so a query can ask
/// for "everything that was not a fault" without parsing a status class.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Outcome {
    /// A mock answered instead of the upstream.
    pub replace: bool,
    /// Loss injection dropped the request.
    pub loss: bool,
    /// The upstream was contacted and did not answer usefully -- a transport
    /// failure, a timeout, or a status of 500 and above.
    pub upstream_error: bool,
}

impl Outcome {
    /// A plain proxied exchange.
    #[must_use]
    pub fn proxied() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn mocked() -> Self {
        Self {
            replace: true,
            ..Self::default()
        }
    }

    #[must_use]
    pub fn lost() -> Self {
        Self {
            loss: true,
            ..Self::default()
        }
    }

    /// The label value: `"1"` or `"0"`, never `"true"`, because a Prometheus
    /// query on a boolean label reads better against the digits and every
    /// dashboard already writes them that way.
    fn flag(value: bool) -> &'static str {
        if value { "1" } else { "0" }
    }
}

/// One request through the proxy listener, however it was answered: proxied,
/// mocked, dropped or rejected.
pub fn record_proxy(proxy: &str, method: &str, status: u16, elapsed: Duration, outcome: Outcome) {
    metrics::histogram!(
        PROXY_DURATION,
        "proxy" => proxy.to_owned(),
        "method" => crate::method::method_label(method),
        "status" => status.to_string(),
        "replace" => Outcome::flag(outcome.replace),
        "loss" => Outcome::flag(outcome.loss),
        "upstream_error" => Outcome::flag(outcome.upstream_error),
    )
    .record(elapsed.as_secs_f64());
}

/// One request to the admin listener, by route template rather than by path.
///
/// `route` is what the router matched -- `/api/v1/proxies/{name}`, not
/// `/api/v1/proxies/alpha` -- so a hundred proxies are one series and a query
/// string is none. That is the same cardinality rule the proxy metrics follow, and
/// it is why this takes a template and not a `Uri`: the type is the rule.
///
/// A request that matched no route is recorded under `UNMATCHED`, so the total
/// here is the total the listener answered.
pub fn record_admin(method: &str, route: &str, status: u16, elapsed: Duration) {
    metrics::histogram!(
        ADMIN_DURATION,
        "route" => route.to_owned(),
        "method" => crate::method::method_label(method),
        "status" => status.to_string(),
    )
    .record(elapsed.as_secs_f64());
}

/// When the proxy listener last answered with an error, and which error it was.
///
/// A timestamp rather than a counter because the question it answers is "is this
/// still happening", and a gauge of seconds since the epoch answers that at any
/// scrape interval: `time() - doppel_proxy_last_error_timestamp_seconds`. A
/// counter answers "how many" and needs a rate window to say anything about now.
pub fn record_proxy_error(code: &str) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |since| since.as_secs_f64());
    metrics::gauge!(LAST_ERROR, "code" => code.to_owned()).set(now);
}

/// The series that exists before anything goes wrong: `code=""`, value `0`.
///
/// Without it, a dashboard panel and an alert on this metric both read "no data"
/// until the first error, which is indistinguishable from "the process is not
/// being scraped". Zero says "no error yet" in a way a query can subtract.
pub fn init_proxy_errors() {
    metrics::gauge!(LAST_ERROR, "code" => "").set(0.0);
}

/// This binary, as a label: the info-metric pattern, value always `1`.
///
/// Version in a label rather than as a value, because a version is not a number
/// -- `0.10.0` and `0.1.0` do not order as floats -- and because that is how every
/// exporter publishes one, so `doppel_build_info` joins with the rest of a fleet's
/// build panels without special-casing.
pub fn describe_build(version: &str) {
    metrics::gauge!(BUILD_INFO, "version" => version.to_owned()).set(1.0);
}

/// Whether this process serves the dashboard, in the same shape.
///
/// Separate from `doppel_build_info` on purpose: one describes the artifact and
/// never changes, the other describes what this deployment turned on. Folding a
/// configuration flag into build info makes two builds of one version look
/// different.
pub fn describe_dashboard(enabled: bool) {
    metrics::gauge!(DASHBOARD_INFO, "enabled" => if enabled { "true" } else { "false" }).set(1.0);
}

/// The mock count of every proxy in the configuration now in effect.
///
/// Called on every runtime swap -- startup, a reload, a write through the admin
/// API -- so what this reports is the configuration being served rather than the
/// one the process started with. A proxy that has left the configuration is set to
/// zero rather than abandoned at its last count: the exporter cannot drop a
/// series, and a stale one reads as a proxy that still exists.
pub fn record_proxy_mocks(counts: &[(&str, usize)]) {
    let mut gauged = GAUGED_PROXIES
        .lock()
        .expect("the gauge set is not poisoned");
    for (proxy, mocks) in counts {
        #[allow(clippy::cast_precision_loss)]
        metrics::gauge!(PROXY_MOCKS, "proxy" => (*proxy).to_owned()).set(*mocks as f64);
        gauged.insert((*proxy).to_owned());
    }

    let present: BTreeSet<&str> = counts.iter().map(|(proxy, _)| *proxy).collect();
    for gone in gauged
        .iter()
        .filter(|name| !present.contains(name.as_str()))
    {
        metrics::gauge!(PROXY_MOCKS, "proxy" => gone.clone()).set(0.0);
    }
}

pub fn record_loss(proxy: &str) {
    metrics::counter!(LOSS_TOTAL, "proxy" => proxy.to_owned()).increment(1);
}

pub fn record_latency_injected(proxy: &str) {
    metrics::counter!(LATENCY_INJECTED_TOTAL, "proxy" => proxy.to_owned()).increment(1);
}

/// Labelled by mock name, which is bounded by the configuration rather than
/// by traffic, so it does not have the problem a path label would.
pub fn record_mock_hit(proxy: &str, mock: &str) {
    metrics::counter!(
        MOCK_HITS_TOTAL,
        "proxy" => proxy.to_owned(),
        "mock" => mock.to_owned(),
    )
    .increment(1);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Record inside a scoped recorder and return the exposition.
    ///
    /// `set_default_local_recorder` is thread-local and `#[test]` bodies run
    /// on one thread, so this observes exactly what `record` did and nothing
    /// another test recorded.
    fn exposition(record: impl FnOnce()) -> String {
        let recorder = build().expect("recorder builds");
        let handle = recorder.handle();
        let _guard = metrics::set_default_local_recorder(&recorder);
        record();
        handle.render()
    }

    #[test]
    fn an_upstream_exchange_is_recorded_with_buckets_and_no_path() {
        let text = exposition(|| {
            record_upstream("alpha", "GET", 200, Duration::from_millis(30));
        });

        assert!(text.contains(UPSTREAM_DURATION), "{text}");
        assert!(text.contains(r#"proxy="alpha""#), "{text}");
        assert!(text.contains(r#"method="GET""#), "{text}");
        assert!(text.contains(r#"status="200""#), "{text}");
        // Buckets, not quantiles. The exporter's default is a summary with
        // quantiles, and a quantile cannot be aggregated across replicas --
        // averaging a p99 is meaningless -- so this would be useless the
        // moment there are two processes.
        assert!(
            text.contains("# TYPE doppel_upstream_request_duration_seconds histogram"),
            "{text}"
        );
        assert!(!text.contains("quantile="), "{text}");
        // A boundary from BUCKETS, and the `+Inf` that closes the ladder, so
        // the assertion pins the configured ladder rather than merely the
        // shape of one.
        assert!(text.contains(r#"le="0.05""#), "{text}");
        assert!(text.contains(r#"le="+Inf""#), "{text}");
        // 30ms falls in the 0.05 bucket and not in the 0.025 one.
        assert!(text.contains(r#"le="0.025"} 0"#), "{text}");
        assert!(text.contains(r#"le="0.05"} 1"#), "{text}");
    }

    #[test]
    fn no_metric_can_carry_a_path_label() {
        // The guarantee is structural -- no function takes a path -- and this
        // asserts the result of that, across every metric at once, so a new
        // one that grew a path label is caught here rather than in a
        // production cardinality incident.
        let text = exposition(|| {
            record_upstream("alpha", "GET", 200, Duration::from_millis(1));
            record_proxy(
                "alpha",
                "GET",
                200,
                Duration::from_millis(2),
                Outcome::proxied(),
            );
            record_loss("alpha");
            record_latency_injected("alpha");
            record_mock_hit("alpha", "mock1");
        });

        assert!(!text.contains("path="), "{text}");
        assert!(!text.contains("uri="), "{text}");
        assert!(!text.contains("endpoint="), "{text}");
    }

    #[test]
    fn every_documented_metric_appears_once_recorded() {
        let text = exposition(|| {
            record_upstream("alpha", "GET", 200, Duration::from_millis(1));
            record_proxy(
                "alpha",
                "GET",
                200,
                Duration::from_millis(2),
                Outcome::proxied(),
            );
            record_loss("alpha");
            record_latency_injected("alpha");
            record_mock_hit("alpha", "mock1");
        });

        for name in [
            UPSTREAM_DURATION,
            PROXY_DURATION,
            LOSS_TOTAL,
            LATENCY_INJECTED_TOTAL,
            MOCK_HITS_TOTAL,
        ] {
            assert!(text.contains(name), "{name} missing from:\n{text}");
        }
    }

    #[test]
    fn an_attacker_chosen_method_cannot_create_a_new_series() {
        // A method arrives from the wire. Without the bound, this loop would
        // add a thousand series to the exposition.
        let text = exposition(|| {
            for i in 0..100 {
                record_proxy(
                    "alpha",
                    &format!("EVIL{i}"),
                    200,
                    Duration::from_millis(1),
                    Outcome::proxied(),
                );
            }
        });

        assert!(text.contains(r#"method="OTHER""#), "{text}");
        assert!(!text.contains("EVIL"), "{text}");
    }

    #[test]
    fn how_a_request_was_answered_is_in_the_labels() {
        let text = exposition(|| {
            record_proxy(
                "alpha",
                "GET",
                200,
                Duration::from_millis(1),
                Outcome::mocked(),
            );
        });

        // Digits rather than `true`/`false`: a query reads better against them and
        // every dashboard writes them that way.
        assert!(text.contains(r#"replace="1""#), "{text}");
        assert!(text.contains(r#"loss="0""#), "{text}");
        assert!(text.contains(r#"upstream_error="0""#), "{text}");
    }

    #[test]
    fn the_three_flags_are_independent() {
        // Not an enum, because they are not exclusive: a mock can answer a request
        // its own loss roll then drops.
        let text = exposition(|| {
            record_proxy(
                "alpha",
                "GET",
                503,
                Duration::from_millis(1),
                Outcome {
                    replace: true,
                    loss: true,
                    upstream_error: false,
                },
            );
        });

        assert!(
            text.contains(r#"loss="1""#) && text.contains(r#"replace="1""#),
            "{text}"
        );
    }

    #[test]
    fn the_proxy_ladder_reaches_a_minute_and_the_admin_ladder_stops_at_five_seconds() {
        // A proxy is asked to be slow on purpose -- `latency` injection goes to
        // whatever the configuration says -- so a 60-second bucket is a real
        // measurement. Nothing on the admin listener has any business there.
        let text = exposition(|| {
            record_proxy(
                "alpha",
                "GET",
                200,
                Duration::from_secs(20),
                Outcome::proxied(),
            );
            record_admin("GET", "/api/v1/proxies", 200, Duration::from_millis(3));
        });

        let proxy_series: String = text
            .lines()
            .filter(|line| line.starts_with(PROXY_DURATION))
            .collect();
        assert!(proxy_series.contains(r#"le="60""#), "{proxy_series}");
        assert!(proxy_series.contains(r#"le="30"} 1"#), "{proxy_series}");
        assert!(proxy_series.contains(r#"le="15"} 0"#), "{proxy_series}");

        let admin_series: String = text
            .lines()
            .filter(|line| line.starts_with(ADMIN_DURATION))
            .collect();
        assert!(admin_series.contains(r#"le="5""#), "{admin_series}");
        assert!(!admin_series.contains(r#"le="10""#), "{admin_series}");
        assert!(!admin_series.contains(r#"le="60""#), "{admin_series}");
    }

    #[test]
    fn an_admin_request_is_labelled_by_its_route_template() {
        // The label is the template, so a hundred proxies are one series. A label
        // built from the path would be one per name, plus one per query string.
        let text = exposition(|| {
            record_admin(
                "GET",
                "/api/v1/proxies/{name}",
                200,
                Duration::from_millis(1),
            );
            record_admin("GET", UNMATCHED, 404, Duration::from_millis(1));
        });

        assert!(text.contains(r#"route="/api/v1/proxies/{name}""#), "{text}");
        assert!(text.contains(r#"route="""#), "{text}");
        assert!(!text.contains("alpha"), "{text}");
    }

    #[test]
    fn the_last_error_exists_before_anything_goes_wrong() {
        let text = exposition(init_proxy_errors);

        // Zero, with an empty code: "no error yet" in a form a query can subtract,
        // rather than the no-data a never-recorded metric gives an alert.
        assert!(text.contains(LAST_ERROR), "{text}");
        assert!(text.contains(r#"code=""} 0"#), "{text}");
    }

    #[test]
    fn an_error_stamps_the_time_under_its_code() {
        let before = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("the clock is after 1970")
            .as_secs_f64();
        let text = exposition(|| {
            init_proxy_errors();
            record_proxy_error("UPSTREAM_TIMEOUT");
        });

        let stamped = text
            .lines()
            .find(|line| line.starts_with(LAST_ERROR) && line.contains("UPSTREAM_TIMEOUT"))
            .unwrap_or_else(|| panic!("{text}"));
        let value: f64 = stamped
            .rsplit(' ')
            .next()
            .and_then(|value| value.parse().ok())
            .unwrap_or_else(|| panic!("{stamped}"));
        assert!(value >= before, "{stamped}");
        // And the default series is still there, still zero: the codes do not
        // replace each other.
        assert!(text.contains(r#"code=""} 0"#), "{text}");
    }

    #[test]
    fn the_build_and_the_dashboard_are_described_separately() {
        let text = exposition(|| {
            describe_build("1.2.3");
            describe_dashboard(true);
        });

        // Value 1, facts in labels: the info-metric pattern, and the version is not
        // a number -- `0.10.0` and `0.1.0` do not order as floats.
        assert!(
            text.contains(r#"doppel_build_info{version="1.2.3"} 1"#),
            "{text}"
        );
        assert!(
            text.contains(r#"doppel_dashboard_info{enabled="true"} 1"#),
            "{text}"
        );
    }

    #[test]
    fn a_proxy_that_leaves_the_configuration_is_zeroed_rather_than_left_behind() {
        // The exporter cannot delete a series, so a proxy that was removed would
        // otherwise keep reporting its last mock count -- a proxy that does not
        // exist, with three mocks, for as long as the process runs.
        let text = exposition(|| {
            record_proxy_mocks(&[("alpha", 3), ("beta", 0)]);
            record_proxy_mocks(&[("beta", 1)]);
        });

        assert!(
            text.contains(r#"doppel_proxy_mocks{proxy="alpha"} 0"#),
            "{text}"
        );
        assert!(
            text.contains(r#"doppel_proxy_mocks{proxy="beta"} 1"#),
            "{text}"
        );
    }

    #[test]
    fn an_unresolved_request_is_counted_under_a_name_no_proxy_can_have() {
        // Rule V35 refuses an empty proxy name, which is what makes this
        // sentinel safe rather than a collision waiting to happen.
        let text = exposition(|| {
            record_proxy(
                UNRESOLVED,
                "GET",
                404,
                Duration::from_millis(1),
                Outcome::proxied(),
            );
        });

        assert!(text.contains(r#"proxy="""#), "{text}");
    }
}
