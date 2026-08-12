//! Reloading the running configuration, and reporting it over `/api/v1/status`.

mod common;

use std::time::Duration;

use common::{BASE_CONFIG, Call, Harness, proxy_json};

const ROOT: &str = "root-token-0000000000000000000000000";

#[tokio::test]
async fn status_reports_the_running_revision_and_every_proxy() {
    let harness = Harness::new();
    let reply = Call::get("/api/v1/status").send(harness.router()).await;

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
        Call::get("/api/v1/status")
            .send(harness.router())
            .await
            .status,
        200
    );
}

#[tokio::test]
async fn status_never_prints_upstream_credentials() {
    // `/api/v1/status` is public and a proxy URL may legitimately carry basic auth,
    // so this endpoint is the one place those two facts could combine into a
    // published password.
    let harness = Harness::with_config(&BASE_CONFIG.replace(
        "https://alpha.example.com/api/",
        "https://svc:hunter2@alpha.example.com/api/",
    ));
    let reply = Call::get("/api/v1/status").send(harness.router()).await;

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
    let reply = Call::get("/api/v1/status").send(harness.router()).await;

    assert_eq!(reply.json()["proxies"][0]["resolve"], "default");
}

#[tokio::test]
async fn status_reports_what_is_running_not_what_is_stored() {
    // A configuration written *behind* the API -- an operator editing
    // `main.yaml`, `doppel config push`, another instance -- is not what this
    // process is doing until it reloads. Reporting the store here would make
    // `/api/v1/status` agree with the file and disagree with reality, which is
    // the opposite of its job.
    //
    // A write *through* the API is the other case, and the test below: that one
    // is in force by the time the response is written.
    let harness = Harness::new();
    let before = Call::get("/api/v1/status")
        .send(harness.router())
        .await
        .json();

    let (mut config, revision) = harness.store.load().await.expect("the fixture loads");
    // Through the same shape the API takes, minus the API: `proxy_json`'s
    // document, deserialized straight into the configuration.
    config.proxies.push(
        serde_json::from_value(
            proxy_json("gamma", "https://gamma.example.com/api/")["proxy"].clone(),
        )
        .expect("a proxy document parses"),
    );
    harness
        .store
        .save(&config, Some(revision))
        .await
        .expect("the store accepts it");

    let after = Call::get("/api/v1/status")
        .send(harness.router())
        .await
        .json();
    assert_eq!(before, after);
    assert_eq!(after["proxies"].as_array().unwrap().len(), 2);
}

