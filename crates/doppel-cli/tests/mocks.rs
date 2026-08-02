//! End-to-end coverage of every mock in `main.example.yaml`.
//!
//! These run the built binary against the reference configuration itself
//! rather than a copy of it, so the file that documents the schema is also the
//! file under test. Only the values that cannot survive a test environment are
//! rewritten: the two ports, the control socket, the templates directory, and
//! the upstream URLs.
//!
//! One further rewrite is deliberate and called out where it happens:
//! `mock3` ships with `replace: 0.5`, which is there to demonstrate the
//! per-mock override. A probabilistic mock cannot be asserted on, so the test
//! copy raises it to 1.0. What is pinned is the mock's response, not its dice.

mod common;

use common::{Server, upstream};

/// Rewrite the reference configuration for a test environment.
fn reference_config(
    server_port: u16,
    upstream_port: u16,
    socket: &std::path::Path,
    templates: &std::path::Path,
) -> String {
    let raw = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../main.example.yaml"
    ))
    .expect("the reference configuration must be readable");

    let upstream = format!("http://127.0.0.1:{upstream_port}/");
    let replaced = raw
        .replace("port: 8080", &format!("port: {server_port}"))
        .replace("port: 8081", &format!("port: {}", common::free_port()))
        .replace(
            "socket: /tmp/doppel.sock",
            &format!("socket: {}", socket.display()),
        )
        .replace("dir: ./templates", &format!("dir: {}", templates.display()))
        .replace("https://external-service.com/api/v1/", &upstream)
        .replace("https://other-service.com/", &upstream)
        // See the module comment: the shipped 0.5 demonstrates the override,
        // but a coin flip cannot be asserted on.
        .replace("replace: 0.5", "replace: 1.0")
        // Same reasoning for the fault demonstrations. The reference config
        // drops 10 percent of requests and delays 45 percent of them, which is
        // exactly what a fault-injection proxy should show off -- and exactly
        // what makes any assertion against it a coin flip. Both are covered
        // deterministically by the unit tests, with a seeded sampler.
        .replace("percentage: 0.1", "percentage: 0.0")
        .replace("percentage: 0.45", "percentage: 0.0")
        .replace("percentage: 0.5", "percentage: 0.0");

    assert!(
        !replaced.contains("external-service.com"),
        "the upstream URL rewrite missed; the test would reach the real internet"
    );
    replaced
}

fn server() -> (common::Upstream, Server) {
    let up = upstream();
    let server = Server::start_with(up.port, reference_config);
    (up, server)
}

#[test]
fn mock1_serves_a_literal_body() {
    let (_up, server) = server();
    let (status, body) = server.get("/api/v1/resource/");
    assert_eq!(status, 200);
    assert_eq!(body, r#"{"message": "Success"}"#);
}

#[test]
fn mock2_renders_from_the_request_body_and_query() {
    let (_up, server) = server();
    let (status, body) = server.request(
        "POST",
        "/api/v1/resource/?filter=open&sort=-id",
        r#"{"name": "ada", "description": "first", "content": {"items": [1, 2, 3]}}"#,
    );
    assert_eq!(status, 201);
    let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(parsed["name"], "ada");
    assert_eq!(parsed["description"], "first");
    assert_eq!(
        parsed["items"], 3,
        "`resourceItems | length` must count the array"
    );
}

#[test]
fn the_literal_42_path_is_mocked_rather_than_proxied() {
    let (_up, server) = server();
    let (status, body) = server.get("/api/v1/resource/42/");
    assert_eq!(status, 200);
    let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(parsed["message"], "Success");
    // Deliberately not named after `mock3`: its response is byte-identical to
    // `mock1`'s, so no assertion on the reply can tell which of the two served
    // it. What this pins is that the path is mocked at all rather than reaching
    // the upstream -- which the upstream's own echo body would reveal. Which
    // mock served a request is attributable only from the log line's `mock`
    // field, covered by the pipeline unit tests.
    assert!(
        !body.contains("upstream saw"),
        "the path must be mocked, not proxied, got {body:?}"
    );
}

#[test]
fn mock4_binds_a_path_capture_into_the_body_and_headers() {
    let (_up, server) = server();
    // `mock4` extracts `requestId` from `X-Request-ID` and renders it into a
    // response header. Rendering is strict, so that request header is not
    // optional -- the reference config says as much where the mock is defined.
    let url = format!("http://127.0.0.1:{}/api/v1/resource/7/", server.port());
    let response = reqwest::blocking::Client::new()
        .get(url)
        .header("x-request-id", "abc-123")
        .send()
        .unwrap();
    assert_eq!(response.status().as_u16(), 200);
    assert_eq!(
        response.headers().get("x-resource-id").unwrap(),
        "7",
        "the capture must render into the response header too"
    );
    let parsed: serde_json::Value = serde_json::from_str(&response.text().unwrap()).unwrap();
    assert_eq!(
        parsed["id"], "7",
        "the resourceId capture must reach the body"
    );
}

#[test]
fn mock4_without_the_header_it_extracts_fails_loudly() {
    let (_up, server) = server();
    // The other half of the sharp edge above: strict rendering turns a missing
    // extracted header into a 500 rather than an empty string, which is the
    // whole point of strict mode -- a silently blank field in a mocked
    // response is worse than a refusal.
    let (status, body) = server.get("/api/v1/resource/7/");
    assert_eq!(status, 500);
    let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(parsed["code"], "TEMPLATE_RENDER_ERROR");
    assert!(
        parsed["message"].as_str().unwrap().contains("requestId"),
        "the message must name the variable, got {}",
        parsed["message"]
    );
}

#[test]
fn mock5_serves_a_bodiless_204() {
    let (_up, server) = server();
    let (status, body) = server.request("DELETE", "/api/v1/resource/9/", "");
    assert_eq!(status, 204);
    assert!(body.is_empty(), "204 forbids a body, got {body:?}");
}

#[test]
fn mock6_renders_a_template_file() {
    let (_up, server) = server();
    server.write_template(
        "proxy1",
        "put.json.j2",
        r#"{"updated": "{{ resourceId }}", "name": "{{ resourceName }}"}"#,
    );
    let (status, body) = server.request(
        "PUT",
        "/api/v1/resource/11/",
        r#"{"name": "ada", "description": "d", "content": {"items": []}}"#,
    );
    assert_eq!(status, 200);
    let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(parsed["updated"], "11");
    assert_eq!(parsed["name"], "ada");
}

#[test]
fn a_template_that_was_never_placed_yields_template_not_found() {
    let (_up, server) = server();
    // No `write_template` call: `mock6` names a file that is not on disk,
    // which is a legal intermediate state because a later phase uploads
    // templates at runtime.
    let (status, body) = server.request(
        "PUT",
        "/api/v1/resource/11/",
        r#"{"name": "ada", "description": "d", "content": {"items": []}}"#,
    );
    assert_eq!(status, 500);
    let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(parsed["status"], "error");
    assert_eq!(parsed["code"], "TEMPLATE_NOT_FOUND");
}

#[test]
fn an_unmatched_path_still_reaches_the_upstream() {
    let (_up, server) = server();
    let (status, body) = server.get("/not-a-mock");
    assert_eq!(status, 200);
    assert_eq!(
        body, "upstream saw /not-a-mock",
        "a path no mock matches must be proxied, not mocked"
    );
}
