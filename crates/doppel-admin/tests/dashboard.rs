//! The dashboard routes: the page, its assets, and `robots.txt`.

mod common;

use common::{BASE_CONFIG, Call, Harness};

/// Whether this binary carries the built dashboard.
///
/// A source build with no `frontend/dist` has no assets to serve, and the tests
/// below that need real ones say so rather than failing for the wrong reason.
/// CI sets `DOPPEL_REQUIRE_DASHBOARD_ASSETS`, which
/// `the_assets_are_present_when_they_are_required` turns into a failure -- the
/// same arrangement `DOPPEL_REQUIRE_DATABASE` makes for the store suites, and for
/// the same reason: `cargo test` captures the output of passing tests, so a skip
/// notice reaches nobody.
fn built() -> bool {
    doppel_admin::dashboard::is_built()
}

/// The configuration element's contents, parsed.
fn injected(body: &str) -> serde_json::Value {
    let at = body
        .find("id=\"doppel-config\"")
        .unwrap_or_else(|| panic!("no config element in the page:\n{body}"));
    let open_ends = body[at..].find('>').expect("an unclosed opening tag") + at + 1;
    let close = body[open_ends..].find("</script>").expect("no closing tag") + open_ends;
    serde_json::from_str(body[open_ends..close].trim()).unwrap_or_else(|err| {
        panic!(
            "the injected block is not JSON ({err}): {}",
            &body[open_ends..close]
        )
    })
}

#[tokio::test]
async fn the_assets_are_present_when_they_are_required() {
    // Not a skip: this is the test that makes the others' skips visible.
    if std::env::var_os("DOPPEL_REQUIRE_DASHBOARD_ASSETS").is_some() {
        assert!(
            built(),
            "DOPPEL_REQUIRE_DASHBOARD_ASSETS is set and this binary has no embedded dashboard; \
             run `npm --prefix frontend ci && npm --prefix frontend run build` before `cargo test`"
        );
    }
}

