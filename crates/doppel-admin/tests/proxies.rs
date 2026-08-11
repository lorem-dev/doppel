//! Proxy CRUD over the admin API.

mod common;

use common::{Call, Harness, RacingStore, assert_absent, proxy_json};
use serde_json::json;

const ROOT: &str = "root-token-0000000000000000000000000";

#[tokio::test]
async fn list_returns_every_proxy_with_its_revision() {
    let harness = Harness::new();
    let reply = Call::get("/api/v1/proxies").send(harness.router()).await;

    assert_eq!(reply.status, 200);
    let body = reply.json();
    let proxies = body["proxies"].as_array().expect("proxies is an array");
    assert_eq!(proxies.len(), 2);
    assert_eq!(proxies[0]["proxy"]["name"], "alpha");
    assert_eq!(proxies[1]["proxy"]["name"], "beta");
    // Distinct proxies have distinct revisions; equal ones would mean the
    // revision is not derived from the proxy at all.
    assert_ne!(proxies[0]["revision"], proxies[1]["revision"]);
    for entry in proxies {
        let revision = entry["revision"].as_str().expect("revision is a string");
        assert_eq!(revision.len(), 16, "revision is 16 hex digits: {revision}");
        assert!(revision.chars().all(|c| c.is_ascii_hexdigit()));
    }
}

#[tokio::test]
async fn read_returns_the_proxy_and_an_etag_matching_the_body_revision() {
    let harness = Harness::new();
    let reply = Call::get("/api/v1/proxies/alpha")
        .send(harness.router())
        .await;

    assert_eq!(reply.status, 200);
    assert_eq!(reply.json()["proxy"]["name"], "alpha");
    // The ETag is the body revision, quoted. A client that copies one into
    // `If-Match` and a client that copies the other must be doing the same
    // thing.
    assert_eq!(
        reply.etag.as_deref(),
        Some(&*format!("\"{}\"", reply.revision()))
    );
}

#[tokio::test]
async fn read_of_a_missing_proxy_is_not_found() {
    let harness = Harness::new();
    let reply = Call::get("/api/v1/proxies/nope")
        .send(harness.router())
        .await;

    assert_eq!(reply.status, 404);
    assert_eq!(reply.error_code(), "NOT_FOUND");
}

#[tokio::test]
async fn create_adds_the_proxy_and_returns_its_location_and_revision() {
    let harness = Harness::new();
    let reply = Call::post("/api/v1/proxies")
        .token(ROOT)
        .json(proxy_json("gamma", "https://gamma.example.com/api/"))
        .send(harness.router())
        .await;

    assert_eq!(reply.status, 201, "{}", reply.body);
    assert_eq!(reply.location.as_deref(), Some("/api/v1/proxies/gamma"));
    assert_eq!(reply.json()["proxy"]["name"], "gamma");
    assert_eq!(
        reply.etag.as_deref(),
        Some(&*format!("\"{}\"", reply.revision()))
    );

    let names: Vec<_> = harness
        .stored()
        .proxies
        .iter()
        .map(|p| p.name.clone())
        .collect();
    assert_eq!(names, ["alpha", "beta", "gamma"]);
}

#[tokio::test]
async fn create_over_an_existing_name_is_conflict() {
    let harness = Harness::new();
    let reply = Call::post("/api/v1/proxies")
        .token(ROOT)
        .json(proxy_json("alpha", "https://other.example.com/api/"))
        .send(harness.router())
        .await;

    assert_eq!(reply.status, 409);
    assert_eq!(reply.error_code(), "CONFLICT");
    // The existing proxy is untouched.
    assert_eq!(
        harness.stored().proxies[0].url.as_str(),
        "https://alpha.example.com/api/"
    );
}

