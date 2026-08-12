//! `GET /api/v1/schema`: the configuration schema, from the running process.

mod common;

use common::{Call, Harness};

#[tokio::test]
async fn the_schema_is_served_to_anybody_who_asks() {
    // No token. It describes the shape of a configuration, never the contents of
    // one, and the identical bytes are on GitHub -- so a right to hold here would
    // guard nothing while stopping the page from checking a field before sign-in.
    let harness = Harness::new();
    let reply = Call::get("/api/v1/schema").send(harness.router()).await;

    assert_eq!(reply.status, 200);
    assert_eq!(
        reply.content_type.as_deref(),
        Some("application/schema+json")
    );

    let schema: serde_json::Value = serde_json::from_str(&reply.body).expect("the body is JSON");
    assert_eq!(
        schema["$schema"], "https://json-schema.org/draft/2020-12/schema",
        "not a 2020-12 document"
    );
    // The three the dashboard reaches for: a pattern to check a name against, a
    // range to bound a ratio, and the proxy document it validates in the YAML
    // editor.
    assert_eq!(
        schema["$defs"]["ProxyName"]["pattern"],
        "^[A-Za-z0-9_-]{2,32}$"
    );
    assert_eq!(schema["$defs"]["Ratio"]["maximum"], 1.0);
    assert!(schema["$defs"]["ProxyConfig"]["properties"]["mocks"].is_object());
}

#[tokio::test]
async fn the_schema_is_the_document_the_repository_keeps() {
    // The file is what editors fetch and what a release attaches; this endpoint is
    // the same document from the process that will actually read the configuration.
    // Two answers to one question is the failure worth catching.
    let harness = Harness::new();
    let reply = Call::get("/api/v1/schema").send(harness.router()).await;

    let served: serde_json::Value = serde_json::from_str(&reply.body).expect("the body is JSON");
    let on_disk: serde_json::Value =
        serde_json::from_str(include_str!("../../../doppel-config.schema.json"))
            .expect("the checked-in schema is JSON");
    assert_eq!(served, on_disk);
}

#[tokio::test]
async fn a_disabled_dashboard_still_serves_the_schema() {
    // It is an API endpoint, not part of the page: a deployment with
    // `dashboard: false` still has clients that validate a document before pushing
    // it.
    let config = common::BASE_CONFIG.replacen("  tokens:", "  dashboard: false\n  tokens:", 1);
    let harness = Harness::with_config(&config);
    let reply = Call::get("/api/v1/schema").send(harness.router()).await;

    assert_eq!(reply.status, 200);
}