#[tokio::test]
async fn the_root_serves_the_page_with_the_injected_configuration() {
    if !built() {
        return;
    }
    let harness = Harness::new();
    let reply = Call::get("/").send(harness.router()).await;

    assert_eq!(reply.status, 200);
    assert_eq!(
        reply.content_type.as_deref(),
        Some("text/html; charset=utf-8")
    );
    // Never cached: the document carries the configuration, so a cached copy
    // would keep showing values from before the last reload.
    assert_eq!(reply.header("cache-control"), Some("no-store"));

    let config = injected(&reply.body);
    assert_eq!(config["title"], "Doppel");
    // Nothing named this one, so the page is free to draw its wordmark.
    assert_eq!(config["titleIsDefault"], true);
    assert_eq!(config["public"], false);
    assert_eq!(config["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(config["authHeader"], "X-Proxy-Authorization");
    assert_eq!(config["refreshMs"], 60_000);
    // The footer's copyright year, from the build stamp rather than the browser's
    // clock. Asserted as a range because the value moves with every build.
    let year = config["copyrightYear"]
        .as_u64()
        .expect("copyrightYear is a number");
    assert!((2026..=9999).contains(&year), "copyrightYear is {year}");
}

#[tokio::test]
async fn the_page_reports_the_configured_title_and_auth_header() {
    if !built() {
        return;
    }
    let config = BASE_CONFIG
        .replacen("  tokens:", "  title: \"Doppel (staging)\"\n  tokens:", 1)
        .replacen(
            "  access:",
            "  auth:\n    header: X-Admin-Token\n  access:",
            1,
        );
    let harness = Harness::with_config(&config);
    let body = Call::get("/").send(harness.router()).await.body;

    let injected = injected(&body);
    assert_eq!(injected["title"], "Doppel (staging)");
    // Named, so the page shows the name rather than the wordmark.
    assert_eq!(injected["titleIsDefault"], false);
    assert_eq!(injected["authHeader"], "X-Admin-Token");
}

#[tokio::test]
async fn a_public_configuration_is_reported_to_the_page() {
    if !built() {
        return;
    }
    // The page uses this to decide whether to ask for a token at all, so getting
    // it wrong means a public deployment showing a pointless dialog.
    let config = BASE_CONFIG.replacen("  tokens:", "  public: true\n  tokens:", 1);
    let harness = Harness::with_config(&config);
    let body = Call::get("/").send(harness.router()).await.body;

    assert_eq!(injected(&body)["public"], true);
}

#[tokio::test]
async fn a_hostile_title_cannot_close_the_script_element() {
    if !built() {
        return;
    }
    // `AdminTitle` accepts markup on purpose -- it is legal text -- so this is
    // the place the escaping has to hold. Without it the title would end the
    // JSON element early and the rest would be parsed as HTML, which is a
    // scripting hole reachable by editing one configuration field.
    let hostile = "</script><script>alert(1)</script>";
    let config = BASE_CONFIG.replacen(
        "  tokens:",
        &format!("  title: \"{hostile}\"\n  tokens:"),
        1,
    );
    let harness = Harness::with_config(&config);
    let body = Call::get("/").send(harness.router()).await.body;

    // The title survives intact where it belongs...
    assert_eq!(injected(&body)["title"], hostile);
    // ...and the page contains exactly the script elements it is supposed to:
    // the config block and the module. An injected one would make three.
    assert_eq!(
        body.matches("<script").count(),
        2,
        "an extra script element was injected:\n{body}"
    );
}

#[tokio::test]
async fn an_asset_is_served_with_immutable_caching() {
    if !built() {
        return;
    }
    let harness = Harness::new();
    let page = Call::get("/").send(harness.router()).await.body;

    // Whatever vite named the entry chunk this build, taken from the page rather
    // than hard-coded: the name carries a content hash and changes every time
    // the frontend changes.
    let at = page
        .find("/static/assets/")
        .unwrap_or_else(|| panic!("the page loads no asset:\n{page}"));
    let rest = &page[at..];
    let end = rest.find('"').expect("an unterminated attribute");
    let url = &rest[..end];

    let reply = Call::get(url).send(harness.router()).await;
    assert_eq!(reply.status, 200, "{url}");
    assert_eq!(
        reply.header("cache-control"),
        Some("public, max-age=31536000, immutable")
    );
    assert_eq!(
        reply.content_type.as_deref(),
        Some("text/javascript; charset=utf-8")
    );
    assert!(!reply.bytes.is_empty());
}

#[tokio::test]
async fn the_favicon_is_served_as_bytes_with_its_own_type() {
    if !built() {
        return;
    }
    // The one embedded asset that is not text, and the one whose content type a
    // browser will not guess correctly from the bytes.
    let harness = Harness::new();
    let reply = Call::get("/static/favicon.ico")
        .send(harness.router())
        .await;

    assert_eq!(reply.status, 200);
    assert_eq!(reply.content_type.as_deref(), Some("image/x-icon"));
    assert_eq!(
        &reply.bytes[..4],
        &[0x00, 0x00, 0x01, 0x00],
        "not an ICO header"
    );
}

#[tokio::test]
async fn an_unknown_static_path_is_a_404_in_the_envelope() {
    let harness = Harness::new();
    let reply = Call::get("/static/nope.js").send(harness.router()).await;

    assert_eq!(reply.status, 404);
    // Deliberately not the page. A single-page fallback that answered 200 with
    // HTML here would make a mistyped API path look like a success to a client
    // and like a working dashboard to a human.
    assert_eq!(reply.json()["code"], "NOT_FOUND");
}

/// The scrape path belongs to the scraper, not to the page.
///
/// This is what a Prometheus scrape of a running Doppel actually got while the
/// exposition lived under `/api/v1/`: `200` with the dashboard's HTML, because
/// the page answers every GET outside `/api/` and `/static/`. A scraper reports
/// that as a parse failure at best, and the operator sees no metrics with
/// nothing anywhere saying why.
#[tokio::test]
async fn metrics_is_the_exposition_and_not_the_page() {
    let harness = Harness::new();
    let reply = Call::get("/metrics").send(harness.router()).await;

    assert_eq!(reply.status, 200);
    assert_eq!(
        reply.content_type.as_deref(),
        Some("text/plain; version=0.0.4; charset=utf-8"),
        "the dashboard fallback answered the scrape path"
    );
    assert!(!reply.body.contains("<!doctype html"), "{}", reply.body);
}

#[tokio::test]
async fn robots_disallows_everything() {
    let harness = Harness::new();
    let reply = Call::get("/robots.txt").send(harness.router()).await;

    assert_eq!(reply.status, 200);
    assert_eq!(
        reply.content_type.as_deref(),
        Some("text/plain; charset=utf-8")
    );
    assert_eq!(reply.body, "User-agent: *\nDisallow: /\n");
}

#[tokio::test]
async fn every_dashboard_response_forbids_indexing() {
    // Three routes, three ways a crawler arrives. The header is the one that
    // covers a crawler which never reads `robots.txt` and never parses markup.
    let harness = Harness::new();
    for path in ["/", "/robots.txt", "/static/nope.js"] {
        let reply = Call::get(path).send(harness.router()).await;
        assert_eq!(
            reply.header("x-robots-tag"),
            Some("noindex, nofollow, noarchive"),
            "{path}"
        );
    }
}

#[tokio::test]
async fn the_page_carries_a_content_security_policy() {
    if !built() {
        return;
    }
    let harness = Harness::new();
    let reply = Call::get("/").send(harness.router()).await;

    let policy = reply
        .header("content-security-policy")
        .unwrap_or_else(|| panic!("no policy on the page: {:?}", reply.headers));
    // `script-src 'self'` without `unsafe-inline` is what the JSON-element
    // approach buys: an inline `window.config = ...` would have needed the
    // opposite.
    assert!(policy.contains("script-src 'self'"), "{policy}");
    assert!(!policy.contains("unsafe-inline"), "{policy}");
    assert!(policy.contains("frame-ancestors 'none'"), "{policy}");
    assert_eq!(reply.header("x-content-type-options"), Some("nosniff"));
}

#[tokio::test]
async fn a_disabled_dashboard_serves_none_of_the_three_routes() {
    let config = BASE_CONFIG.replacen("  tokens:", "  dashboard: false\n  tokens:", 1);
    let harness = Harness::with_config(&config);

    for path in ["/", "/robots.txt", "/static/anything.js"] {
        let reply = Call::get(path).send(harness.router()).await;
        assert_eq!(reply.status, 404, "{path} must not be routed");
        assert_eq!(reply.json()["code"], "NOT_FOUND", "{path}");
    }

    // And the JSON API is untouched, which is the whole point of the flag being
    // about the dashboard rather than about the listener.
    assert_eq!(
        Call::get("/api/v1/proxies")
            .send(harness.router())
            .await
            .status,
        200
    );
}

#[tokio::test]
async fn a_binary_without_the_assets_says_what_to_build() {
    if built() {
        return;
    }
    let harness = Harness::new();
    let reply = Call::get("/").send(harness.router()).await;

    assert_eq!(reply.status, 503);
    assert_eq!(reply.json()["code"], "DASHBOARD_NOT_BUILT");
    assert!(
        reply.json()["message"]
            .as_str()
            .is_some_and(|message| message.contains("npm")),
        "the message must name the command that fixes it: {}",
        reply.body
    );
}

#[tokio::test]
async fn a_client_side_route_reloaded_arrives_at_the_page() {
    if !built() {
        return;
    }
    // The reason the API moved under `/api/`: the dashboard has a page at `/status`
    // and the API used to have an endpoint there, so a reload on that tab answered
    // with JSON. Now every GET outside `/api/` and `/static/` is the page.
    let harness = Harness::new();
    for path in [
        "/status",
        "/proxies/alpha",
        "/proxies/alpha/templates",
        "/anything",
    ] {
        let reply = Call::get(path).send(harness.router()).await;
        assert_eq!(reply.status, 200, "{path}");
        assert_eq!(
            reply.content_type.as_deref(),
            Some("text/html; charset=utf-8"),
            "{path}"
        );
        assert!(reply.body.contains("id=\"doppel-config\""), "{path}");
    }
}

#[tokio::test]
async fn a_mistyped_api_path_still_answers_as_the_api() {
    // The other half of the division. A fallback that served the page for
    // everything would make a client read HTML as a successful response, and a
    // typo'd endpoint look like it worked.
    let harness = Harness::new();
    for path in ["/api/v1/proxes", "/api/v1/status/extra", "/api/nope"] {
        let reply = Call::get(path).send(harness.router()).await;
        assert_eq!(reply.status, 404, "{path}");
        assert_eq!(reply.json()["code"], "NOT_FOUND", "{path}");
    }

    // And a missing asset stays a 404, because answering it with the page means a
    // typo'd script tag loads HTML and fails somewhere else entirely.
    let asset = Call::get("/static/nope.js").send(harness.router()).await;
    assert_eq!(asset.status, 404);
    assert_eq!(asset.json()["code"], "NOT_FOUND");
}

#[tokio::test]
async fn a_write_to_a_path_that_does_not_exist_is_not_answered_with_the_page() {
    // A GET is a page; a POST is not. Without that distinction a client posting to
    // a mistyped path would read an HTML document as its answer.
    let harness = Harness::new();
    let reply = Call::post("/not-an-endpoint").send(harness.router()).await;
    assert_eq!(reply.status, 404);
    assert_eq!(reply.json()["code"], "NOT_FOUND");
}