#[tokio::test]
async fn create_carrying_a_revision_is_rejected() {
    let harness = Harness::new();
    let mut body = proxy_json("gamma", "https://gamma.example.com/api/");
    body["revision"] = json!("0123456789abcdef");
    let reply = Call::post("/api/v1/proxies")
        .token(ROOT)
        .json(body)
        .send(harness.router())
        .await;

    // A revision identifies a version of something that exists. Sending one
    // on a create is a client that meant to update, and silently ignoring it
    // would turn that mistake into an overwrite of someone else's proxy.
    assert_eq!(reply.status, 400, "{}", reply.body);
    assert_eq!(reply.error_code(), "CONFIG_INVALID");
    assert_eq!(harness.stored().proxies.len(), 2);
}

#[tokio::test]
async fn update_with_the_current_revision_replaces_the_proxy() {
    let harness = Harness::new();
    let read = Call::get("/api/v1/proxies/alpha")
        .send(harness.router())
        .await;
    let revision = read.revision();

    let reply = Call::put("/api/v1/proxies/alpha")
        .token(ROOT)
        .if_match(format!("\"{revision}\""))
        .json(proxy_json("alpha", "https://alpha-2.example.com/api/"))
        .send(harness.router())
        .await;

    assert_eq!(reply.status, 200, "{}", reply.body);
    assert_ne!(reply.revision(), revision, "the revision must move");
    assert_eq!(
        harness.stored().proxies[0].url.as_str(),
        "https://alpha-2.example.com/api/"
    );
    // The other proxy is untouched: an update is not a whole-config write.
    assert_eq!(
        harness.stored().proxies[1].url.as_str(),
        "https://beta.example.com/api/"
    );
}

#[tokio::test]
async fn update_accepts_the_revision_in_the_body_as_well_as_in_if_match() {
    let harness = Harness::new();
    let revision = Call::get("/api/v1/proxies/alpha")
        .send(harness.router())
        .await
        .revision();

    let mut body = proxy_json("alpha", "https://alpha-3.example.com/api/");
    body["revision"] = json!(revision);
    let reply = Call::put("/api/v1/proxies/alpha")
        .token(ROOT)
        .json(body)
        .send(harness.router())
        .await;

    assert_eq!(reply.status, 200, "{}", reply.body);
    assert_eq!(
        harness.stored().proxies[0].url.as_str(),
        "https://alpha-3.example.com/api/"
    );
}

#[tokio::test]
async fn update_of_a_missing_proxy_is_not_found() {
    let harness = Harness::new();
    let reply = Call::put("/api/v1/proxies/nope")
        .token(ROOT)
        .if_match("\"0123456789abcdef\"")
        .json(proxy_json("nope", "https://nope.example.com/api/"))
        .send(harness.router())
        .await;

    assert_eq!(reply.status, 404);
    assert_eq!(reply.error_code(), "NOT_FOUND");
}

#[tokio::test]
async fn update_with_a_stale_revision_is_revision_mismatch() {
    let harness = Harness::new();
    let stale = Call::get("/api/v1/proxies/alpha")
        .send(harness.router())
        .await
        .revision();

    // Someone else lands a change first.
    Call::put("/api/v1/proxies/alpha")
        .token(ROOT)
        .if_match(format!("\"{stale}\""))
        .json(proxy_json("alpha", "https://alpha-first.example.com/api/"))
        .send(harness.router())
        .await;

    let reply = Call::put("/api/v1/proxies/alpha")
        .token(ROOT)
        .if_match(format!("\"{stale}\""))
        .json(proxy_json("alpha", "https://alpha-second.example.com/api/"))
        .send(harness.router())
        .await;

    assert_eq!(reply.status, 409, "{}", reply.body);
    assert_eq!(reply.error_code(), "REVISION_MISMATCH");
    // The first write survives: a rejected update changes nothing.
    assert_eq!(
        harness.stored().proxies[0].url.as_str(),
        "https://alpha-first.example.com/api/"
    );
}

