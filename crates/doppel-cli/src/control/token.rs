//! Issuing an admin token over the control channel.
//!
//! The command exists because the alternative is editing `admin.tokens` by
//! hand and reloading, which means the operator chooses the secret. A
//! generated one is a better secret, and going through the running process
//! means the token is in force when the command returns rather than at some
//! later moment nobody records.

use doppel_core::config::{EnvTokens, Name, Token, TokenConfig};
use doppel_core::store::{ConfigStore, StoreError};
use doppel_core::{Config, ErrorCode, RuntimeHolder, Violation};

use super::ControlResponse;

/// How many times a losing compare-and-swap is retried.
///
/// A `save` carries the revision the configuration was read at, so a change
/// landing in between makes this one fail rather than overwrite it. Retrying
/// re-reads and re-appends, which is correct because appending a token
/// commutes with anything else that happened. Three is a bound on a loop that
/// should not run twice; a caller losing it three times in a row is not
/// contention, it is something else.
const CAS_ATTEMPTS: usize = 3;

/// Generate a token, persist it, and bring it into force.
///
/// Held under the reload lock by the caller, so the read-modify-write below
/// cannot interleave with a reload swapping a different runtime in behind it.
pub(super) async fn add(
    holder: &RuntimeHolder,
    store: &dyn ConfigStore,
    startup_config: &Config,
    env_tokens: &EnvTokens,
    name: Name,
    group: Name,
) -> ControlResponse {
    // A name the environment claims cannot be issued here: the environment
    // is searched first at authentication, so the token this would generate
    // and store would never authenticate. Handing one out anyway is the
    // worst outcome available -- a credential that looks issued and is not.
    if env_tokens.shadows(&name) {
        return ControlResponse::Error {
            code: ErrorCode::ConfigInvalid,
            errors: vec![Violation::new(
                "name",
                format!(
                    "`{name}` is supplied by the environment, so a token issued here \
                     under that name would never authenticate; pick another name or \
                     remove it from the environment"
                ),
            )],
        };
    }

    let token = Token::generate();

    for attempt in 1..=CAS_ATTEMPTS {
        let (mut config, revision) = match store.load().await {
            Ok(loaded) => loaded,
            Err(err) => return store_failure(&err),
        };

        // Checked here rather than left to rule V26 so the answer names the
        // command's own argument. V26 reports `admin.tokens[3].name`, which
        // is a position in a document the caller never saw.
        if config.admin.tokens.iter().any(|t| t.name == name) {
            return ControlResponse::Error {
                code: ErrorCode::ConfigInvalid,
                errors: vec![Violation::new(
                    "name",
                    format!("a token named `{name}` already exists"),
                )],
            };
        }

        config.admin.tokens.push(TokenConfig {
            name: name.clone(),
            group: group.clone(),
            token: token.clone(),
        });

        match store.save(&config, Some(revision)).await {
            Ok(_) => break,
            Err(StoreError::RevisionMismatch { .. }) if attempt < CAS_ATTEMPTS => {
                // Someone else wrote in between. Re-read and append again.
                continue;
            }
            Err(err) => return store_failure(&err),
        }
    }

    // The token is persisted but not yet in force: the admin listener
    // authenticates against the runtime the holder is serving, so it has to
    // be swapped before this returns. A response that reported a token the
    // very next request would reject is worse than no command at all.
    match doppel_core::reload(holder, store, startup_config).await {
        Ok(outcome) => ControlResponse::TokenAdded {
            name,
            group,
            token,
            revision: outcome.revision.0,
        },
        Err(failure) => ControlResponse::Error {
            code: failure.code,
            errors: failure.violations,
        },
    }
}

fn store_failure(err: &StoreError) -> ControlResponse {
    match err {
        StoreError::Invalid(violations) => ControlResponse::Error {
            code: ErrorCode::ConfigInvalid,
            errors: violations.clone(),
        },
        other => ControlResponse::Error {
            code: ErrorCode::StoreError,
            errors: vec![Violation::new("", other.to_string())],
        },
    }
}

/// The group a token gets when the caller does not name one.
///
/// `user` rather than `admin`: a command that hands out administrative rights
/// by default is one mistyped invocation away from an incident, and widening
/// a token afterwards is a deliberate act while narrowing one is a discovery.
#[must_use]
pub fn default_group() -> Name {
    Name::parse("user").expect("a literal name")
}
