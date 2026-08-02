//! What every admin handler needs.

use std::sync::Arc;
use std::time::{Duration, Instant};

use doppel_core::store::ConfigStore;
use doppel_core::{Config, RuntimeHolder};
use metrics_exporter_prometheus::PrometheusHandle;
use tokio::sync::Mutex;

/// Shared handler state.
#[derive(Clone)]
pub struct AdminState {
    store: Arc<dyn ConfigStore>,
    holder: Arc<RuntimeHolder>,
    startup: Arc<Config>,
    reload_lock: Arc<Mutex<()>>,
    metrics: PrometheusHandle,
    started_at: Instant,
}

impl AdminState {
    /// `reload_lock` must be the *same* mutex the control socket takes. Two
    /// reloads that interleave can swap runtimes in the wrong order and leave
    /// the process serving the older of two configurations; one shared lock
    /// across both entry points is what prevents it. Passing a fresh mutex
    /// here compiles and looks right, and quietly removes the protection.
    ///
    /// `started_at` is passed in rather than taken here so that uptime means
    /// the process's, not this struct's.
    #[must_use]
    pub fn new(
        store: Arc<dyn ConfigStore>,
        holder: Arc<RuntimeHolder>,
        startup: Arc<Config>,
        reload_lock: Arc<Mutex<()>>,
        metrics: PrometheusHandle,
        started_at: Instant,
    ) -> Self {
        Self {
            store,
            holder,
            startup,
            reload_lock,
            metrics,
            started_at,
        }
    }

    /// The configuration store. The API reads and writes through this and
    /// never touches the filesystem itself, which is what makes swapping in
    /// the PostgreSQL store a matter of constructing a different `Arc`.
    #[must_use]
    pub fn store(&self) -> &dyn ConfigStore {
        self.store.as_ref()
    }

    /// The runtime the proxy listener is serving from.
    #[must_use]
    pub fn holder(&self) -> &RuntimeHolder {
        &self.holder
    }

    /// The configuration the process started under, which `reload` compares
    /// against to report sections that need a restart.
    #[must_use]
    pub fn startup(&self) -> &Config {
        &self.startup
    }

    #[must_use]
    pub fn reload_lock(&self) -> &Mutex<()> {
        &self.reload_lock
    }

    /// The handle `/metrics` renders from. Must come from
    /// `doppel_core::metrics::install`, the recorder the proxy pipeline
    /// actually records into -- a handle from a second recorder renders an
    /// empty exposition, which reads as "no traffic" rather than as a
    /// misconfiguration.
    #[must_use]
    pub fn metrics(&self) -> &PrometheusHandle {
        &self.metrics
    }

    #[must_use]
    pub fn uptime(&self) -> Duration {
        self.started_at.elapsed()
    }
}
