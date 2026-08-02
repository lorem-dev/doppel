//! Reloading the running configuration, and reporting it over `/status`.

mod common;

use common::{BASE_CONFIG, Call, Harness, proxy_json};

const ROOT: &str = "root-token";

#[tokio::test]
async fn status_reports_the_running_revision_and_every_proxy() {
    let harness = Harness::new();
    let reply = Call::get("/status").send(harness.router()).await;

    assert_eq!(reply.status, 200, "{}", reply.body);
    let body = reply.json();
    assert_eq!(body["revision"], harness.holder.load().revision.to_string());
    let proxies = body["proxies"].as_array().expect("proxies is an array");
    assert_eq!(proxies.len(), 2);
    assert_eq!(proxies[0]["name"], "alpha");
    assert_eq!(proxies[0]["upstream"], "https://alpha.example.com/api/");
    assert_eq!(proxies[0]["resolve"], "header:X-Proxy-Name");
    assert_eq!(proxies[0]["mocks"], 0);
    assert!(body["uptime_seconds"].is_u64());
}

#[tokio::test]
async fn status_needs_no_token() {
    // It is what a load balancer calls, so it cannot require credentials.
    let harness = Harness::new();
    assert_eq!(
        Call::get("/status").send(harness.router()).await.status,
        200
    );
}

#[tokio::test]
async fn status_never_prints_upstream_credentials() {
    // `/status` is public and a proxy URL may legitimately carry basic auth,
    // so this endpoint is the one place those two facts could combine into a
    // published password.
    let harness = Harness::with_config(&BASE_CONFIG.replace(
        "https://alpha.example.com/api/",
        "https://svc:hunter2@alpha.example.com/api/",
    ));
    let reply = Call::get("/status").send(harness.router()).await;

    assert_eq!(reply.status, 200, "{}", reply.body);
    assert!(!reply.body.contains("hunter2"), "{}", reply.body);
    assert!(!reply.body.contains("svc"), "{}", reply.body);
    assert!(reply.body.contains("alpha.example.com"), "{}", reply.body);
}

#[tokio::test]
async fn status_reports_a_default_resolver_as_default() {
    let harness = Harness::with_config(&BASE_CONFIG.replace(
        "    resolve:\n      type: header\n      header: X-Proxy-Name\n  - name: beta",
        "  - name: beta",
    ));
    let reply = Call::get("/status").send(harness.router()).await;

    assert_eq!(reply.json()["proxies"][0]["resolve"], "default");
}

