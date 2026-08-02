//! Removing credentials from a URL before it is shown to anyone.

/// A URL with its userinfo removed.
///
/// Two things in this configuration carry credentials in a URL. A proxy's
/// `url` accepts `https://user:secret@host/` and no validation rule refuses
/// it, so an upstream behind basic auth can be configured that way. A Sentry
/// DSN puts the key that authorises sending events in exactly the same
/// position. Anything that displays either to someone not already trusted
/// with the configuration -- `/status`, which is public, and any log line or
/// error message -- goes through this.
///
/// Both the username and the password go. For a proxy URL the username is
/// half a credential; for a Sentry DSN it *is* the credential, so masking
/// only the password would leave the interesting part in place.
///
/// A string that does not parse is redacted whole. It should not be
/// reachable, because validation parses every proxy URL, but the safe
/// failure for a redactor is to reveal less rather than to fall back to the
/// input it could not understand.
#[must_use]
pub fn redact_credentials(url: &str) -> String {
    const REDACTED: &str = "<redacted>";

    let Ok(mut parsed) = reqwest::Url::parse(url) else {
        return REDACTED.to_owned();
    };
    if parsed.username().is_empty() && parsed.password().is_none() {
        return url.to_owned();
    }
    // Both setters fail only for a URL that cannot have a host -- `mailto:`
    // and friends -- which proxy validation already rejects.
    if parsed.set_password(None).is_err() || parsed.set_username("").is_err() {
        return REDACTED.to_owned();
    }
    parsed.to_string()
}

#[cfg(test)]
mod redaction_tests {
    use super::redact_credentials;

    #[test]
    fn a_plain_url_is_returned_unchanged() {
        // Byte for byte, not merely equivalent: `/status` shows this to an
        // operator comparing it against what they configured.
        let url = "https://alpha.example.com/api/v1/";
        assert_eq!(redact_credentials(url), url);
    }

    #[test]
    fn a_password_is_removed_and_the_host_is_kept() {
        let redacted = redact_credentials("https://user:secret@alpha.example.com/api/");
        assert!(!redacted.contains("secret"), "{redacted}");
        assert!(!redacted.contains("user"), "{redacted}");
        assert!(redacted.contains("alpha.example.com"), "{redacted}");
    }

    #[test]
    fn a_username_without_a_password_is_also_removed() {
        let redacted = redact_credentials("https://token@alpha.example.com/api/");
        assert!(!redacted.contains("token"), "{redacted}");
    }

    #[test]
    fn an_at_sign_outside_the_authority_is_not_treated_as_credentials() {
        // The `@` here belongs to the path. A hand-rolled "split on @"
        // redactor mangles this one; the real parser does not.
        let url = "https://alpha.example.com/api/user@example.com/";
        assert_eq!(redact_credentials(url), url);
    }

    #[test]
    fn an_unparsable_url_is_redacted_whole_rather_than_echoed() {
        assert_eq!(redact_credentials("not a url"), "<redacted>");
    }
}
