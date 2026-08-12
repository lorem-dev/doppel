//! What the caller may do, reported to the caller.
//!
//! The dashboard needs this: a button for an action the server will refuse is
//! worse than no button, because the operator finds out by trying. Nothing else
//! in the API says anything about the *caller's* rights -- every other endpoint
//! either performs an action or refuses it -- so this is the one endpoint whose
//! subject is the request rather than the configuration.

use axum::Router;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::routing::get;
use doppel_core::config::ProxyConfig;
use serde::Serialize;
use std::collections::BTreeMap;

use crate::access::{self, Action, Caller};
use crate::state::AdminState;

pub fn routes() -> Router<AdminState> {
    Router::new().route("/api/v1/access", get(rights))
}

/// Who the caller is, as far as this process is concerned.
///
/// Returning the token's own name and group leaks nothing -- the caller
/// presented it -- and it is what lets the dashboard say who is signed in
/// rather than only that somebody is.
#[derive(Debug, Serialize, utoipa::ToSchema)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum CallerView {
    /// No token, or one this configuration does not recognise. The two are not
    /// distinguished, for the reason `Caller` gives: telling them apart would
    /// confirm which tokens exist.
    Anonymous,
    Token {
        name: String,
        group: String,
    },
}

/// The six actions, as booleans.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ActionRights {
    pub list: bool,
    pub read: bool,
    pub create: bool,
    pub update: bool,
    pub delete: bool,
    pub upload: bool,
}

/// The four actions a proxy's own `access` block may override.
///
/// `list` and `create` are not about one proxy, so they are absent here rather
/// than repeated per proxy -- the same reason `ProxyAccessConfig` cannot spell
/// them.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ProxyRights {
    pub read: bool,
    pub update: bool,
    pub delete: bool,
    pub upload: bool,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct AccessReport {
    pub caller: CallerView,
    /// The global `access` block, evaluated for this caller.
    pub global: ActionRights,
    /// Per proxy, where a proxy's own `access` overrides the global answer.
    ///
    /// Absent -- not empty -- for a caller who may not `list`. The map is keyed
    /// by proxy name, so returning it would be a proxy listing by another route,
    /// and `access.rs` is explicit that a caller without read access must not be
    /// able to tell a real proxy from an invented one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proxies: Option<BTreeMap<String, ProxyRights>>,
}

impl From<Caller> for CallerView {
    fn from(caller: Caller) -> Self {
        match caller {
            Caller::Anonymous => Self::Anonymous,
            Caller::Token { name, group } => Self::Token { name, group },
        }
    }
}

/// `GET /api/v1/access` -- what the caller may do.
///
/// Answers 200 for everybody, anonymous included: an endpoint whose purpose is
/// to report that the caller may do nothing cannot itself demand a right, and
/// there is nothing here a caller could not learn by attempting the six actions
/// and reading the statuses.
///
/// Every field is `authorize` evaluated, never a second reading of the `access`
/// blocks. A copy of that decision would be a copy that can disagree with the
/// one enforcing it, and the disagreement would reach the operator as a button
/// that does nothing -- or, worse, as a hidden action they were allowed to take.
#[utoipa::path(
    get, path = "/api/v1/access", tag = "process",
    responses((status = 200, description = "What the calling token may do", body = AccessReport)),
    security(("token" = [])),
)]
pub(crate) async fn rights(
    State(state): State<AdminState>,
    headers: HeaderMap,
) -> axum::Json<AccessReport> {
    // The policy the last reload put into effect, like every other handler --
    // not whatever is in the store right now. Reporting rights from an unloaded
    // document would describe a server nobody is running.
    let config = access::policy(&state);
    let caller = access::caller_from_headers_with_env(&config.admin, state.env_tokens(), &headers);

    let permits = |proxy: Option<&ProxyConfig>, action: Action| {
        access::authorize(&config.admin, proxy, action, &caller).is_ok()
    };

    let global = ActionRights {
        list: permits(None, Action::List),
        read: permits(None, Action::Read),
        create: permits(None, Action::Create),
        update: permits(None, Action::Update),
        delete: permits(None, Action::Delete),
        upload: permits(None, Action::Upload),
    };

    let proxies = global.list.then(|| {
        config
            .proxies
            .iter()
            .map(|proxy| {
                (
                    proxy.name.to_string(),
                    ProxyRights {
                        read: permits(Some(proxy), Action::Read),
                        update: permits(Some(proxy), Action::Update),
                        delete: permits(Some(proxy), Action::Delete),
                        upload: permits(Some(proxy), Action::Upload),
                    },
                )
            })
            .collect()
    });

    axum::Json(AccessReport {
        caller: caller.into(),
        global,
        proxies,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_anonymous_caller_is_reported_without_a_name() {
        let json = serde_json::to_value(CallerView::from(Caller::Anonymous)).unwrap();
        assert_eq!(json, serde_json::json!({"kind": "anonymous"}));
    }

    #[test]
    fn a_token_is_reported_by_its_name_and_group() {
        let json = serde_json::to_value(CallerView::from(Caller::Token {
            name: "ci".to_owned(),
            group: "ops".to_owned(),
        }))
        .unwrap();
        assert_eq!(
            json,
            serde_json::json!({"kind": "token", "name": "ci", "group": "ops"})
        );
    }

    #[test]
    fn an_absent_proxy_map_is_absent_rather_than_empty() {
        // An empty object says "there are no proxies", which is a statement about
        // the configuration. The point of withholding the map is to say nothing
        // at all, so the key has to be gone.
        let report = AccessReport {
            caller: CallerView::Anonymous,
            global: ActionRights {
                list: false,
                read: false,
                create: false,
                update: false,
                delete: false,
                upload: false,
            },
            proxies: None,
        };
        let json = serde_json::to_value(&report).unwrap();
        assert!(
            json.get("proxies").is_none(),
            "proxies must be absent, not null or empty: {json}"
        );
    }
}
