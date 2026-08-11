//! Who the caller is, and what they may do.

use axum::http::HeaderMap;
use doppel_core::config::{AdminConfig, EnvTokens, ProxyConfig, Subjects};
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
/// The access policy in force.
///
/// The configuration the last reload put into effect, which is not the same
/// thing as whatever is in the store right now. Every handler authenticates
/// and authorizes against this and reads its *data* from the store.
///
/// The distinction is the whole of a security property. The proxies and
/// templates handlers used to do both against a config loaded from the store
/// per request, so someone who could write the configuration out of band --
/// but held no token -- could add a token for themselves and use it on the
/// very next request, with no reload and nobody's approval. The reload
/// endpoint was already written this way and has a test saying why; the other
/// eight handlers were not, and the test did not reach them.
#[must_use]
pub fn policy(state: &crate::AdminState) -> std::sync::Arc<doppel_core::Config> {
    std::sync::Arc::clone(&state.holder().load().config)
}

/// Resolve the caller against the configured tokens alone.
///
/// Kept for the tests that have no environment to speak of. Everything
/// serving a request goes through `caller_from_headers_with_env`.
#[must_use]
pub fn caller_from_headers(admin: &AdminConfig, headers: &HeaderMap) -> Caller {
    caller_from_headers_with_env(admin, &EnvTokens::default(), headers)
}

/// Resolve the caller against the environment's tokens and then the
/// configured ones.
///
/// The environment is searched first, which is what "overrides on conflict"
/// means once it is made precise. A name given in both resolves to the
/// environment's group, and a *value* given in both resolves to the
/// environment's name -- so a deployment can replace a configured token
/// without editing the document that names it.
///
/// A configured token whose name the environment also claims is skipped
/// entirely rather than left reachable by its own value. Leaving it would
/// mean two live secrets for one identity, one of which nobody remembers
/// issuing.
#[must_use]
pub fn caller_from_headers_with_env(
    admin: &AdminConfig,
    env: &EnvTokens,
    headers: &HeaderMap,
) -> Caller {
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

    env.find(token)
        .or_else(|| {
            admin
                .tokens
                .iter()
                .filter(|t| !env.shadows(&t.name))
                // `Token::matches` rather than `==`: the comparison against a
                // presented secret is the one place where stopping at the
                // first differing byte is measurable from outside.
                .find(|t| t.token.matches(token))
        })
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
            if names
                .iter()
                .any(|s| s.as_str() == name || s.as_str() == group)
            {
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

/// So a public configuration can be answered with a borrow like any other.
const PUBLIC: Subjects = Subjects::Public;

fn effective_subjects<'a>(
    admin: &'a AdminConfig,
    proxy: Option<&'a ProxyConfig>,
    action: Action,
) -> &'a Subjects {
    // `public: true`, or `groups: []`, makes every action public -- including a
    // proxy's overrides, which is why this comes before them. A per-proxy
    // `read: admin` under a public configuration would otherwise be the one
    // thing still asking for a token, which is not what "public" was set for.
    if admin.is_public() {
        return &PUBLIC;
    }

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

    /// The two token values `CONFIG` below carries.
    ///
    /// Named because the value is 36 characters -- long enough to satisfy the
    /// `Token` bound -- and repeating that inline made every assertion about
    /// authentication wider than the assertion itself.
    const ALICE: &str = "alice-token-000000000000000000000000";
    const BOB: &str = "bob-token-00000000000000000000000000";

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
      token: alice-token-000000000000000000000000
    - name: bob
      group: readers
      token: bob-token-00000000000000000000000000
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
    fn the_named_tokens_are_the_ones_the_fixture_configures() {
        // `CONFIG` is a raw literal and cannot interpolate the constants, so
        // this is what stops the two copies drifting apart -- without it, a
        // renamed token would quietly make every authentication test assert
        // that an unknown token is anonymous.
        assert!(CONFIG.contains(ALICE), "alice's token is not in CONFIG");
        assert!(CONFIG.contains(BOB), "bob's token is not in CONFIG");
    }

    #[test]
    fn a_known_token_resolves_to_its_name_and_group() {
        let c = config();
        assert_eq!(
            caller_from_headers(&c.admin, &headers(Some(&format!("Bearer {ALICE}")))),
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
        for value in [ALICE, &format!("Basic {ALICE}"), "Bearer", ""] {
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

    /// `public: true` answers every action publicly, and the fixture's `access`
    /// is deliberately restrictive so this cannot pass by the defaults being
    /// permissive already. Enumerated over every action rather than spot-checked:
    /// the point of the flag is that nothing is left needing a token.
    #[test]
    fn a_public_admin_api_authorises_every_action_for_anyone() {
        let config = load_from_str(&CONFIG.replacen("  access:", "  public: true\n  access:", 1))
            .expect("fixture must parse");
        for action in [
            Action::List,
            Action::Read,
            Action::Create,
            Action::Update,
            Action::Delete,
            Action::Upload,
        ] {
            assert!(
                authorize(&config.admin, None, action, &Caller::Anonymous).is_ok(),
                "`{}` must be public",
                action.as_str()
            );
        }
    }

    /// `groups: []` names nobody, so it means the same thing. Worth its own test
    /// because it is the spelling an operator reaches by trying to lock the
    /// configuration down, which is the opposite of what it does.
    #[test]
    fn an_empty_groups_list_authorises_every_action_too() {
        let config = load_from_str(&CONFIG.replacen("  access:", "  groups: []\n  access:", 1))
            .expect("fixture must parse");
        assert!(config.admin.is_public());
        assert!(authorize(&config.admin, None, Action::Delete, &Caller::Anonymous).is_ok());
    }

    /// A per-proxy override cannot claw back a token requirement under a public
    /// configuration -- it would be the one thing still asking for a token, which
    /// is not what the flag was set for.
    #[test]
    fn a_proxy_override_does_not_survive_a_public_admin_api() {
        let config = load_from_str(&CONFIG.replacen("  access:", "  public: true\n  access:", 1))
            .expect("fixture must parse");
        let locked = config
            .proxies
            .iter()
            .find(|proxy| proxy.access.is_some())
            .expect("the fixture must define a proxy with overrides");
        assert!(
            authorize(
                &config.admin,
                Some(locked),
                Action::Read,
                &Caller::Anonymous
            )
            .is_ok()
        );
    }

    #[test]
    fn the_configured_header_name_is_the_one_read() {
        let c = config();
        let mut map = HeaderMap::new();
        map.insert("authorization", format!("Bearer {ALICE}").parse().unwrap());
        assert_eq!(
            caller_from_headers(&c.admin, &map),
            Caller::Anonymous,
            "a token in the wrong header must not authenticate"
        );
    }
}
