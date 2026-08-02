//! The metrics Doppel exports, and the only place their names and labels are
//! written.
//!
//! Every recording goes through a function here rather than through a
//! `metrics::histogram!` at the call site. That is what makes the label sets
//! reviewable in one screen, and it is why no function below accepts a
//! request path: the cardinality rule is enforced by the signatures, not by
//! remembering it at five call sites.

use std::time::Duration;

use metrics_exporter_prometheus::{BuildError, PrometheusBuilder, PrometheusHandle};

pub const UPSTREAM_DURATION: &str = "doppel_upstream_request_duration_seconds";
pub const PROXY_DURATION: &str = "doppel_proxy_request_duration_seconds";
pub const LOSS_TOTAL: &str = "doppel_loss_total";
pub const LATENCY_INJECTED_TOTAL: &str = "doppel_latency_injected_total";
pub const MOCK_HITS_TOTAL: &str = "doppel_mock_hits_total";

/// Recorded when a request reached the proxy listener but matched no proxy.
///
/// Rule V35 refuses an empty proxy name, so this cannot collide with a real
/// one. Dropping these requests from the metric instead would make the proxy
/// histogram silently disagree with the total traffic the process handled,
/// which is the kind of gap an operator finds during an incident.
pub const UNRESOLVED: &str = "";

/// Latency buckets, in seconds.
///
/// Explicit because the Prometheus exporter renders a histogram as a summary
/// with quantiles unless buckets are configured, and quantiles cannot be
/// aggregated across instances -- averaging a p99 is meaningless. The
/// requirement asks for buckets, and buckets are also the only form that
/// survives more than one replica.
const BUCKETS: &[f64] = &[
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
];

/// Build a recorder without touching the global one.
///
/// Tests use this with `metrics::set_default_local_recorder`, so they observe
/// only their own recordings; a global recorder is process-wide and would
/// make every test see every other test's counters.
pub fn build() -> Result<metrics_exporter_prometheus::PrometheusRecorder, BuildError> {
    Ok(PrometheusBuilder::new()
        .set_buckets(BUCKETS)?
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

/// One request through the proxy listener, however it was answered: proxied,
/// mocked, dropped or rejected.
pub fn record_proxy(proxy: &str, method: &str, status: u16, elapsed: Duration) {
    metrics::histogram!(
        PROXY_DURATION,
        "proxy" => proxy.to_owned(),
        "method" => crate::method::method_label(method),
        "status" => status.to_string(),
    )
    .record(elapsed.as_secs_f64());
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
            record_proxy("alpha", "GET", 200, Duration::from_millis(2));
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
            record_proxy("alpha", "GET", 200, Duration::from_millis(2));
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
                record_proxy("alpha", &format!("EVIL{i}"), 200, Duration::from_millis(1));
            }
        });

        assert!(text.contains(r#"method="OTHER""#), "{text}");
        assert!(!text.contains("EVIL"), "{text}");
    }

    #[test]
    fn an_unresolved_request_is_counted_under_a_name_no_proxy_can_have() {
        // Rule V35 refuses an empty proxy name, which is what makes this
        // sentinel safe rather than a collision waiting to happen.
        let text = exposition(|| {
            record_proxy(UNRESOLVED, "GET", 404, Duration::from_millis(1));
        });

        assert!(text.contains(r#"proxy="""#), "{text}");
    }
}
