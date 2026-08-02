//! The HTTP methods this project recognises by name.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A method a mock may declare, and the only values that ever reach a metric
/// label.
///
/// An enum rather than a list of strings, for two callers with the same need
/// from opposite directions. A mock's `request.method` comes from a
/// configuration, where `FETCH` should be an error rather than a mock that
/// silently never matches -- so the set has to be closed. A metric's `method`
/// label comes from the wire, where an unrecognised method is
/// attacker-controlled, and a label taking arbitrary strings is the same
/// cardinality explosion as a path label -- so the set has to be closed there
/// too. Anything off this list is recorded as `OTHER`.
///
/// This is a typo guard, not a protocol restriction. A genuinely non-standard
/// method needs a variant adding here, and nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum HttpMethod {
    Get,
    Head,
    Post,
    Put,
    Patch,
    Delete,
    Options,
    Trace,
    Connect,
    /// The safe, idempotent method for a request that carries a body -- the
    /// "GET with a body" gap.
    Query,
    // WebDAV (RFC 4918).
    Propfind,
    Proppatch,
    Mkcol,
    Copy,
    Move,
    Lock,
    Unlock,
}

/// Every variant, in the order they are documented.
///
/// Written out rather than derived: it is what `FromStr` searches and what
/// the OpenAPI schema publishes. The exhaustiveness test below is what keeps
/// a new variant from being added to the enum and forgotten here.
pub const ALL_METHODS: &[HttpMethod] = &[
    HttpMethod::Get,
    HttpMethod::Head,
    HttpMethod::Post,
    HttpMethod::Put,
    HttpMethod::Patch,
    HttpMethod::Delete,
    HttpMethod::Options,
    HttpMethod::Trace,
    HttpMethod::Connect,
    HttpMethod::Query,
    HttpMethod::Propfind,
    HttpMethod::Proppatch,
    HttpMethod::Mkcol,
    HttpMethod::Copy,
    HttpMethod::Move,
    HttpMethod::Lock,
    HttpMethod::Unlock,
];

/// The label recorded for a method Doppel does not recognise.
pub const OTHER: &str = "OTHER";

impl HttpMethod {
    /// The wire spelling: always upper case, always `&'static str`.
    ///
    /// The return type is the guarantee that matters for metrics: no value
    /// derived from a request can escape into a label, because none can
    /// satisfy it.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Head => "HEAD",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Patch => "PATCH",
            Self::Delete => "DELETE",
            Self::Options => "OPTIONS",
            Self::Trace => "TRACE",
            Self::Connect => "CONNECT",
            Self::Query => "QUERY",
            Self::Propfind => "PROPFIND",
            Self::Proppatch => "PROPPATCH",
            Self::Mkcol => "MKCOL",
            Self::Copy => "COPY",
            Self::Move => "MOVE",
            Self::Lock => "LOCK",
            Self::Unlock => "UNLOCK",
        }
    }
}

/// Why a method was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MethodError {
    /// The common mistake, separated from the general one because the fix is
    /// different and obvious once said.
    ///
    /// The value is not silently upper-cased: HTTP methods are case
    /// sensitive, a stored `get` would never match an incoming `GET`, and
    /// rewriting what the operator wrote is a worse answer than refusing it.
    #[error("HTTP methods are case-sensitive; use `{upper}`, not `{written}`")]
    NotUpperCase { written: String, upper: String },
    #[error(
        "`{0}` is not a method Doppel knows; this list is a typo guard, not a \
         protocol restriction, so if it is genuinely non-standard, add it to \
         the list"
    )]
    Unknown(String),
}

impl FromStr for HttpMethod {
    type Err = MethodError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if let Some(found) = ALL_METHODS.iter().find(|m| m.as_str() == value) {
            return Ok(*found);
        }
        let upper = value.to_ascii_uppercase();
        if upper != value && ALL_METHODS.iter().any(|m| m.as_str() == upper) {
            return Err(MethodError::NotUpperCase {
                written: value.to_owned(),
                upper,
            });
        }
        Err(MethodError::Unknown(value.to_owned()))
    }
}

impl fmt::Display for HttpMethod {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for HttpMethod {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for HttpMethod {
    /// Hand-written rather than derived, for the message. A derive answers
    /// `get` with "unknown variant `get`, expected one of ...", which lists
    /// seventeen alternatives and leaves the reader to spot that theirs is in
    /// there, spelled differently.
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let value = String::deserialize(d)?;
        value.parse().map_err(serde::de::Error::custom)
    }
}

impl utoipa::PartialSchema for HttpMethod {
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
        utoipa::openapi::schema::ObjectBuilder::new()
            .schema_type(utoipa::openapi::schema::Type::String)
            .description(Some("An HTTP method, upper case."))
            .enum_values(Some(ALL_METHODS.iter().map(|method| method.as_str())))
            .into()
    }
}