#[tokio::test]
async fn update_with_no_revision_at_all_is_revision_required() {
    let harness = Harness::new();
    let reply = Call::put("/api/v1/proxies/alpha")
        .token(ROOT)
        .json(proxy_json("alpha", "https://alpha-4.example.com/api/"))
        .send(harness.router())
        .await;

    // Not a mismatch -- nothing was compared. The client skipped the
    // precondition that stops a lost update, and 428 is the status that says
    // exactly that.
    assert_eq!(reply.status, 428, "{}", reply.body);
    assert_eq!(reply.error_code(), "REVISION_REQUIRED");
    assert_eq!(
        harness.stored().proxies[0].url.as_str(),
        "https://alpha.example.com/api/"
    );
}

#[tokio::test]
async fn if_match_star_is_rejected_rather_than_treated_as_a_wildcard() {
    let harness = Harness::new();
    let reply = Call::put("/api/v1/proxies/alpha")
        .token(ROOT)
        .if_match("*")
        .json(proxy_json("alpha", "https://alpha-5.example.com/api/"))
        .send(harness.router())
        .await;

    // RFC 9110 reads `If-Match: *` as "if the resource exists", which here
    // would mean "overwrite whatever is there" -- the precise thing the
    // precondition exists to prevent. Refusing it is the only reading that
    // does not quietly disable the check.
    assert_eq!(reply.status, 428, "{}", reply.body);
    assert_eq!(reply.error_code(), "REVISION_REQUIRED");
}

#[tokio::test]
async fn if_match_and_the_body_revision_must_agree() {
    let harness = Harness::new();
    let revision = Call::get("/api/v1/proxies/alpha")
        .send(harness.router())
        .await
        .revision();

    let mut body = proxy_json("alpha", "https://alpha-6.example.com/api/");
    body["revision"] = json!("0123456789abcdef");
    let reply = Call::put("/api/v1/proxies/alpha")
        .token(ROOT)
        .if_match(format!("\"{revision}\""))
        .json(body)
        .send(harness.router())
        .await;

    // Two different answers to the same question. Picking one would make the
    // client's other half silently meaningless.
    assert_eq!(reply.status, 400, "{}", reply.body);
    assert_eq!(reply.error_code(), "CONFIG_INVALID");
}

#[tokio::test]
async fn an_unparsable_revision_is_rejected() {
    let harness = Harness::new();
    let reply = Call::put("/api/v1/proxies/alpha")
        .token(ROOT)
        .if_match("\"not-hex\"")
        .json(proxy_json("alpha", "https://alpha-7.example.com/api/"))
        .send(harness.router())
        .await;

    assert_eq!(reply.status, 400, "{}", reply.body);
    assert_eq!(reply.error_code(), "CONFIG_INVALID");
}

#[tokio::test]
async fn update_that_renames_the_proxy_is_rejected() {
    let harness = Harness::new();
    let revision = Call::get("/api/v1/proxies/alpha")
        .send(harness.router())
        .await
        .revision();

    let reply = Call::put("/api/v1/proxies/alpha")
        .token(ROOT)
        .if_match(format!("\"{revision}\""))
        .json(proxy_json("renamed", "https://alpha.example.com/api/"))
        .send(harness.router())
        .await;

    // The name is the template directory. A rename through PUT would leave
    // the old directory orphaned and the new proxy with no templates, which
    // is not what anyone typing a new name into a body means.
    assert_eq!(reply.status, 400, "{}", reply.body);
    assert_eq!(reply.error_code(), "CONFIG_INVALID");
    assert_eq!(harness.stored().proxies[0].name, "alpha");
}

