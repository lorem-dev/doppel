//! `GET /api/v1/access`: what the caller may do.

mod common;

use common::{BASE_CONFIG, Call, Harness, proxy_json};

const ROOT: &str = "root-token-0000000000000000000000000";
const READER: &str = "reader-token-00000000000000000000000";

/// `BASE_CONFIG` with the global `access` block replaced.
///
/// `replacen` with a count of one: the document has an `access` key under
/// `admin` and can have one under a proxy, and a plain `replace` would edit
/// whichever came first in a way that reads as working.
fn with_access(block: &str) -> String {
    BASE_CONFIG.replacen("  access:\n    list: public\n    read: public\n", block, 1)
}

#[tokio::test]
async fn an_anonymous_caller_gets_the_public_actions_and_nothing_else() {
    // `BASE_CONFIG` makes `list` and `read` public and the four writes
    // admin-only, which is the shape an operator most often has.
    let harness = Harness::new();
    let body = Call::get("/api/v1/access")
        .send(harness.router())
        .await
        .json();

    assert_eq!(body["caller"]["kind"], "anonymous");
    assert_eq!(body["global"]["list"], true);
    assert_eq!(body["global"]["read"], true);
    for action in ["create", "update", "delete", "upload"] {
        assert_eq!(body["global"][action], false, "{action} must be refused");
    }
}

#[tokio::test]
async fn a_token_is_reported_by_its_own_name_and_group() {
    let harness = Harness::new();
    let body = Call::get("/api/v1/access")
        .token(READER)
        .send(harness.router())
        .await
        .json();

    assert_eq!(body["caller"]["kind"], "token");
    assert_eq!(body["caller"]["name"], "reader");
    assert_eq!(body["caller"]["group"], "user");
    // `reader` is in group `user`, and the writes want `admin`.
    assert_eq!(body["global"]["read"], true);
    assert_eq!(body["global"]["create"], false);
}

#[tokio::test]
async fn an_admin_token_may_do_everything() {
    let harness = Harness::new();
    let body = Call::get("/api/v1/access")
        .token(ROOT)
        .send(harness.router())
        .await
        .json();

    for action in ["list", "read", "create", "update", "delete", "upload"] {
        assert_eq!(body["global"][action], true, "{action} must be allowed");
    }
    assert_eq!(body["proxies"]["alpha"]["delete"], true);
}

#[tokio::test]
async fn the_proxy_map_names_every_proxy_for_a_caller_who_may_list() {
    let harness = Harness::new();
    let body = Call::get("/api/v1/access")
        .send(harness.router())
        .await
        .json();

    let proxies = body["proxies"].as_object().expect("a proxy map");
    let mut names: Vec<_> = proxies.keys().map(String::as_str).collect();
    names.sort_unstable();
    assert_eq!(names, ["alpha", "beta"]);
    assert_eq!(body["proxies"]["alpha"]["read"], true);
    assert_eq!(body["proxies"]["alpha"]["update"], false);
}

#[tokio::test]
async fn a_caller_who_may_not_list_gets_no_proxy_map_at_all() {
    // Keyed by proxy name, the map is a proxy listing by another route. An empty
    // object would still say "there are no proxies"; the key has to be gone.
    let config = with_access("  access:\n    list: [\"admin\"]\n    read: [\"admin\"]\n");
    let harness = Harness::with_config(&config);
    let body = Call::get("/api/v1/access")
        .send(harness.router())
        .await
        .json();

    assert_eq!(body["global"]["list"], false);
    assert!(
        body.get("proxies").is_none(),
        "a caller without `list` must not learn the proxy names: {body}"
    );
}

#[tokio::test]
async fn a_proxy_override_is_reported_instead_of_the_global_answer() {
    // The whole reason the map exists: `reader` may read `alpha` and no other
    // proxy, and a report built from the global block alone would say `false`
    // for both.
    let config = with_access("  access:\n    list: public\n    read: [\"admin\"]\n").replacen(
        "  - name: alpha\n    type: http\n    url: \"https://alpha.example.com/api/\"\n",
        "  - name: alpha\n    type: http\n    url: \"https://alpha.example.com/api/\"\n    \
         access:\n      read: [\"reader\"]\n",
        1,
    );
    let harness = Harness::with_config(&config);
    let body = Call::get("/api/v1/access")
        .token(READER)
        .send(harness.router())
        .await
        .json();

    assert_eq!(body["global"]["read"], false);
    assert_eq!(body["proxies"]["alpha"]["read"], true);
    assert_eq!(body["proxies"]["beta"]["read"], false);
}

#[tokio::test]
async fn a_public_configuration_reports_everything_true_for_anonymous() {
    let config = BASE_CONFIG.replacen("  tokens:", "  public: true\n  tokens:", 1);
    let harness = Harness::with_config(&config);
    let body = Call::get("/api/v1/access")
        .send(harness.router())
        .await
        .json();

    for action in ["list", "read", "create", "update", "delete", "upload"] {
        assert_eq!(body["global"][action], true, "{action} under public: true");
    }
}

#[tokio::test]
async fn the_report_agrees_with_what_the_request_actually_answers() {
    // The test that matters. The report is `authorize` evaluated, and this is
    // what catches it drifting from `authorize` as enforced: a reported `false`
    // has to come back 401 or 403, and a reported `true` must not.
    //
    // A harness per action, because `delete` really deletes and `create` really
    // creates -- sharing one would make the later actions depend on the earlier.
    for token in [None, Some(READER), Some(ROOT)] {
        let report = {
            let harness = Harness::new();
            let mut call = Call::get("/api/v1/access");
            if let Some(token) = token {
                call = call.token(token);
            }
            call.send(harness.router()).await.json()
        };

        for action in ["list", "read", "create", "update", "delete", "upload"] {
            let harness = Harness::new();
            // The revision `update` needs, read before the attempt so a refusal
            // is about access rather than about a missing `If-Match`.
            let revision = Call::get("/api/v1/proxies/alpha")
                .token(ROOT)
                .send(harness.router())
                .await
                .revision();

            let mut call = match action {
                "list" => Call::get("/api/v1/proxies"),
                "read" => Call::get("/api/v1/proxies/alpha"),
                "create" => Call::post("/api/v1/proxies")
                    .json(proxy_json("gamma", "https://gamma.example.com/")),
                "update" => Call::put("/api/v1/proxies/alpha")
                    .if_match(&revision)
                    .json(proxy_json("alpha", "https://moved.example.com/")),
                "delete" => Call::delete("/api/v1/proxies/alpha").if_match(&revision),
                "upload" => Call::post("/api/v1/proxies/alpha/templates/page.json.j2").raw("hello"),
                other => unreachable!("{other}"),
            };
            if let Some(token) = token {
                call = call.token(token);
            }
            let status = call.send(harness.router()).await.status;

            let permitted = report["global"][action]
                .as_bool()
                .unwrap_or_else(|| panic!("{action} is not a boolean in {report}"));
            let refused = status == 401 || status == 403;
            assert_eq!(
                permitted, !refused,
                "{action} as {token:?}: the report said {permitted} and the request answered \
                 {status}"
            );
        }
    }
}
