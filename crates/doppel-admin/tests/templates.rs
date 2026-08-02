//! Template upload, listing and deletion over the admin API.

mod common;

use common::{Call, Harness, TEMPLATE_CONFIG, assert_absent};
use serde_json::json;

const ROOT: &str = "root-token";

fn harness() -> Harness {
    Harness::with_config(TEMPLATE_CONFIG)
}

#[tokio::test]
async fn upload_list_and_delete_round_trip() {
    let harness = harness();

    let upload = Call::post("/api/v1/proxies/alpha/templates/one.json.j2")
        .token(ROOT)
        .raw("{\"ok\": true}")
        .send(harness.router())
        .await;
    assert_eq!(upload.status, 204, "{}", upload.body);
    assert!(harness.template_path("alpha", "one.json.j2").exists());

    let list = Call::get("/api/v1/proxies/alpha/templates")
        .send(harness.router())
        .await;
    assert_eq!(list.status, 200, "{}", list.body);
    assert_eq!(
        list.json()["templates"],
        json!([{ "name": "one.json.j2", "size": 12 }])
    );

    let delete = Call::delete("/api/v1/proxies/alpha/templates/one.json.j2")
        .token(ROOT)
        .send(harness.router())
        .await;
    assert_eq!(delete.status, 204, "{}", delete.body);
    assert_absent(&harness.template_path("alpha", "one.json.j2"));
}

#[tokio::test]
async fn uploading_the_same_file_twice_replaces_it() {
    let harness = harness();
    for body in ["first", "second"] {
        let reply = Call::post("/api/v1/proxies/alpha/templates/one.json.j2")
            .token(ROOT)
            .raw(body)
            .send(harness.router())
            .await;
        assert_eq!(reply.status, 204, "{}", reply.body);
    }

    let stored = std::fs::read_to_string(harness.template_path("alpha", "one.json.j2")).unwrap();
    assert_eq!(stored, "second");
}

#[tokio::test]
async fn a_file_no_mock_declares_is_rejected() {
    let harness = harness();
    let reply = Call::post("/api/v1/proxies/alpha/templates/stray.json.j2")
        .token(ROOT)
        .raw("{}")
        .send(harness.router())
        .await;

    // An upload nothing will ever read is a mistake worth reporting rather
    // than a file worth keeping.
    assert_eq!(reply.status, 422, "{}", reply.body);
    assert_eq!(reply.error_code(), "TEMPLATE_NOT_DECLARED");
    assert_absent(&harness.template_path("alpha", "stray.json.j2"));
}

#[tokio::test]
async fn a_file_declared_by_a_different_proxy_is_rejected() {
    // `beta` has no mocks at all, so `one.json.j2` is declared in the
    // configuration but not by this proxy. Checking the whole document
    // rather than the named proxy would let anyone with upload rights write
    // into any proxy's directory.
    let harness = harness();
    let reply = Call::post("/api/v1/proxies/beta/templates/one.json.j2")
        .token(ROOT)
        .raw("{}")
        .send(harness.router())
        .await;

    assert_eq!(reply.status, 422, "{}", reply.body);
    assert_eq!(reply.error_code(), "TEMPLATE_NOT_DECLARED");
}

#[tokio::test]
async fn an_oversized_body_is_rejected() {
    let harness = harness();
    let reply = Call::post("/api/v1/proxies/alpha/templates/one.json.j2")
        .token(ROOT)
        .raw("x".repeat(65))
        .send(harness.router())
        .await;

    assert_eq!(reply.status, 413, "{}", reply.body);
    assert_eq!(reply.error_code(), "UPLOAD_TOO_LARGE");
    assert_absent(&harness.template_path("alpha", "one.json.j2"));
}