#[tokio::test]
async fn an_invalid_proxy_is_config_invalid_and_names_the_violation() {
    let harness = Harness::new();
    let revision = Call::get("/api/v1/proxies/alpha")
        .send(harness.router())
        .await
        .revision();

    let mut body = proxy_json("alpha", "https://alpha.example.com/api/");
    body["proxy"]["latency"] = json!({ "percentage": 0.5, "min": 2.0, "max": 1.0 });
    let reply = Call::put("/api/v1/proxies/alpha")
        .token(ROOT)
        .if_match(format!("\"{revision}\""))
        .json(body)
        .send(harness.router())
        .await;

    assert_eq!(reply.status, 400, "{}", reply.body);
    assert_eq!(reply.error_code(), "CONFIG_INVALID");
    // The message carries the violating path, not just "invalid".
    assert!(
        reply.body.contains("latency"),
        "message should name the violation: {}",
        reply.body
    );
    assert_eq!(
        harness.stored().proxies[0].url.as_str(),
        "https://alpha.example.com/api/"
    );
}

#[tokio::test]
async fn a_body_that_is_not_json_is_rejected_without_touching_the_store() {
    let harness = Harness::new();
    let reply = Call::post("/api/v1/proxies")
        .token(ROOT)
        .raw("{ not json")
        .send(harness.router())
        .await;

    assert_eq!(reply.status, 400, "{}", reply.body);
    assert_eq!(reply.error_code(), "CONFIG_INVALID");
    assert_eq!(harness.stored().proxies.len(), 2);
}

#[tokio::test]
async fn delete_removes_the_proxy() {
    let harness = Harness::new();
    let reply = Call::delete("/api/v1/proxies/beta")
        .token(ROOT)
        .send(harness.router())
        .await;

    assert_eq!(reply.status, 204, "{}", reply.body);
    assert!(reply.body.is_empty());
    let names: Vec<_> = harness
        .stored()
        .proxies
        .iter()
        .map(|p| p.name.clone())
        .collect();
    assert_eq!(names, ["alpha"]);
}

#[tokio::test]
async fn delete_of_a_missing_proxy_is_not_found() {
    let harness = Harness::new();
    let reply = Call::delete("/api/v1/proxies/nope")
        .token(ROOT)
        .send(harness.router())
        .await;

    assert_eq!(reply.status, 404);
    assert_eq!(reply.error_code(), "NOT_FOUND");
}

#[tokio::test]
async fn delete_honours_if_match_when_the_client_supplies_one() {
    let harness = Harness::new();
    let reply = Call::delete("/api/v1/proxies/beta")
        .token(ROOT)
        .if_match("\"0123456789abcdef\"")
        .send(harness.router())
        .await;

    assert_eq!(reply.status, 409, "{}", reply.body);
    assert_eq!(reply.error_code(), "REVISION_MISMATCH");
    assert_eq!(harness.stored().proxies.len(), 2);
}

#[tokio::test]
async fn delete_without_if_match_is_allowed() {
    // Unlike an update, a delete names its target completely: there are no
    // unread fields it could clobber, so there is no lost update to prevent.
    let harness = Harness::new();
    let reply = Call::delete("/api/v1/proxies/beta")
        .token(ROOT)
        .send(harness.router())
        .await;

    assert_eq!(reply.status, 204);
}

#[tokio::test]
async fn delete_drops_the_proxys_templates() {
    let harness = Harness::new();
    harness.write_template("beta", "body.json.j2", "{}");
    let kept = harness.template_path("alpha", "keep.json.j2");
    harness.write_template("alpha", "keep.json.j2", "{}");

    let reply = Call::delete("/api/v1/proxies/beta")
        .token(ROOT)
        .send(harness.router())
        .await;

    assert_eq!(reply.status, 204, "{}", reply.body);
    assert_absent(&harness.template_path("beta", "body.json.j2"));
    assert!(kept.exists(), "another proxy's templates must survive");
}

