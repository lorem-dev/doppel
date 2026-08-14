//! Optional Sentry reporting.
//!
//! Behind the `sentry` cargo feature. The default build has no Sentry code in
//! it at all, and `init` below still exists and still succeeds, so nothing
//! that calls it needs a `cfg`.

use doppel_core::config::SentryConfig;
use doppel_core::redact_credentials;

use crate::TelemetryError;

/// What `init` actually did, so a caller -- or a test -- can tell the three
/// cases apart without reading log output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SentryStatus {
    /// No `sentry` section, or an empty DSN. The documented way to turn it
    /// off, and not an error.
    Disabled,
    /// A DSN was configured but this binary was built without the `sentry`
    /// feature, so nothing will be reported.
    Unsupported,
    Enabled,
}

/// Holds the Sentry client for as long as the process runs.
///
/// Dropping it flushes pending events and stops reporting, so `serve` binds
/// it to a name that lives until it returns. A `let _ = init(..)` would drop
/// it immediately and silently disable everything -- which is why this type
/// is `#[must_use]`.
#[must_use = "dropping the guard flushes and disables Sentry"]
pub struct Sentry {
    pub status: SentryStatus,
    #[cfg(feature = "sentry")]
    _guard: Option<::sentry::ClientInitGuard>,
}

impl std::fmt::Debug for Sentry {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        // No DSN field, and none reachable from the guard's `Debug` either:
        // the point of this type is that the DSN does not travel.
        f.debug_struct("Sentry")
            .field("status", &self.status)
            .finish_non_exhaustive()
    }
}

/// Where a DSN came from, for the line that says reporting is on.
///
/// Worth logging: "sentry reporting enabled" with a redacted DSN does not tell an
/// operator whether the document or the environment won, and that is exactly what
/// they need to know when it is the wrong one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DsnSource {
    Document,
    Environment,
}

impl DsnSource {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Document => "sentry.dsn",
            Self::Environment => "DOPPEL_SENTRY_DSN",
        }
    }
}

/// The DSN to use, and where it came from.
///
/// The environment wins. A DSN is a credential, and a deployment that provisions
/// credentials through the environment is overriding a document it may not even
/// be able to edit -- the same precedence `DOPPEL_ADMIN_TOKENS` has over
/// `admin.tokens`.
///
/// An absent section, a section with an empty or whitespace-only DSN, and an unset
/// variable all mean the same thing. Treating `dsn: ""` as a value would try to
/// initialise Sentry against nothing and fail startup for what is plainly a way of
/// writing "off".
fn resolve_dsn<'a>(
    config: Option<&'a SentryConfig>,
    from_env: Option<&'a str>,
) -> Option<(&'a str, DsnSource)> {
    if let Some(dsn) = from_env.map(str::trim).filter(|dsn| !dsn.is_empty()) {
        return Some((dsn, DsnSource::Environment));
    }
    config
        .map(|sentry| sentry.dsn.trim())
        .filter(|dsn| !dsn.is_empty())
        .map(|dsn| (dsn, DsnSource::Document))
}

#[cfg(feature = "sentry")]
pub fn init(
    config: Option<&SentryConfig>,
    from_env: Option<&str>,
) -> Result<Sentry, TelemetryError> {
    let Some((dsn, source)) = resolve_dsn(config, from_env) else {
        return Ok(Sentry {
            status: SentryStatus::Disabled,
            _guard: None,
        });
    };

    // Parsed before `init`, so a malformed DSN is reported as one rather
    // than as a client that silently drops everything: `sentry::init`
    // accepts a value it could not parse and returns a disabled client.
    let parsed =
        dsn.parse::<::sentry::types::Dsn>()
            .map_err(|err| TelemetryError::BadSentryDsn {
                dsn: redact_credentials(dsn),
                reason: err.to_string(),
            })?;

    // `ClientOptions` is `#[non_exhaustive]`, so it is built by mutating a
    // default rather than by a struct literal.
    let mut options = ::sentry::ClientOptions::default();
    options.dsn = Some(parsed);
    // Errors from a released binary are only actionable if the release is on
    // them.
    options.release = ::sentry::release_name!();
    let guard = ::sentry::init(options);

    tracing::info!(
        dsn = %redact_credentials(dsn),
        source = source.as_str(),
        "sentry reporting enabled"
    );
    Ok(Sentry {
        status: SentryStatus::Enabled,
        _guard: Some(guard),
    })
}