/// The other half, and the one an operator reported: a `PUT` that changed an
/// upstream returned 200, the document held the new value, and the traffic kept
/// going to the old one until somebody reloaded.
///
/// Every write applies to the running process now, so a 200 means what a client
/// reads it to mean.
#[tokio::test]
async fn a_write_through_the_api_is_in_force_immediately() {
    let harness = Harness::new();
    let before = Call::get("/api/v1/status")
        .send(harness.router())
        .await
        .json();

    let create = Call::post("/api/v1/proxies")
        .token(ROOT)
        .json(proxy_json("gamma", "https://gamma.example.com/api/"))
        .send(harness.router())
        .await;
    assert_eq!(create.status, 201, "{}", create.body);

    let after = Call::get("/api/v1/status")
        .send(harness.router())
        .await
        .json();
    assert_eq!(after["proxies"].as_array().unwrap().len(), 3);
    assert_ne!(after["revision"], before["revision"]);

    // And an update reaches it as well, which is the shape of the report: the
    // upstream in the running configuration is the one just written.
    let update = Call::put("/api/v1/proxies/gamma")
        .token(ROOT)
        // The revision the create handed back, which is what an update is
        // required to carry.
        .if_match(create.etag.clone().expect("create sets an ETag"))
        .json(proxy_json("gamma", "https://moved.example.com/api/"))
        .send(harness.router())
        .await;
    assert_eq!(update.status, 200, "{}", update.body);

    let running = Call::get("/api/v1/status")
        .send(harness.router())
        .await
        .json();
    let gamma = running["proxies"]
        .as_array()
        .unwrap()
        .iter()
        .find(|proxy| proxy["name"] == "gamma")
        .expect("gamma is in the running configuration");
    assert_eq!(gamma["upstream"], "https://moved.example.com/api/");

    // A delete too: a proxy the operator has been told is gone must not still be
    // served.
    let delete = Call::delete("/api/v1/proxies/gamma")
        .token(ROOT)
        .send(harness.router())
        .await;
    assert_eq!(delete.status, 204, "{}", delete.body);
    let gone = Call::get("/api/v1/status")
        .send(harness.router())
        .await
        .json();
    assert_eq!(gone["proxies"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn a_reload_after_a_write_through_the_api_changes_nothing_more() {
    let harness = Harness::new();
    let create = Call::post("/api/v1/proxies")
        .token(ROOT)
        .json(proxy_json("gamma", "https://gamma.example.com/api/"))
        .send(harness.router())
        .await;
    assert_eq!(create.status, 201, "{}", create.body);

    // The write already applied, so this is the idempotent case: same store,
    // same revision, nothing to promote.
    let reload = Call::post("/api/v1/config/reload")
        .token(ROOT)
        .send(harness.router())
        .await;
    assert_eq!(reload.status, 200, "{}", reload.body);
    assert_eq!(reload.json()["proxies"], 3);

    let status = Call::get("/api/v1/status")
        .send(harness.router())
        .await
        .json();
    assert_eq!(status["proxies"].as_array().unwrap().len(), 3);
    assert_eq!(status["revision"], reload.json()["revision"]);
}

#[tokio::test]
async fn reload_reports_the_same_revision_when_nothing_changed() {
    // The revision comes from the stored content, so a reload that changes
    // nothing must not look like a change.
    let harness = Harness::new();
    let before = Call::get("/api/v1/status")
        .send(harness.router())
        .await
        .json()["revision"]
        .clone();

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
    let before = Call::get("/api/v1/status")
        .send(harness.router())
        .await
        .json();

    harness.overwrite_config(&BASE_CONFIG.replace("port: 18080", "port: 0"));
    let reload = Call::post("/api/v1/config/reload")
        .token(ROOT)
        .send(harness.router())
        .await;

    assert_eq!(reload.status, 400, "{}", reload.body);
    assert_eq!(reload.error_code(), "CONFIG_INVALID");
    // Every step before the swap can fail; the swap cannot. So a rejected
    // reload leaves the process serving exactly what it was serving.
    let after = Call::get("/api/v1/status")
        .send(harness.router())
        .await
        .json();
    assert_eq!(before["revision"], after["revision"]);
}

#[tokio::test]
async fn an_unparsable_stored_config_is_rejected_and_the_running_one_survives() {
    let harness = Harness::new();
    let before = Call::get("/api/v1/status")
        .send(harness.router())
        .await
        .json();

    harness.overwrite_config("this: is: not: valid: yaml:\n  - [");
    let reload = Call::post("/api/v1/config/reload")
        .token(ROOT)
        .send(harness.router())
        .await;

    assert_eq!(reload.status, 400, "{}", reload.body);
    assert_eq!(reload.error_code(), "CONFIG_INVALID");
    let after = Call::get("/api/v1/status")
        .send(harness.router())
        .await
        .json();
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
        .token("reader-token-00000000000000000000000")
        .send(harness.router())
        .await;

    assert_eq!(reply.status, 403, "{}", reply.body);
    assert_eq!(reply.error_code(), "FORBIDDEN");
}

#[tokio::test]
async fn a_stored_config_cannot_authorise_anything_at_all() {
    // The same escalation as the test below, against the handlers it does not
    // reach. Those authorized against a configuration loaded from the store
    // per request, so a token written out of band worked on the very next
    // call -- no reload, nobody's approval. `GET /api/v1/proxies` answered
    // 200 with the whole proxy set, and the write verbs were open the same
    // way.
    let harness = Harness::new();
    let tampered = BASE_CONFIG.replace(
        "    - name: reader",
        "    - name: intruder\n      group: admin\n      token: intruder-token-000000000000000000000\n    - name: reader",
    );
    assert!(
        tampered.contains("intruder-token-000000000000000000000"),
        "the tampered document must actually differ, or this test proves nothing"
    );
    harness.overwrite_config(&tampered);

    // The write verbs. `list` and `read` are `public` in this fixture, so a
    // 200 from those says nothing about who the caller is -- including them
    // would be a test that passes for the wrong reason.
    for (method, path) in [
        ("DELETE", "/api/v1/proxies/alpha"),
        ("PUT", "/api/v1/proxies/alpha"),
        ("POST", "/api/v1/proxies"),
        ("DELETE", "/api/v1/proxies/alpha/templates/x.j2"),
    ] {
        let reply = Call::new(method, path)
            .token("intruder-token-000000000000000000000")
            .send(harness.router())
            .await;
        assert_eq!(
            reply.status, 401,
            "{method} {path} answered {}: {}",
            reply.status, reply.body
        );
    }

    // And the proxy set is untouched, which is the outcome the status codes
    // are standing in for.
    let listed = Call::get("/api/v1/proxies").send(harness.router()).await;
    assert!(listed.body.contains("alpha"), "{}", listed.body);
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
        "    - name: intruder\n      group: admin\n      token: intruder-token-000000000000000000000\n    - name: reader",
    );
    assert!(
        tampered.contains("intruder-token-000000000000000000000"),
        "the tampered document must actually differ, or this test proves nothing"
    );
    harness.overwrite_config(&tampered);

    let reply = Call::post("/api/v1/config/reload")
        .token("intruder-token-000000000000000000000")
        .send(harness.router())
        .await;

    // Unknown to the running configuration, so anonymous, so 401.
    assert_eq!(reply.status, 401, "{}", reply.body);
    assert_eq!(reply.error_code(), "UNAUTHORIZED");

    // And it never took effect, so a second attempt fails the same way
    // rather than succeeding on the strength of the first.
    let again = Call::post("/api/v1/config/reload")
        .token("intruder-token-000000000000000000000")
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

#[tokio::test]
async fn metrics_renders_the_exposition_with_the_registered_content_type() {
    let harness = Harness::new();
    {
        // Recorded into the harness's own recorder, which is the one the
        // handler renders from. A test that recorded into the global
        // recorder would pass here whether or not the handler was wired to
        // anything.
        let _guard = metrics::set_default_local_recorder(&harness.recorder);
        doppel_core::metrics::record_proxy(
            "alpha",
            "GET",
            200,
            Duration::from_millis(5),
            doppel_core::metrics::Outcome::proxied(),
        );
        doppel_core::metrics::record_loss("alpha");
    }

    let reply = Call::get("/metrics").send(harness.router()).await;

    assert_eq!(reply.status, 200);
    assert_eq!(
        reply.content_type.as_deref(),
        Some("text/plain; version=0.0.4; charset=utf-8")
    );
    assert!(
        reply.body.contains("doppel_proxy_request_duration_seconds"),
        "{}",
        reply.body
    );
    assert!(reply.body.contains("doppel_loss_total"), "{}", reply.body);
    assert!(reply.body.contains(r#"proxy="alpha""#), "{}", reply.body);
}

/// Both spellings of every path the API answers.
///
/// Axum stopped redirecting `/x/` to `/x` in 0.8, so without the trailing-slash
/// layer each of these is a 404 -- and a scrape config or a curl carrying the
/// slash gets nothing, with the endpoint sitting right there.
#[tokio::test]
async fn a_trailing_slash_reaches_the_same_endpoint() {
    let harness = Harness::new();
    for (with, without) in [
        ("/metrics/", "/metrics"),
        ("/api/v1/status/", "/api/v1/status"),
        ("/api/v1/proxies/", "/api/v1/proxies"),
        ("/api/v1/schema/", "/api/v1/schema"),
        ("/api/v1/access/", "/api/v1/access"),
    ] {
        let slashed = Call::get(with)
            .token("root-token-0000000000000000000000000")
            .send(harness.router())
            .await;
        let bare = Call::get(without)
            .token("root-token-0000000000000000000000000")
            .send(harness.router())
            .await;
        assert_eq!(slashed.status, bare.status, "{with} against {without}");
        assert_eq!(
            slashed.content_type, bare.content_type,
            "{with} against {without}"
        );
        assert_eq!(slashed.body, bare.body, "{with} against {without}");
    }
}

/// And the root is not a trailing slash to be trimmed away.
#[tokio::test]
async fn the_root_still_answers() {
    let harness = Harness::new();
    let reply = Call::get("/").send(harness.router()).await;
    assert_ne!(reply.status, 404, "the layer trimmed the root itself");
}

/// The admin listener's own latency, labelled by the route template.
///
/// The recorder is the harness's, held for the whole exchange: the middleware
/// records inside the request future, and `oneshot` runs it on this thread, so a
/// thread-local recorder sees it.
#[tokio::test]
async fn an_admin_request_records_its_own_latency_by_route() {
    let harness = Harness::new();
    {
        let _guard = metrics::set_default_local_recorder(&harness.recorder);
        let read = Call::get("/api/v1/proxies/alpha")
            .token("root-token-0000000000000000000000000")
            .send(harness.router())
            .await;
        assert_eq!(read.status, 200, "{}", read.body);
    }

    let exposition = harness.recorder.handle().render();
    assert!(
        exposition.contains(r#"route="/api/v1/proxies/{name}""#),
        "the template, not the path: {exposition}"
    );
    // The proxy's own name must not be in there. One series per proxy is how a
    // cardinality incident starts.
    assert!(!exposition.contains("alpha"), "{exposition}");
    assert!(
        exposition.contains("doppel_admin_request_duration_seconds_bucket"),
        "{exposition}"
    );
}

#[tokio::test]
async fn metrics_needs_no_token() {
    // A scraper is a machine with nowhere to put one.
    let harness = Harness::new();
    assert_eq!(
        Call::get("/metrics").send(harness.router()).await.status,
        200
    );
}

#[tokio::test]
async fn the_openapi_document_is_served_as_json() {
    let harness = Harness::new();
    let reply = Call::get("/openapi.json").send(harness.router()).await;

    assert_eq!(reply.status, 200, "{}", reply.body);
    let doc = reply.json();
    assert_eq!(doc["info"]["title"], "Doppel admin API");
    assert!(
        doc["paths"]["/api/v1/proxies"]["get"].is_object(),
        "{}",
        reply.body
    );
}

#[tokio::test]
async fn the_openapi_document_needs_no_token() {
    // It describes the API rather than exposing any of it, and a client
    // cannot authenticate before it knows how to.
    let harness = Harness::new();
    assert_eq!(
        Call::get("/openapi.json")
            .send(harness.router())
            .await
            .status,
        200
    );
}

#[tokio::test]
async fn swagger_ui_is_served() {
    let harness = Harness::new();
    // The UI root redirects to its index; both are the UI answering rather
    // than the router falling through to a 404.
    let reply = Call::get("/swagger-ui").send(harness.router()).await;
    assert!(
        reply.status.is_success() || reply.status.is_redirection(),
        "{} {}",
        reply.status,
        reply.body
    );

    let index = Call::get("/swagger-ui/index.html")
        .send(harness.router())
        .await;
    assert_eq!(index.status, 200, "{}", index.body);
    assert!(
        index.body.contains("swagger"),
        "{}",
        &index.body[..200.min(index.body.len())]
    );
}

#[tokio::test]
async fn the_documented_error_envelope_is_the_one_the_api_actually_sends() {
    // The document promises `status`, `message` and `code`. This checks a
    // real error response against that promise, so the two cannot drift
    // apart in the direction the document is silent about.
    let harness = Harness::new();
    let doc = Call::get("/openapi.json")
        .send(harness.router())
        .await
        .json();
    let documented: Vec<String> = doc["components"]["schemas"]["ErrorBody"]["properties"]
        .as_object()
        .expect("ErrorBody properties")
        .keys()
        .cloned()
        .collect();

    let actual = Call::get("/api/v1/proxies/nope")
        .send(harness.router())
        .await
        .json();
    let mut fields: Vec<String> = actual
        .as_object()
        .expect("an object")
        .keys()
        .cloned()
        .collect();
    fields.sort();
    let mut documented = documented;
    documented.sort();
    assert_eq!(fields, documented);
}

#[tokio::test]
async fn an_unknown_route_answers_the_error_envelope_not_an_empty_body() {
    // The paths a client hits by accident were the ones axum answered with
    // no body at all, so a client parsing errors uniformly got nothing to
    // parse exactly when it most needed the message.
    let harness = Harness::new();
    let reply = Call::get("/api/v1/nope").send(harness.router()).await;

    assert_eq!(reply.status, 404, "{}", reply.body);
    assert_eq!(reply.error_code(), "NOT_FOUND");
    assert!(reply.body.contains("/api/v1/nope"), "{}", reply.body);
}

#[tokio::test]
async fn a_wrong_method_answers_405_with_both_an_envelope_and_an_allow_header() {
    // Distinct from the 404 above: the resource exists and the verb is
    // wrong, which the client fixes differently. RFC 9110 also requires the
    // `Allow` header, so the envelope must not come at its expense.
    let harness = Harness::new();
    let reply = Call::delete("/api/v1/proxies")
        .token(ROOT)
        .send(harness.router())
        .await;

    assert_eq!(reply.status, 405, "{}", reply.body);
    assert_eq!(reply.error_code(), "METHOD_NOT_ALLOWED");
    let allow = reply.allow.expect("405 must carry Allow");
    assert!(allow.contains("GET"), "{allow}");
    assert!(allow.contains("POST"), "{allow}");
    assert!(!allow.contains("DELETE"), "{allow}");
}

#[tokio::test]
async fn an_oversized_configuration_document_is_refused_in_the_envelope() {
    // axum's own body limit answers with a plain-text 413 from a layer that
    // cannot produce this envelope, so every route that takes a body does
    // its own bounding.
    let harness = Harness::new();
    let reply = Call::post("/api/v1/proxies")
        .token(ROOT)
        .raw("x".repeat(2 * 1024 * 1024))
        .send(harness.router())
        .await;

    assert_eq!(reply.status, 413, "{}", reply.body);
    assert_eq!(reply.error_code(), "UPLOAD_TOO_LARGE");
    assert_eq!(harness.stored().proxies.len(), 2);
}

#[tokio::test]
async fn a_configuration_document_of_ordinary_size_is_not_refused() {
    // The bound must not be so tight that a real document trips it. This one
    // carries a comfortably large headers map.
    let harness = Harness::new();
    let mut proxy = proxy_json("gamma", "https://gamma.example.com/api/");
    let headers: serde_json::Map<String, serde_json::Value> = (0..200)
        .map(|i| (format!("X-Header-{i}"), serde_json::json!("value")))
        .collect();
    proxy["proxy"]["headers"] = serde_json::Value::Object(headers);

    let reply = Call::post("/api/v1/proxies")
        .token(ROOT)
        .json(proxy)
        .send(harness.router())
        .await;

    assert_eq!(reply.status, 201, "{}", reply.body);
}

#[tokio::test]
async fn turning_the_admin_listener_off_is_reported_as_needing_a_restart() {
    // `admin` is already in the unapplied set, so the new field inherits the
    // behaviour -- but inheriting it is a claim, and the claim is what this
    // checks. An operator who sets `enable: false` and reloads must be told
    // the listener is still up rather than discovering it later.
    let harness = Harness::new();
    harness.overwrite_config(&BASE_CONFIG.replace("admin:\n", "admin:\n  enable: false\n"));

    let reload = Call::post("/api/v1/config/reload")
        .token(ROOT)
        .send(harness.router())
        .await;

    assert_eq!(reload.status, 200, "{}", reload.body);
    assert_eq!(reload.json()["unapplied"], serde_json::json!(["admin"]));

    // And the listener really is still answering, which is what "unapplied"
    // means here.
    assert_eq!(
        Call::get("/api/v1/status")
            .send(harness.router())
            .await
            .status,
        200
    );
}