#[tokio::test]
async fn status_reports_what_is_running_not_what_is_stored() {
    // A configuration written but not reloaded is not what this process is
    // doing. Reporting the store here would make `/status` agree with the
    // file and disagree with reality, which is the opposite of its job.
    let harness = Harness::new();
    let before = Call::get("/status").send(harness.router()).await.json();

    let create = Call::post("/api/v1/proxies")
        .token(ROOT)
        .json(proxy_json("gamma", "https://gamma.example.com/api/"))
        .send(harness.router())
        .await;
    assert_eq!(create.status, 201, "{}", create.body);

    let after = Call::get("/status").send(harness.router()).await.json();
    assert_eq!(before, after);
    assert_eq!(after["proxies"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn reload_applies_a_change_written_through_the_api() {
    let harness = Harness::new();
    let create = Call::post("/api/v1/proxies")
        .token(ROOT)
        .json(proxy_json("gamma", "https://gamma.example.com/api/"))
        .send(harness.router())
        .await;
    assert_eq!(create.status, 201, "{}", create.body);

    let reload = Call::post("/api/v1/config/reload")
        .token(ROOT)
        .send(harness.router())
        .await;
    assert_eq!(reload.status, 200, "{}", reload.body);
    assert_eq!(reload.json()["proxies"], 3);

    let status = Call::get("/status").send(harness.router()).await.json();
    assert_eq!(status["proxies"].as_array().unwrap().len(), 3);
    assert_eq!(status["revision"], reload.json()["revision"]);
}

#[tokio::test]
async fn reload_reports_the_same_revision_when_nothing_changed() {
    // The revision comes from the stored content, so a reload that changes
    // nothing must not look like a change.
    let harness = Harness::new();
    let before = Call::get("/status").send(harness.router()).await.json()["revision"].clone();

    let reload = Call::post("/api/v1/config/reload")
        .token(ROOT)
        .send(harness.router())
        .await;
    assert_eq!(reload.status, 200, "{}", reload.body);
    assert_eq!(reload.json()["revision"], before);
}

#[tokio::test]
async fn an_invalid_stored_config_is_rejected_and_the_running_one_survives() {
    let harness = Harness::new();
    let before = Call::get("/status").send(harness.router()).await.json();

    harness.overwrite_config(&BASE_CONFIG.replace("port: 18080", "port: 0"));
    let reload = Call::post("/api/v1/config/reload")
        .token(ROOT)
        .send(harness.router())
        .await;

    assert_eq!(reload.status, 400, "{}", reload.body);
    assert_eq!(reload.error_code(), "CONFIG_INVALID");
    // Every step before the swap can fail; the swap cannot. So a rejected
    // reload leaves the process serving exactly what it was serving.
    let after = Call::get("/status").send(harness.router()).await.json();
    assert_eq!(before["revision"], after["revision"]);
}

#[tokio::test]
async fn an_unparsable_stored_config_is_rejected_and_the_running_one_survives() {
    let harness = Harness::new();
    let before = Call::get("/status").send(harness.router()).await.json();

    harness.overwrite_config("this: is: not: valid: yaml:\n  - [");
    let reload = Call::post("/api/v1/config/reload")
        .token(ROOT)
        .send(harness.router())
        .await;

    assert_eq!(reload.status, 400, "{}", reload.body);
    assert_eq!(reload.error_code(), "CONFIG_INVALID");
    let after = Call::get("/status").send(harness.router()).await.json();
    assert_eq!(before["revision"], after["revision"]);
}

#[tokio::test]
async fn reload_names_sections_that_only_take_effect_on_restart() {
    let harness = Harness::new();
    harness.overwrite_config(&BASE_CONFIG.replace("port: 18080", "port: 18099"));

    let reload = Call::post("/api/v1/config/reload")
        .token(ROOT)
        .send(harness.router())
        .await;

    assert_eq!(reload.status, 200, "{}", reload.body);
    // Accepted and counted into the new revision, but the listener is
    // already bound. Silence here would let an operator believe the port
    // moved.
    assert_eq!(reload.json()["unapplied"], serde_json::json!(["server"]));
}

#[tokio::test]
async fn an_ordinary_reload_reports_no_unapplied_sections_at_all() {
    // Absent rather than an empty list: the common answer stays quiet.
    let harness = Harness::new();
    let reload = Call::post("/api/v1/config/reload")
        .token(ROOT)
        .send(harness.router())
        .await;

    assert_eq!(reload.status, 200, "{}", reload.body);
    assert!(reload.json().get("unapplied").is_none(), "{}", reload.body);
}

#[tokio::test]
async fn reload_without_a_token_is_unauthorized() {
    let harness = Harness::new();
    let reply = Call::post("/api/v1/config/reload")
        .send(harness.router())
        .await;

    assert_eq!(reply.status, 401, "{}", reply.body);
    assert_eq!(reply.error_code(), "UNAUTHORIZED");
}

#[tokio::test]
async fn a_token_without_write_rights_may_not_reload() {
    let harness = Harness::new();
    let reply = Call::post("/api/v1/config/reload")
        .token("reader-token")
        .send(harness.router())
        .await;

    assert_eq!(reply.status, 403, "{}", reply.body);
    assert_eq!(reply.error_code(), "FORBIDDEN");
}

#[tokio::test]
async fn a_stored_config_cannot_authorise_its_own_promotion() {
    // The escalation this guards against: someone who can write the
    // configuration file out of band, but holds no valid token, adds a token
    // for themselves and then reloads it into effect. Authorizing against
    // the running configuration -- the policy the operator actually put in
    // force -- is what closes it.
    //
    // Note the attack has to be a new *token* rather than `update: public`,
    // which rule V34 already refuses. A token grant is perfectly valid
    // configuration, so nothing else in the system would stop it.
    let harness = Harness::new();
    let tampered = BASE_CONFIG.replace(
        "    - name: reader",
        "    - name: intruder\n      group: admin\n      token: intruder-token\n    - name: reader",
    );
    assert!(
        tampered.contains("intruder-token"),
        "the tampered document must actually differ, or this test proves nothing"
    );
    harness.overwrite_config(&tampered);

    let reply = Call::post("/api/v1/config/reload")
        .token("intruder-token")
        .send(harness.router())
        .await;

    // Unknown to the running configuration, so anonymous, so 401.
    assert_eq!(reply.status, 401, "{}", reply.body);
    assert_eq!(reply.error_code(), "UNAUTHORIZED");

    // And it never took effect, so a second attempt fails the same way
    // rather than succeeding on the strength of the first.
    let again = Call::post("/api/v1/config/reload")
        .token("intruder-token")
        .send(harness.router())
        .await;
    assert_eq!(again.status, 401);

    // A legitimate operator can still reload, which is what makes the
    // refusal above about the token rather than about reload being broken.
    let genuine = Call::post("/api/v1/config/reload")
        .token(ROOT)
        .send(harness.router())
        .await;
    assert_eq!(genuine.status, 200, "{}", genuine.body);
}
