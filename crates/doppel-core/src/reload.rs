//! Reloading the running configuration.
//!
//! One implementation, two callers: the control socket's `reload` command and
//! the admin API's `POST /api/v1/config/reload`. They differ only in how they
//! render the answer. Writing the sequence twice would mean two places where
//! "load, validate, compile, swap" could drift apart, and the one that drifts
//! is the one nobody is testing.

use std::sync::Arc;

use crate::runtime::{Runtime, RuntimeHolder};
use crate::store::{ConfigStore, Revision, StoreError};
use crate::validate::Violation;
use crate::{Config, ErrorCode};

/// What a successful reload changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReloadOutcome {
    /// The revision now in effect. Derived from the stored configuration's
    /// content, so a reload that changes nothing reports the number it
    /// reported before.
    pub revision: Revision,
    pub proxies: usize,
    /// Sections that changed but will not take effect until a restart.
    pub unapplied: Vec<&'static str>,
}

/// Why a reload did not happen. Carries violations rather than one string so
/// a caller can report every problem at once, which is the whole point of the
/// validation pass.
#[derive(Debug, Clone)]
pub struct ReloadFailure {
    pub code: ErrorCode,
    pub violations: Vec<Violation>,
}

/// Load, validate, compile, then swap.
///
/// Every step before the swap can fail harmlessly; the swap itself cannot
/// fail, so a rejected reload leaves the process serving exactly what it was
/// serving before.
///
/// This function takes no lock of its own. Callers that can reload
/// concurrently -- and the control socket and the admin API together are
/// exactly that -- must hold one shared mutex across the whole call,
/// otherwise two reloads can interleave their swaps and the later-finishing
/// one wins regardless of which was newer.
pub async fn reload(
    holder: &RuntimeHolder,
    store: &dyn ConfigStore,
    startup_config: &Config,
) -> Result<ReloadOutcome, ReloadFailure> {
    let (config, revision) = match store.load().await {
        Ok(loaded) => loaded,
        Err(StoreError::Invalid(violations)) => {
            return Err(ReloadFailure {
                code: ErrorCode::ConfigInvalid,
                violations,
            });
        }
        Err(err) => {
            return Err(ReloadFailure {
                code: ErrorCode::StoreError,
                violations: vec![Violation::new("", err.to_string())],
            });
        }
    };

    // Before compiling, so a mock that names a template finds the file. A
    // store that keeps its templates in a database has to put them on disk
    // first; `FileStore`'s default does nothing, so both stores go through one
    // reload sequence rather than one branching on which store it has.
    if let Err(err) = store.materialize_templates(&config.templates.dir).await {
        return Err(ReloadFailure {
            code: ErrorCode::StoreError,
            violations: vec![Violation::new(
                "templates.dir",
                format!("cannot materialize templates: {err}"),
            )],
        });
    }

    let unapplied = unapplied_sections(startup_config, &config);
    if !unapplied.is_empty() {
        tracing::warn!(
            sections = ?unapplied,
            "reload accepted changes to sections that only take effect on restart"
        );
    }

    match Runtime::compile(Arc::new(config), revision) {
        Ok(runtime) => {
            let proxies = runtime.proxies.len();
            holder.store(runtime);
            tracing::info!(revision = revision.0, proxies, "config reloaded");
            Ok(ReloadOutcome {
                revision,
                proxies,
                unapplied,
            })
        }
        Err(err) => Err(ReloadFailure {
            code: err.code,
            violations: vec![Violation::new("", err.message)],
        }),
    }
}

/// Top-level sections `Runtime::compile` never reads, named for the section
/// that changed between `old` and `new`. Each section type already derives
/// `PartialEq`, so a direct field comparison is all this needs -- no generic
/// config-diffing machinery, just naming the handful of places a reload
/// cannot actually reach.
///
/// `old` is the configuration the process *started* under, not the last one
/// compiled. `Runtime::compile` never reads these sections, so once one has
/// drifted from what the process is running, it stays drifted through every
/// later reload. Comparing against the last compiled configuration would
/// find it unchanged from itself and go quiet while the drift was still
/// there; comparing against the fixed startup value keeps the warning
/// level-triggered rather than edge-triggered.
#[must_use]
pub fn unapplied_sections(old: &Config, new: &Config) -> Vec<&'static str> {
    let mut sections = Vec::new();
    if old.server != new.server {
        sections.push("server");
    }
    if old.logging != new.logging {
        sections.push("logging");
    }
    if old.control != new.control {
        sections.push("control");
    }
    if old.templates != new.templates {
        sections.push("templates");
    }
    if old.sentry != new.sentry {
        sections.push("sentry");
    }
    if old.admin != new.admin {
        sections.push("admin");
    }
    sections
}