#[tokio::test]
async fn a_body_of_exactly_the_limit_is_accepted() {
    // The limit is a maximum, not a strict bound. An off-by-one here would
    // reject the very size an operator configured.
    let harness = harness();
    let reply = Call::post("/api/v1/proxies/alpha/templates/one.json.j2")
        .token(ROOT)
        .raw("x".repeat(64))
        .send(harness.router())
        .await;

    assert_eq!(reply.status, 204, "{}", reply.body);
    assert_eq!(
        std::fs::metadata(harness.template_path("alpha", "one.json.j2"))
            .unwrap()
            .len(),
        64
    );
}

#[tokio::test]
async fn an_undeclared_name_is_reported_before_the_body_is_read() {
    // The order of the three checks is load bearing: a name nothing declares
    // is refused without buffering the body, so an oversized upload to a
    // bogus name costs nothing. Reporting 413 here would mean the server had
    // already read it.
    let harness = harness();
    let reply = Call::post("/api/v1/proxies/alpha/templates/stray.json.j2")
        .token(ROOT)
        .raw("x".repeat(4096))
        .send(harness.router())
        .await;

    assert_eq!(reply.status, 422, "{}", reply.body);
    assert_eq!(reply.error_code(), "TEMPLATE_NOT_DECLARED");
}

#[tokio::test]
async fn a_traversing_name_is_rejected() {
    let harness = harness();
    for name in ["%2e%2e", "..%2fescape.j2", "%2fetc%2fpasswd"] {
        let reply = Call::post(format!("/api/v1/proxies/alpha/templates/{name}"))
            .token(ROOT)
            .raw("{}")
            .send(harness.router())
            .await;

        assert_eq!(reply.status, 400, "{name}: {}", reply.body);
        assert_eq!(reply.error_code(), "CONFIG_INVALID", "{name}");
    }
    // Nothing landed anywhere near the template root.
    assert_absent(&harness.templates_dir.join("escape.j2"));
}

#[tokio::test]
async fn listing_templates_of_a_proxy_with_none_is_empty_not_an_error() {
    let harness = harness();
    let reply = Call::get("/api/v1/proxies/beta/templates")
        .send(harness.router())
        .await;

    assert_eq!(reply.status, 200, "{}", reply.body);
    assert_eq!(reply.json()["templates"], json!([]));
}

#[tokio::test]
async fn listing_templates_of_a_missing_proxy_is_not_found() {
    // Distinct from the case above: "this proxy has no templates" and "there
    // is no such proxy" are different answers and a client acts on them
    // differently.
    let harness = harness();
    let reply = Call::get("/api/v1/proxies/nope/templates")
        .send(harness.router())
        .await;

    assert_eq!(reply.status, 404, "{}", reply.body);
    assert_eq!(reply.error_code(), "NOT_FOUND");
}

#[tokio::test]
async fn uploading_to_a_missing_proxy_is_not_found() {
    let harness = harness();
    let reply = Call::post("/api/v1/proxies/nope/templates/one.json.j2")
        .token(ROOT)
        .raw("{}")
        .send(harness.router())
        .await;

    assert_eq!(reply.status, 404, "{}", reply.body);
    assert_eq!(reply.error_code(), "NOT_FOUND");
}

#[tokio::test]
async fn deleting_a_file_that_is_not_there_is_not_found() {
    let harness = harness();
    let reply = Call::delete("/api/v1/proxies/alpha/templates/one.json.j2")
        .token(ROOT)
        .send(harness.router())
        .await;

    assert_eq!(reply.status, 404, "{}", reply.body);
    assert_eq!(reply.error_code(), "NOT_FOUND");
}

#[tokio::test]
async fn updating_a_proxy_to_drop_a_mock_removes_that_mocks_template() {
    let harness = harness();
    for file in ["one.json.j2", "two.json.j2"] {
        let reply = Call::post(format!("/api/v1/proxies/alpha/templates/{file}"))
            .token(ROOT)
            .raw("{}")
            .send(harness.router())
            .await;
        assert_eq!(reply.status, 204, "{}", reply.body);
    }

    let read = Call::get("/api/v1/proxies/alpha")
        .send(harness.router())
        .await;
    let mut proxy = read.json()["proxy"].clone();
    // Drop the mock that declares `two.json.j2`.
    let mocks = proxy["mocks"].as_array().unwrap().clone();
    proxy["mocks"] = json!([mocks[0].clone()]);

    let update = Call::put("/api/v1/proxies/alpha")
        .token(ROOT)
        .if_match(format!("\"{}\"", read.revision()))
        .json(json!({ "proxy": proxy }))
        .send(harness.router())
        .await;
    assert_eq!(update.status, 200, "{}", update.body);

    // The file the surviving mock names stays; the other goes, because
    // nothing can render it any more.
    assert!(harness.template_path("alpha", "one.json.j2").exists());
    assert_absent(&harness.template_path("alpha", "two.json.j2"));
}