impl utoipa::ToSchema for HttpMethod {}

/// What to record for `method` in a metric label.
///
/// Returns the spelling of a known method or the literal `OTHER`, never the
/// caller's string.
#[must_use]
pub fn method_label(method: &str) -> &'static str {
    HttpMethod::from_str(method).map_or(OTHER, HttpMethod::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_known_method_labels_as_itself() {
        assert_eq!(method_label("GET"), "GET");
        assert_eq!(method_label("QUERY"), "QUERY");
        assert_eq!(method_label("UNLOCK"), "UNLOCK");
    }

    #[test]
    fn an_unknown_method_collapses_to_one_bounded_value() {
        // The point is the bound, so a thousand distinct inputs must give one
        // label. Anything else is a way to fill a metrics backend from the
        // outside.
        for i in 0..1000 {
            assert_eq!(method_label(&format!("BREW{i}")), "OTHER");
        }
        assert_eq!(method_label(""), "OTHER");
    }

    #[test]
    fn matching_is_case_sensitive() {
        // HTTP methods are case sensitive, and `get` is not `GET`. Folding
        // case here would silently merge a malformed request with a real one.
        assert_eq!(method_label("get"), "OTHER");
    }

    #[test]
    fn every_variant_is_listed_and_round_trips() {
        // `ALL_METHODS` is written by hand, so this is what stops a new
        // variant being added to the enum and left out of the list -- which
        // would make it unparseable and unlabelable while still compiling.
        let mut seen = std::collections::BTreeSet::new();
        for method in ALL_METHODS {
            assert!(seen.insert(*method), "{method} is listed twice");
            assert_eq!(method.as_str().parse::<HttpMethod>().unwrap(), *method);
        }

        // The other half: a `match` the compiler checks. Adding a variant
        // without extending `ALL_METHODS` leaves this arm list complete and
        // the count below wrong, so one of the two always fails.
        let counted = ALL_METHODS
            .iter()
            .filter(|method| match method {
                HttpMethod::Get
                | HttpMethod::Head
                | HttpMethod::Post
                | HttpMethod::Put
                | HttpMethod::Patch
                | HttpMethod::Delete
                | HttpMethod::Options
                | HttpMethod::Trace
                | HttpMethod::Connect
                | HttpMethod::Query
                | HttpMethod::Propfind
                | HttpMethod::Proppatch
                | HttpMethod::Mkcol
                | HttpMethod::Copy
                | HttpMethod::Move
                | HttpMethod::Lock
                | HttpMethod::Unlock => true,
            })
            .count();
        assert_eq!(counted, 17);
        assert_eq!(seen.len(), ALL_METHODS.len());
    }

    #[test]
    fn a_lower_case_method_is_told_what_to_write() {
        let err = "get".parse::<HttpMethod>().unwrap_err();
        assert_eq!(
            err,
            MethodError::NotUpperCase {
                written: "get".to_owned(),
                upper: "GET".to_owned(),
            }
        );
        let message = err.to_string();
        assert!(message.contains("case-sensitive"), "{message}");
        assert!(message.contains("use `GET`"), "{message}");
    }

    #[test]
    fn a_typo_is_a_typo_and_not_a_case_problem() {
        // `fetch` is neither a method nor a mis-cased one, so a message
        // telling the reader to write `FETCH` would be wrong.
        for written in ["FETCH", "fetch", "", "GET "] {
            assert!(
                matches!(written.parse::<HttpMethod>(), Err(MethodError::Unknown(_))),
                "`{written}` should be unknown"
            );
        }
    }

    #[test]
    fn a_method_round_trips_through_yaml_as_its_wire_spelling() {
        let yaml = serde_norway::to_string(&HttpMethod::Propfind).unwrap();
        assert_eq!(yaml.trim(), "PROPFIND");
        assert_eq!(
            serde_norway::from_str::<HttpMethod>(&yaml).unwrap(),
            HttpMethod::Propfind
        );
    }

    #[test]
    fn deserializing_a_bad_method_carries_the_reason() {
        let err = serde_norway::from_str::<HttpMethod>("get").unwrap_err();
        assert!(err.to_string().contains("case-sensitive"), "{err}");
        let unknown = serde_norway::from_str::<HttpMethod>("FETCH").unwrap_err();
        assert!(unknown.to_string().contains("typo guard"), "{unknown}");
    }
}
