//! Who the caller is, and what they may do.

use axum::http::HeaderMap;
use doppel_core::config::{AdminConfig, ProxyConfig, Subjects};
use doppel_core::{Error, ErrorCode};

/// The caller, once the token has been resolved.
///
/// An absent token and an unrecognised one both land on `Anonymous`. They are
/// deliberately not distinguished: telling a caller that their token was
/// *recognised but wrong* versus *not recognised* would confirm which tokens
/// exist, and both cases answer 401 anyway.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Caller {
    Anonymous,
    Token { name: String, group: String },
}

/// The six things access is decided for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    List,
    Read,
    Create,
    Update,
    Delete,
    Upload,
}

impl Action {
    fn as_str(self) -> &'static str {
        match self {
            Self::List => "list",
            Self::Read => "read",
            Self::Create => "create",
            Self::Update => "update",
            Self::Delete => "delete",
            Self::Upload => "upload",
        }
    }

    /// Whether a per-proxy `access` block may override this action.
    ///
    /// `list` and `create` are not about one proxy, so a proxy cannot have an
    /// opinion about them. `ProxyAccessConfig` already makes that unspellable
    /// in configuration; this keeps the same rule where the decision is made.
    fn overridable(self) -> bool {
        matches!(
            self,
            Self::Read | Self::Update | Self::Delete | Self::Upload
        )
    }
}

/// Resolve the caller from the request headers.
///
/// A malformed header -- no `Bearer` prefix, non-ASCII bytes, an unknown token
/// -- is anonymous rather than an error, so the access decision has exactly one
/// place where it can refuse.
#[must_use]
pub fn caller_from_headers(admin: &AdminConfig, headers: &HeaderMap) -> Caller {
    let Some(value) = headers.get(admin.auth.header.as_str()) else {
        return Caller::Anonymous;
    };
    let Ok(value) = value.to_str() else {
        return Caller::Anonymous;
    };
    let Some(token) = value.strip_prefix("Bearer ") else {
        return Caller::Anonymous;
    };
    let token = token.trim();

    admin
        .tokens
        .iter()
        .find(|t| t.token == token)
        .map_or(Caller::Anonymous, |t| Caller::Token {
            name: t.name.to_string(),
            group: t.group.to_string(),
        })
}

/// Decide whether `caller` may perform `action`, optionally against `proxy`.
///
/// The proxy's own `access` block wins over the global one for the four
/// actions it may override. Callers must run this *before* checking whether
/// the named proxy exists: otherwise a caller without read access could tell a
/// real proxy from an invented one by whether they got 404 or 403, and probe
/// the configuration one name at a time.
pub fn authorize(
    admin: &AdminConfig,
    proxy: Option<&ProxyConfig>,
    action: Action,
    caller: &Caller,
) -> Result<(), Error> {
    let subjects = effective_subjects(admin, proxy, action);

    let names = match subjects {
        Subjects::Public => return Ok(()),
        Subjects::Names(names) => names,
    };

    match caller {
        Caller::Anonymous => Err(Error::new(
            ErrorCode::Unauthorized,
            format!(
                "`{}` requires a token in the `{}` header",
                action.as_str(),
                admin.auth.header
            ),
        )),
        Caller::Token { name, group } => {
            if names.iter().any(|s| s == name || s == group) {
                Ok(())
            } else {
                Err(Error::new(
                    ErrorCode::Forbidden,
                    format!("token `{name}` may not `{}`", action.as_str()),
                ))
            }
        }
    }
}

