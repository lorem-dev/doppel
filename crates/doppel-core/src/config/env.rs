//! Configuration supplied by the environment: the admin tokens, and the
//! external url.
//!
//! A deployment that provisions its secrets through the environment should
//! not have to write them into the configuration document to use them. These
//! tokens sit beside the configured ones and take precedence over any that
//! share a name.
//!
//! They are deliberately **not** merged into `Config`. The revision is derived
//! from the configuration's content, so folding the environment into it would
//! make two instances reading one stored document compute two different
//! revisions -- and every compare-and-swap between them would fail for a
//! difference neither of them wrote. The merge happens where the tokens are
//! used, which is authentication, and nowhere else.

use std::collections::BTreeMap;

use serde::Deserialize;

use super::{ExternalUrl, Name, Token, TokenConfig, UrlError};

/// The variable read at startup.
pub const VAR: &str = "DOPPEL_ADMIN_TOKENS";

/// The variable that overrides `server.external_url`.
pub const EXTERNAL_URL_VAR: &str = "DOPPEL_EXTERNAL_URL";

/// Why `DOPPEL_EXTERNAL_URL` was refused.
///
/// Fails startup rather than being logged and skipped, for the reason the token
/// errors do: an operator who set it and got no error believes redirects are
/// being rewritten to that address.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{EXTERNAL_URL_VAR} is not usable: {reason}")]
pub struct EnvExternalUrlError {
    pub reason: UrlError,
}

/// `DOPPEL_EXTERNAL_URL`, or `None` when it is unset or empty.
///
/// Empty counts as unset, like the token variable: `DOPPEL_EXTERNAL_URL=${HOST}`
/// with nothing behind `HOST` is a compose file that means nothing by it, not a
/// deployment to refuse to start.
///
/// Not merged into `Config`, and for the same reason the tokens are not: the
/// revision is computed over the document, so folding an environment value into
/// it would make two instances reading one stored document disagree about the
/// revision, and every compare-and-swap between them fail for a difference
/// neither wrote.
pub fn external_url_from_env() -> Result<Option<ExternalUrl>, EnvExternalUrlError> {
    match std::env::var(EXTERNAL_URL_VAR) {
        Ok(raw) if raw.trim().is_empty() => Ok(None),
        Ok(raw) => ExternalUrl::parse(raw.trim())
            .map(Some)
            .map_err(|reason| EnvExternalUrlError { reason }),
        Err(_) => Ok(None),
    }
}

/// Tokens provided by the environment, in the order they were written.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EnvTokens(Vec<TokenConfig>);

/// Why the variable was refused.
///
/// Every one of these fails startup rather than being logged and skipped. An
/// operator who provisioned a token and got no error believes they have
/// access; finding out otherwise at the moment they need it is the worst
/// possible time.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EnvTokensError {
    #[error("{VAR} is not valid JSON: {0}")]
    NotJson(String),
    /// The message names the entry, never the value: this variable is a place
    /// secrets live, and an error about one is still a place it would end up.
    #[error("{VAR} entry `{name}` is not usable: {reason}")]
    BadEntry { name: String, reason: String },
    #[error("{VAR} entry `{name}` is not a usable name: {reason}")]
    BadName { name: String, reason: String },
    #[error("{VAR} gives the same token value to `{first}` and `{second}`")]
    DuplicateValue { first: String, second: String },
}

/// One entry as written in the JSON object.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    token: String,
    /// Defaults to `user`, matching `doppel token add`. A variable that
    /// granted administration by default would make one forgotten field an
    /// escalation, and this one is set by whatever provisions the
    /// environment rather than by a person reading it back.
    #[serde(default)]
    group: Option<String>,
}

impl EnvTokens {
    /// Read and check the variable, or nothing if it is unset.
    ///
    /// An empty or whitespace-only value counts as unset. Deployment tooling
    /// that renders an empty string for an absent secret is common enough
    /// that treating it as a JSON error would fail startup for a
    /// configuration nobody meant to write.
    pub fn from_env() -> Result<Self, EnvTokensError> {
        match std::env::var(VAR) {
            Ok(raw) if raw.trim().is_empty() => Ok(Self::default()),
            Ok(raw) => Self::parse(&raw),
            Err(_) => Ok(Self::default()),
        }
    }