#[cfg(not(feature = "sentry"))]
pub fn init(
    config: Option<&SentryConfig>,
    from_env: Option<&str>,
) -> Result<Sentry, TelemetryError> {
    let status = if let Some((dsn, source)) = resolve_dsn(config, from_env) {
        // Loud rather than silent. The operator asked for reporting and will
        // not get it; a knob that reads as honoured and is not is the defect
        // this project already removed once, in `admin.workers`. Not fatal,
        // though -- Sentry is optional by design, and refusing to start would
        // turn an observability gap into an outage.
        tracing::warn!(
            dsn = %redact_credentials(dsn),
            source = source.as_str(),
            "a sentry dsn is configured but this binary was built without the `sentry` \
             feature; nothing will be reported"
        );
        SentryStatus::Unsupported
    } else {
        SentryStatus::Disabled
    };
    Ok(Sentry { status })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(dsn: &str) -> SentryConfig {
        SentryConfig {
            dsn: dsn.to_owned(),
        }
    }

    #[test]
    fn no_sentry_section_is_disabled_and_not_an_error() {
        assert_eq!(init(None, None).unwrap().status, SentryStatus::Disabled);
    }

    #[test]
    fn an_empty_dsn_is_the_documented_way_to_turn_it_off() {
        for dsn in ["", "   ", "\t"] {
            assert_eq!(
                init(Some(&config(dsn)), None).unwrap().status,
                SentryStatus::Disabled,
                "{dsn:?} should read as off"
            );
        }
    }

    #[test]
    fn the_environment_provides_a_dsn_when_the_document_has_none() {
        // The case this exists for: a deployment that provisions credentials
        // through the environment and a configuration that names none.
        let (dsn, source) = resolve_dsn(None, Some("https://key@sentry.example.com/1"))
            .expect("the variable is a dsn");

        assert_eq!(dsn, "https://key@sentry.example.com/1");
        assert_eq!(source, DsnSource::Environment);
    }

    #[test]
    fn the_environment_wins_over_the_document() {
        // The same precedence `DOPPEL_ADMIN_TOKENS` has: a deployment overriding
        // a document it may not be able to edit.
        let document = config("https://from-document@sentry.example.com/1");
        let (dsn, source) = resolve_dsn(
            Some(&document),
            Some("https://from-environment@sentry.example.com/2"),
        )
        .expect("a dsn is resolved");

        assert_eq!(dsn, "https://from-environment@sentry.example.com/2");
        assert_eq!(source, DsnSource::Environment);
    }

    #[test]
    fn an_empty_variable_leaves_the_document_in_force() {
        // Deliberately this direction. `DOPPEL_SENTRY_DSN=${SENTRY_DSN}` with
        // nothing behind `SENTRY_DSN` is a compose file that means nothing by it,
        // and silently turning error reporting off is the worse reading.
        let document = config("https://key@sentry.example.com/1");
        for empty in ["", "   "] {
            let (dsn, source) =
                resolve_dsn(Some(&document), Some(empty)).expect("the document still names one");

            assert_eq!(dsn, "https://key@sentry.example.com/1", "{empty:?}");
            assert_eq!(source, DsnSource::Document, "{empty:?}");
        }
    }

    #[test]
    fn a_dsn_from_the_environment_is_trimmed_like_one_from_the_document() {
        let (dsn, _) = resolve_dsn(None, Some("  https://key@sentry.example.com/1  "))
            .expect("whitespace is not part of a dsn");
        assert_eq!(dsn, "https://key@sentry.example.com/1");
    }

    #[test]
    fn the_source_is_named_the_way_an_operator_would_look_for_it() {
        // It goes in a log line, so it has to be the thing they would grep for
        // rather than a word this module invented.
        assert_eq!(DsnSource::Document.as_str(), "sentry.dsn");
        assert_eq!(DsnSource::Environment.as_str(), "DOPPEL_SENTRY_DSN");
    }

    #[test]
    fn the_debug_of_the_guard_carries_no_dsn() {
        // `serve` may log its state, and a `Debug` that printed the DSN would
        // put the key in the log the first time anyone did.
        let sentry = init(Some(&config("https://key@sentry.example.com/1")), None).unwrap();
        let rendered = format!("{sentry:?}");
        assert!(!rendered.contains("key"), "{rendered}");
        assert!(!rendered.contains("sentry.example.com"), "{rendered}");
    }

    #[cfg(feature = "sentry")]
    #[test]
    fn a_malformed_dsn_is_reported_with_the_key_masked() {
        // Reported rather than accepted: `sentry::init` takes an unparseable
        // value and hands back a client that drops everything, so the only
        // signal an operator would get is silence.
        let err = init(Some(&config("https://s3cr3tkey@/missing-project")), None)
            .expect_err("a DSN with no host must be refused");
        let message = err.to_string();
        assert!(!message.contains("s3cr3tkey"), "{message}");
    }

    #[cfg(feature = "sentry")]
    #[test]
    fn a_wholly_unparseable_dsn_is_not_echoed_back() {
        let err = init(
            Some(&config("this is not a dsn but it might be a secret")),
            None,
        )
        .expect_err("must be refused");
        let message = err.to_string();
        assert!(!message.contains("might be a secret"), "{message}");
        assert!(message.contains("<redacted>"), "{message}");
    }

    #[cfg(not(feature = "sentry"))]
    #[test]
    fn a_configured_dsn_without_the_feature_is_unsupported_not_silently_disabled() {
        // The distinction is the point: `Disabled` means the operator turned
        // it off, `Unsupported` means they asked and this build cannot.
        assert_eq!(
            init(Some(&config("https://key@sentry.example.com/1")), None)
                .unwrap()
                .status,
            SentryStatus::Unsupported
        );
    }
}