fn effective_subjects<'a>(
    admin: &'a AdminConfig,
    proxy: Option<&'a ProxyConfig>,
    action: Action,
) -> &'a Subjects {
    if action.overridable()
        && let Some(overrides) = proxy.and_then(|p| p.access.as_ref())
    {
        {
            let chosen = match action {
                Action::Read => overrides.read.as_ref(),
                Action::Update => overrides.update.as_ref(),
                Action::Delete => overrides.delete.as_ref(),
                Action::Upload => overrides.upload.as_ref(),
                Action::List | Action::Create => None,
            };
            if let Some(subjects) = chosen {
                return subjects;
            }
        }
    }

    let access = &admin.access;
    match action {
        Action::List => &access.list,
        Action::Read => &access.read,
        Action::Create => &access.create,
        Action::Update => &access.update,
        Action::Delete => &access.delete,
        Action::Upload => &access.upload,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use doppel_core::config::{Config, load_from_str};

    const CONFIG: &str = r#"
server:
  host: "127.0.0.1"
  port: 8080
admin:
  host: "127.0.0.1"
  port: 8081
  tokens:
    - name: alice
      group: admin
      token: alice-token
    - name: bob
      group: readers
      token: bob-token
  access:
    list: public
    read: readers
    create: ["admin"]
    update: alice
    delete: admin
    upload: admin
  upload:
    limit: 1Mi
proxies:
  - name: open
    type: http
    url: "https://example.com/"
  - name: guarded
    type: http
    url: "https://example.com/"
    resolve:
      type: header
      header: X-Proxy-Name
    access:
      read: ["bob"]
      update: public
"#;

    fn config() -> Config {
        load_from_str(CONFIG).expect("fixture must parse")
    }

    fn alice() -> Caller {
        Caller::Token {
            name: "alice".to_owned(),
            group: "admin".to_owned(),
        }
    }

    fn bob() -> Caller {
        Caller::Token {
            name: "bob".to_owned(),
            group: "readers".to_owned(),
        }
    }

    fn headers(value: Option<&str>) -> HeaderMap {
        let mut map = HeaderMap::new();
        if let Some(value) = value {
            map.insert("x-proxy-authorization", value.parse().unwrap());
        }
        map
    }

    #[test]
    fn a_public_action_needs_no_token() {
        let c = config();
        assert!(authorize(&c.admin, None, Action::List, &Caller::Anonymous).is_ok());
    }

    #[test]
    fn a_group_name_satisfies_a_named_subject() {
        let c = config();
        // `create` names the group `admin`, and alice is in it.
        assert!(authorize(&c.admin, None, Action::Create, &alice()).is_ok());
    }

    #[test]
    fn a_token_name_satisfies_a_named_subject() {
        let c = config();
        // `update` names alice by token name rather than by group.
        assert!(authorize(&c.admin, None, Action::Update, &alice()).is_ok());
    }

    #[test]
    fn a_token_outside_the_named_subjects_is_forbidden() {
        let c = config();
        let err = authorize(&c.admin, None, Action::Delete, &bob()).unwrap_err();
        assert_eq!(err.code, ErrorCode::Forbidden);
        assert_eq!(err.status(), 403);
    }

    #[test]
    fn anonymous_against_a_guarded_action_is_unauthorized_not_forbidden() {
        // The two are deliberately distinct: 401 says "identify yourself",
        // 403 says "you did, and it is not enough". A caller fixes them
        // differently.
        let c = config();
        let err = authorize(&c.admin, None, Action::Delete, &Caller::Anonymous).unwrap_err();
        assert_eq!(err.code, ErrorCode::Unauthorized);
        assert_eq!(err.status(), 401);
        assert!(
            err.message.contains("X-Proxy-Authorization"),
            "the message should name the header to use, got {}",
            err.message
        );
    }

    #[test]
    fn a_proxy_override_beats_the_global_for_the_actions_it_may_cover() {
        let c = config();
        let guarded = c.proxies.iter().find(|p| p.name == "guarded").unwrap();

        // Globally `read` is the `readers` group, which alice is not in.
        assert!(authorize(&c.admin, None, Action::Read, &alice()).is_err());
        // The proxy overrides `read` to name bob, so alice is still refused
        // and bob is allowed -- by name, not by his group.
        assert!(authorize(&c.admin, Some(guarded), Action::Read, &alice()).is_err());
        assert!(authorize(&c.admin, Some(guarded), Action::Read, &bob()).is_ok());

        // The override makes `update` public on this proxy alone.
        assert!(authorize(&c.admin, Some(guarded), Action::Update, &Caller::Anonymous).is_ok());
        assert!(authorize(&c.admin, None, Action::Update, &Caller::Anonymous).is_err());
    }

    #[test]
    fn a_proxy_without_an_override_falls_back_to_the_global() {
        let c = config();
        let open = c.proxies.iter().find(|p| p.name == "open").unwrap();
        assert!(authorize(&c.admin, Some(open), Action::Read, &bob()).is_ok());
        assert!(authorize(&c.admin, Some(open), Action::Read, &alice()).is_err());
    }

    #[test]
    fn a_proxy_cannot_override_list_or_create() {
        let c = config();
        let guarded = c.proxies.iter().find(|p| p.name == "guarded").unwrap();
        // Passing the proxy must change nothing for these two, whatever its
        // override block says.
        assert_eq!(
            authorize(&c.admin, Some(guarded), Action::List, &Caller::Anonymous).is_ok(),
            authorize(&c.admin, None, Action::List, &Caller::Anonymous).is_ok()
        );
        assert_eq!(
            authorize(&c.admin, Some(guarded), Action::Create, &bob()).is_err(),
            authorize(&c.admin, None, Action::Create, &bob()).is_err()
        );
    }

    #[test]
    fn a_known_token_resolves_to_its_name_and_group() {
        let c = config();
        assert_eq!(
            caller_from_headers(&c.admin, &headers(Some("Bearer alice-token"))),
            alice()
        );
    }

    #[test]
    fn an_unknown_token_is_anonymous_so_it_answers_401_not_403() {
        // Distinguishing "recognised but wrong" from "not recognised" would
        // confirm which tokens exist.
        let c = config();
        assert_eq!(
            caller_from_headers(&c.admin, &headers(Some("Bearer nope"))),
            Caller::Anonymous
        );
    }

    #[test]
    fn a_malformed_header_is_anonymous_rather_than_a_panic() {
        let c = config();
        for value in ["alice-token", "Basic alice-token", "Bearer", ""] {
            assert_eq!(
                caller_from_headers(&c.admin, &headers(Some(value))),
                Caller::Anonymous,
                "`{value}` should not resolve to a caller"
            );
        }
        assert_eq!(
            caller_from_headers(&c.admin, &headers(None)),
            Caller::Anonymous
        );
    }

    #[test]
    fn the_configured_header_name_is_the_one_read() {
        let c = config();
        let mut map = HeaderMap::new();
        map.insert("authorization", "Bearer alice-token".parse().unwrap());
        assert_eq!(
            caller_from_headers(&c.admin, &map),
            Caller::Anonymous,
            "a token in the wrong header must not authenticate"
        );
    }
}