/// Rule V5 used to refuse this, so emptying a Doppel over the API meant
/// deleting every proxy but one and then editing the file by hand. An empty
/// proxy list is a legal configuration now: the delete goes through, and a
/// request afterwards is answered `503 NO_PROXIES_CONFIGURED` rather than the
/// deletion being blocked to keep that from happening.
#[tokio::test]
async fn deleting_the_last_proxy_is_allowed_and_leaves_none() {
    let (only_alpha, _) = common::BASE_CONFIG
        .split_once("  - name: beta")
        .expect("BASE_CONFIG defines beta");
    let harness = Harness::with_config(only_alpha);
    harness.write_template("alpha", "body.json.j2", "{}");

    let reply = Call::delete("/api/v1/proxies/alpha")
        .token(ROOT)
        .send(harness.router())
        .await;

    assert_eq!(reply.status, 204, "{}", reply.body);
    assert!(harness.stored().proxies.is_empty());
    // The write authorises dropping them, and the write happened.
    assert_absent(&harness.template_path("alpha", "body.json.j2"));
}

#[tokio::test]
async fn a_concurrent_edit_to_a_different_proxy_succeeds_after_a_retry() {
    let mut harness = Harness::new();
    let revision = Call::get("/api/v1/proxies/alpha")
        .send(harness.router())
        .await
        .revision();

    // The first save collides with a write to `beta`, which the client's
    // per-proxy revision says nothing about.
    harness.wrap_store(|inner| {
        std::sync::Arc::new(RacingStore::new(
            inner,
            vec![RacingStore::touch(
                "beta",
                "https://beta-moved.example.com/api/",
            )],
        ))
    });

    let reply = Call::put("/api/v1/proxies/alpha")
        .token(ROOT)
        .if_match(format!("\"{revision}\""))
        .json(proxy_json(
            "alpha",
            "https://alpha-retried.example.com/api/",
        ))
        .send(harness.router())
        .await;

    assert_eq!(reply.status, 200, "{}", reply.body);
    let stored = harness.stored();
    // Both writes survive: the retry rebuilt its change on top of the other
    // one rather than overwriting it.
    assert_eq!(
        stored.proxies[0].url.as_str(),
        "https://alpha-retried.example.com/api/"
    );
    assert_eq!(
        stored.proxies[1].url.as_str(),
        "https://beta-moved.example.com/api/"
    );
}

#[tokio::test]
async fn a_concurrent_edit_to_the_same_proxy_is_rejected() {
    let mut harness = Harness::new();
    let revision = Call::get("/api/v1/proxies/alpha")
        .send(harness.router())
        .await
        .revision();

    harness.wrap_store(|inner| {
        std::sync::Arc::new(RacingStore::new(
            inner,
            vec![RacingStore::touch(
                "alpha",
                "https://alpha-someone-else.example.com/api/",
            )],
        ))
    });

    let reply = Call::put("/api/v1/proxies/alpha")
        .token(ROOT)
        .if_match(format!("\"{revision}\""))
        .json(proxy_json("alpha", "https://alpha-mine.example.com/api/"))
        .send(harness.router())
        .await;

    // The retry re-reads and finds the client's revision no longer current.
    // This is the client's conflict and must surface, not be retried away.
    assert_eq!(reply.status, 409, "{}", reply.body);
    assert_eq!(reply.error_code(), "REVISION_MISMATCH");
    assert_eq!(
        harness.stored().proxies[0].url.as_str(),
        "https://alpha-someone-else.example.com/api/"
    );
}