    /// Check a JSON object of tokens.
    ///
    /// ```json
    /// {"ci": {"token": "...", "group": "admin"}, "readonly": {"token": "..."}}
    /// ```
    pub fn parse(raw: &str) -> Result<Self, EnvTokensError> {
        let entries: BTreeMap<String, Entry> =
            serde_json::from_str(raw).map_err(|err| EnvTokensError::NotJson(err.to_string()))?;

        let mut tokens = Vec::with_capacity(entries.len());
        for (name, entry) in entries {
            let parsed = Name::parse(name.clone()).map_err(|err| EnvTokensError::BadName {
                name: name.clone(),
                reason: err.to_string(),
            })?;
            let group = match entry.group {
                Some(group) => Name::parse(group).map_err(|err| EnvTokensError::BadName {
                    name: name.clone(),
                    reason: err.to_string(),
                })?,
                None => default_group(),
            };
            let token = Token::parse(entry.token).map_err(|err| EnvTokensError::BadEntry {
                name: name.clone(),
                reason: err.to_string(),
            })?;
            tokens.push(TokenConfig {
                name: parsed,
                group,
                token,
            });
        }

        // The names are a map's keys, so they are unique by construction; the
        // values are not. Two names for one secret makes which identity a
        // caller gets depend on iteration order, so it is refused rather than
        // resolved.
        for (i, token) in tokens.iter().enumerate() {
            if let Some(other) = tokens[i + 1..]
                .iter()
                .find(|other| other.token == token.token)
            {
                return Err(EnvTokensError::DuplicateValue {
                    first: token.name.to_string(),
                    second: other.name.to_string(),
                });
            }
        }

        Ok(Self(tokens))
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// The names these tokens claim, for reporting which configured ones they
    /// shadow.
    pub fn names(&self) -> impl Iterator<Item = &Name> {
        self.0.iter().map(|token| &token.name)
    }

    /// Find the token whose value is `presented`.
    ///
    /// Searched before the configured tokens, which is what "overrides on
    /// conflict" means in practice: a name given here wins, and so does a
    /// value, so a configured token can be replaced without editing the
    /// document that names it.
    #[must_use]
    pub fn find(&self, presented: &str) -> Option<&TokenConfig> {
        self.0.iter().find(|token| token.token.matches(presented))
    }

    /// Whether a configured token of this name is shadowed by one from here.
    #[must_use]
    pub fn shadows(&self, name: &Name) -> bool {
        self.0.iter().any(|token| token.name == *name)
    }
}

/// The group an entry gets when it does not name one.
fn default_group() -> Name {
    Name::parse("user").expect("a literal name")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_well_formed_object_yields_its_tokens() {
        let tokens = EnvTokens::parse(
            r#"{
                "ci": {"token": "0123456789abcdef0123456789abcdef", "group": "admin"},
                "readonly": {"token": "fedcba9876543210fedcba9876543210"}
            }"#,
        )
        .unwrap();

        assert_eq!(tokens.len(), 2);
        let ci = tokens.find("0123456789abcdef0123456789abcdef").unwrap();
        assert_eq!(ci.name, "ci");
        assert_eq!(ci.group, "admin");
        // Defaulted, and to `user` rather than `admin`.
        let readonly = tokens.find("fedcba9876543210fedcba9876543210").unwrap();
        assert_eq!(readonly.group, "user");
    }

    #[test]
    fn an_absent_or_empty_variable_is_no_tokens_rather_than_an_error() {
        // Deployment tooling that renders an empty string for an absent
        // secret is common; failing startup for it would be refusing a
        // configuration nobody wrote.
        assert!(EnvTokens::default().is_empty());
        assert!(EnvTokens::parse("{}").unwrap().is_empty());
    }

    #[test]
    fn malformed_json_is_refused_rather_than_ignored() {
        // The whole point of failing: an operator who provisioned a token and
        // saw no error believes they have access.
        let err = EnvTokens::parse("not json").unwrap_err();
        assert!(matches!(err, EnvTokensError::NotJson(_)), "{err:?}");
        assert!(err.to_string().contains(VAR), "{err}");
    }

    #[test]
    fn every_field_is_held_to_the_same_rule_as_the_configuration() {
        let short = EnvTokens::parse(r#"{"ci": {"token": "tooshort"}}"#).unwrap_err();
        assert!(
            matches!(short, EnvTokensError::BadEntry { .. }),
            "{short:?}"
        );

        let bad_name =
            EnvTokens::parse(r#"{"a/b": {"token": "0123456789abcdef0123456789abcdef"}}"#)
                .unwrap_err();
        assert!(
            matches!(bad_name, EnvTokensError::BadName { .. }),
            "{bad_name:?}"
        );

        let bad_group = EnvTokens::parse(
            r#"{"ci": {"token": "0123456789abcdef0123456789abcdef", "group": "a b"}}"#,
        )
        .unwrap_err();
        assert!(
            matches!(bad_group, EnvTokensError::BadName { .. }),
            "{bad_group:?}"
        );

        let unknown_field = EnvTokens::parse(
            r#"{"ci": {"token": "0123456789abcdef0123456789abcdef", "grp": "admin"}}"#,
        );
        assert!(unknown_field.is_err(), "a misspelled field must not pass");
    }

    #[test]
    fn an_error_about_a_token_does_not_carry_the_token() {
        let err = EnvTokens::parse(r#"{"ci": {"token": "tooshort"}}"#)
            .unwrap_err()
            .to_string();
        assert!(err.contains("`ci`"), "{err}");
        assert!(!err.contains("tooshort"), "{err}");
    }

    #[test]
    fn two_names_for_one_secret_are_refused() {
        // Which identity a caller gets would otherwise depend on iteration
        // order.
        let err = EnvTokens::parse(
            r#"{
                "one": {"token": "0123456789abcdef0123456789abcdef"},
                "two": {"token": "0123456789abcdef0123456789abcdef"}
            }"#,
        )
        .unwrap_err();
        assert!(
            matches!(err, EnvTokensError::DuplicateValue { .. }),
            "{err:?}"
        );
        let message = err.to_string();
        assert!(
            message.contains("one") && message.contains("two"),
            "{message}"
        );
        assert!(!message.contains("0123456789"), "{message}");
    }

    #[test]
    fn a_presented_value_that_matches_nothing_is_nothing() {
        let tokens =
            EnvTokens::parse(r#"{"ci": {"token": "0123456789abcdef0123456789abcdef"}}"#).unwrap();
        assert!(tokens.find("something else entirely").is_none());
        assert!(tokens.find("").is_none());
    }

    #[test]
    fn shadowing_is_by_name() {
        let tokens =
            EnvTokens::parse(r#"{"ci": {"token": "0123456789abcdef0123456789abcdef"}}"#).unwrap();
        assert!(tokens.shadows(&Name::parse("ci").unwrap()));
        assert!(!tokens.shadows(&Name::parse("other").unwrap()));
        assert_eq!(tokens.names().map(Name::as_str).collect::<Vec<_>>(), ["ci"]);
    }
}
