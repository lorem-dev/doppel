//! Upstream base URLs, parsed rather than validated.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The base URL a proxy forwards to.
///
/// Absolute, `http` or `https`, with no query string and no fragment. The
/// value is kept parsed rather than as text, so the request path never parses
/// it again and there is no second place where an unusable URL could surface.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UpstreamUrl(reqwest::Url);

/// Why an upstream URL was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UrlError {
    #[error("url must be absolute: {reason}")]
    NotAbsolute { reason: String },
    #[error("url scheme must be http or https, not `{0}`")]
    BadScheme(String),
    /// The forwarding path replaces the query wholesale with the incoming
    /// request's, so a query configured here would be dropped on every
    /// request rather than merged. Refusing the configuration is simpler and
    /// more honest than teaching the URL builder to merge two query strings.
    #[error("a query string or fragment is not supported on an upstream base url")]
    HasQueryOrFragment,
}

impl UpstreamUrl {
    /// Check a string and keep it parsed, or say why not.
    pub fn parse(value: &str) -> Result<Self, UrlError> {
        let url = reqwest::Url::parse(value).map_err(|err| UrlError::NotAbsolute {
            reason: err.to_string(),
        })?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(UrlError::BadScheme(url.scheme().to_owned()));
        }
        if url.query().is_some() || url.fragment().is_some() {
            return Err(UrlError::HasQueryOrFragment);
        }
        Ok(Self(url))
    }

    #[must_use]
    pub fn as_url(&self) -> &reqwest::Url {
        &self.0
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    #[must_use]
    pub fn into_url(self) -> reqwest::Url {
        self.0
    }

    /// Whether the URL carries a username or a password.
    ///
    /// Legal, and occasionally what an operator means, but it puts a
    /// credential in a field that `GET /api/v1/proxies` returns and that
    /// appears in a log line naming the upstream. Reported as a startup
    /// advisory rather than refused.
    #[must_use]
    pub fn has_credentials(&self) -> bool {
        !self.0.username().is_empty() || self.0.password().is_some()
    }
}

impl fmt::Display for UpstreamUrl {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(self.0.as_str())
    }
}

impl FromStr for UpstreamUrl {
    type Err = UrlError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl Serialize for UpstreamUrl {
    /// The parsed form, which is normalised: `https://example.com` comes back
    /// as `https://example.com/`. That is deliberate. The revision is
    /// computed over what this writes, and two documents naming the same
    /// upstream in two spellings must not hash apart.
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.0.as_str())
    }
}

impl<'de> Deserialize<'de> for UpstreamUrl {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let value = String::deserialize(d)?;
        Self::parse(&value).map_err(serde::de::Error::custom)
    }
}

impl utoipa::PartialSchema for UpstreamUrl {
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
        utoipa::openapi::schema::ObjectBuilder::new()
            .schema_type(utoipa::openapi::schema::Type::String)
            .description(Some(
                "An absolute http or https base url, with no query string or \
                 fragment.",
            ))
            .examples([serde_json::json!("https://example.com/api/")])
            .into()
    }
}

impl utoipa::ToSchema for UpstreamUrl {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_urls_a_proxy_actually_forwards_to_are_accepted() {
        for value in [
            "https://example.com/",
            "http://127.0.0.1:8080/",
            "https://example.com/api/v1/",
            "https://user:secret@example.com/",
        ] {
            assert!(UpstreamUrl::parse(value).is_ok(), "`{value}` should parse");
        }
    }

    #[test]
    fn a_relative_url_is_refused_as_not_absolute() {
        // This was the first half of V8. The message says "absolute" rather
        // than repeating the parser's, because "relative URL without a base"
        // describes the parser's state and not the configuration's problem.
        for value in ["/api/", "example.com", ""] {
            let err = UpstreamUrl::parse(value).unwrap_err();
            assert!(
                matches!(err, UrlError::NotAbsolute { .. }),
                "`{value}`: {err:?}"
            );
            assert!(err.to_string().contains("must be absolute"), "{err}");
        }
    }

    #[test]
    fn a_scheme_doppel_cannot_forward_over_is_refused() {
        for value in ["ftp://example.com/", "file:///etc/passwd"] {
            let err = UpstreamUrl::parse(value).unwrap_err();
            assert!(matches!(err, UrlError::BadScheme(_)), "`{value}`: {err:?}");
        }
    }

    #[test]
    fn a_query_or_fragment_is_refused_because_it_would_be_dropped() {
        // This was V32. The forwarding path replaces the query with the
        // incoming request's, so one configured here is not merged, it is
        // discarded silently on every request.
        for value in [
            "https://example.com/?a=1",
            "https://example.com/#anchor",
            "https://example.com/api/?",
        ] {
            assert_eq!(
                UpstreamUrl::parse(value),
                Err(UrlError::HasQueryOrFragment),
                "`{value}` must be refused"
            );
        }
    }

    #[test]
    fn credentials_are_legal_and_reported() {
        assert!(
            UpstreamUrl::parse("https://user:secret@example.com/")
                .unwrap()
                .has_credentials()
        );
        assert!(
            UpstreamUrl::parse("https://user@example.com/")
                .unwrap()
                .has_credentials()
        );
        assert!(
            !UpstreamUrl::parse("https://example.com/")
                .unwrap()
                .has_credentials()
        );
    }

    #[test]
    fn a_url_round_trips_through_yaml_in_its_normalised_form() {
        // Normalising is what makes the round trip stable: `example.com` with
        // no path comes back with one, and then stays put.
        let url = UpstreamUrl::parse("https://example.com").unwrap();
        assert_eq!(url.as_str(), "https://example.com/");
        let yaml = serde_norway::to_string(&url).unwrap();
        assert_eq!(serde_norway::from_str::<UpstreamUrl>(&yaml).unwrap(), url);
        // And the second pass produces the same text as the first.
        let again = serde_norway::from_str::<UpstreamUrl>(&yaml).unwrap();
        assert_eq!(serde_norway::to_string(&again).unwrap(), yaml);
    }

    #[test]
    fn deserializing_a_bad_url_carries_the_reason() {
        let err = serde_norway::from_str::<UpstreamUrl>("\"ftp://example.com/\"").unwrap_err();
        assert!(err.to_string().contains("http or https"), "{err}");
    }
}