#[tokio::test]
async fn unrelenting_contention_exhausts_the_bound_and_is_conflict() {
    let mut harness = Harness::new();
    let revision = Call::get("/api/v1/proxies/alpha")
        .send(harness.router())
        .await
        .revision();

    // A competing writer that never stops, but only ever touches `beta`, so
    // the per-proxy check keeps passing and only the whole-config CAS fails.
    // The bound is what makes this terminate at all.
    let edits = (0..64)
        .map(|i| {
            let url: &'static str =
                Box::leak(format!("https://beta-{i}.example.com/api/").into_boxed_str());
            RacingStore::touch("beta", url)
        })
        .collect();
    harness.wrap_store(|inner| std::sync::Arc::new(RacingStore::new(inner, edits)));

    let reply = Call::put("/api/v1/proxies/alpha")
        .token(ROOT)
        .if_match(format!("\"{revision}\""))
        .json(proxy_json("alpha", "https://alpha-never.example.com/api/"))
        .send(harness.router())
        .await;

    // Not REVISION_MISMATCH: the client's revision was fine every time.
    // Telling them to re-read would be wrong advice.
    assert_eq!(reply.status, 409, "{}", reply.body);
    assert_eq!(reply.error_code(), "CONFLICT");
}

#[tokio::test]
async fn a_write_without_a_token_is_unauthorized() {
    let harness = Harness::new();
    let reply = Call::post("/api/v1/proxies")
        .json(proxy_json("gamma", "https://gamma.example.com/api/"))
        .send(harness.router())
        .await;

    assert_eq!(reply.status, 401);
    assert_eq!(reply.error_code(), "UNAUTHORIZED");
    assert_eq!(harness.stored().proxies.len(), 2);
}

#[tokio::test]
async fn a_token_without_the_right_is_forbidden() {
    let harness = Harness::new();
    let reply = Call::post("/api/v1/proxies")
        .token("reader-token-00000000000000000000000")
        .json(proxy_json("gamma", "https://gamma.example.com/api/"))
        .send(harness.router())
        .await;

    assert_eq!(reply.status, 403);
    assert_eq!(reply.error_code(), "FORBIDDEN");
    assert_eq!(harness.stored().proxies.len(), 2);
}

#[tokio::test]
async fn authorization_is_decided_before_existence() {
    // A caller who may not delete gets the same answer for a proxy that
    // exists and one that does not. Otherwise the pair of statuses is a
    // membership oracle over the proxy names.
    let harness = Harness::new();
    let existing = Call::delete("/api/v1/proxies/alpha")
        .token("reader-token-00000000000000000000000")
        .send(harness.router())
        .await;
    let missing = Call::delete("/api/v1/proxies/nope")
        .token("reader-token-00000000000000000000000")
        .send(harness.router())
        .await;

    assert_eq!(existing.status, 403);
    assert_eq!(missing.status, 403);
    assert_eq!(existing.error_code(), missing.error_code());
}

#[tokio::test]
async fn an_omitted_access_block_does_not_publish_upstream_credentials() {
    // The defect this pins: `list` and `read` once defaulted to public, and a
    // proxy document carries the headers that proxy injects upstream. An
    // anonymous GET therefore returned the operator's upstream token.
    let yaml = common::BASE_CONFIG
        .replace(
            "  access:\n    list: public\n    read: public\n",
            "  access:\n",
        )
        .replace(
            "    url: \"https://alpha.example.com/api/\"\n",
            "    url: \"https://alpha.example.com/api/\"\n    headers:\n      Authorization: \"Bearer upstream-secret\"\n",
        );
    assert!(
        yaml.contains("upstream-secret") && !yaml.contains("list: public"),
        "the fixture must actually drop the access block and inject a secret"
    );
    let harness = Harness::with_config(&yaml);

    for uri in ["/api/v1/proxies", "/api/v1/proxies/alpha"] {
        let reply = Call::get(uri).send(harness.router()).await;
        assert_eq!(reply.status, 401, "{uri}: {}", reply.body);
        assert!(
            !reply.body.contains("upstream-secret"),
            "{uri} leaked the injected credential: {}",
            reply.body
        );
    }
}

#[tokio::test]
async fn an_explicit_public_read_is_still_served_to_anyone() {
    // The default is safe; the choice remains the operator's.
    let harness = Harness::new();
    assert_eq!(
        Call::get("/api/v1/proxies/alpha")
            .send(harness.router())
            .await
            .status,
        200
    );
}