#[tokio::test]
async fn an_update_that_keeps_every_mock_keeps_every_template() {
    let harness = harness();
    for file in ["one.json.j2", "two.json.j2"] {
        Call::post(format!("/api/v1/proxies/alpha/templates/{file}"))
            .token(ROOT)
            .raw("{}")
            .send(harness.router())
            .await;
    }

    let read = Call::get("/api/v1/proxies/alpha")
        .send(harness.router())
        .await;
    let mut proxy = read.json()["proxy"].clone();
    proxy["url"] = json!("https://alpha-moved.example.com/api/");

    let update = Call::put("/api/v1/proxies/alpha")
        .token(ROOT)
        .if_match(format!("\"{}\"", read.revision()))
        .json(json!({ "proxy": proxy }))
        .send(harness.router())
        .await;
    assert_eq!(update.status, 200, "{}", update.body);

    for file in ["one.json.j2", "two.json.j2"] {
        assert!(
            harness.template_path("alpha", file).exists(),
            "{file} should have survived an unrelated update"
        );
    }
}

#[tokio::test]
async fn a_refused_update_does_not_touch_the_templates() {
    // The template sweep is authorised by the configuration write. If the
    // write is rejected the old mocks are still in force, so their files must
    // still be there.
    let harness = harness();
    Call::post("/api/v1/proxies/alpha/templates/two.json.j2")
        .token(ROOT)
        .raw("{}")
        .send(harness.router())
        .await;

    let read = Call::get("/api/v1/proxies/alpha")
        .send(harness.router())
        .await;
    let mut proxy = read.json()["proxy"].clone();
    proxy["mocks"] = json!([]);

    let update = Call::put("/api/v1/proxies/alpha")
        .token(ROOT)
        .if_match("\"0123456789abcdef\"")
        .json(json!({ "proxy": proxy }))
        .send(harness.router())
        .await;
    assert_eq!(update.status, 409, "{}", update.body);
    assert!(harness.template_path("alpha", "two.json.j2").exists());
}

#[tokio::test]
async fn an_upload_without_a_token_is_unauthorized() {
    let harness = harness();
    let reply = Call::post("/api/v1/proxies/alpha/templates/one.json.j2")
        .raw("{}")
        .send(harness.router())
        .await;

    assert_eq!(reply.status, 401);
    assert_eq!(reply.error_code(), "UNAUTHORIZED");
}

#[tokio::test]
async fn upload_authorization_is_decided_before_existence() {
    let harness = harness();
    let existing = Call::post("/api/v1/proxies/alpha/templates/one.json.j2")
        .token("reader-token")
        .raw("{}")
        .send(harness.router())
        .await;
    let missing = Call::post("/api/v1/proxies/nope/templates/one.json.j2")
        .token("reader-token")
        .raw("{}")
        .send(harness.router())
        .await;

    assert_eq!(existing.status, 403);
    assert_eq!(missing.status, 403);
}

#[tokio::test]
async fn listing_templates_needs_read_not_upload() {
    // Listing a proxy's files is a read of that proxy, so `read` governs it.
    // Requiring `upload` would mean an operator who may look at a proxy could
    // not see which of its templates are present.
    let harness = harness();
    let reply = Call::get("/api/v1/proxies/alpha/templates")
        .send(harness.router())
        .await;

    assert_eq!(reply.status, 200, "{}", reply.body);
}
