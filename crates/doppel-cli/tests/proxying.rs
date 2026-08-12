//! End-to-end: a real `doppel` binary proxying a request to a real upstream.

mod common;

use common::{Server, upstream};

#[test]
fn proxies_a_request_end_to_end() {
    let up = upstream();
    let server = Server::start(up.port);
    let (status, body) = server.get("/hello");
    assert_eq!(status, 200);
    assert_eq!(body, "upstream saw /hello");
}

/// The whole chain, through the built binary: an upstream redirect to its own
/// authority comes back naming Doppel.
///
/// The unit tests pin the rewriting rules; this pins the wiring, which is what
/// was missing when an operator reported a redirect still pointing at the
/// upstream. `server.external_url` is unset in this configuration, so the
/// address comes from `server.host` and `server.port` -- the fallback that makes
/// the default deployment work without configuring anything.
#[test]
fn a_redirect_to_the_upstreams_own_host_comes_back_naming_doppel() {
    let up = upstream();
    let server = Server::start(up.port);

    let (status, location) = server.get_unfollowed("/redirect-self");

    assert_eq!(status, 302);
    assert_eq!(
        location.as_deref(),
        Some(format!("http://127.0.0.1:{}/moved", server.port()).as_str()),
        "the Location must name Doppel, not the upstream"
    );
}

/// And `DOPPEL_EXTERNAL_URL` over the top of it, which is the answer for a
/// deployment behind a port mapping or an ingress: the address Doppel bound is
/// not the address the client used.
#[test]
fn the_environment_overrides_which_host_a_rewritten_redirect_names() {
    let up = upstream();
    let server = Server::start_with_env(
        up.port,
        common::config,
        &[("DOPPEL_EXTERNAL_URL", "https://doppel.example.com/")],
    );

    let (status, location) = server.get_unfollowed("/redirect-self");

    assert_eq!(status, 302);
    assert_eq!(
        location.as_deref(),
        Some("https://doppel.example.com/moved")
    );
}

/// A body that names the upstream comes back naming Doppel.
///
/// Through the built binary, both listeners, so this covers the wiring as well as
/// the rules: `rewrite_urls` defaults to on, and the address comes from
/// `server.host` and `server.port` like a rewritten redirect's does.
#[test]
fn a_body_that_names_the_upstream_comes_back_naming_doppel() {
    let up = upstream();
    let server = Server::start(up.port);

    let (status, body) = server.get("/page");

    assert_eq!(status, 200, "{body}");
    assert!(
        body.contains(&format!("http://127.0.0.1:{}/next", server.port())),
        "the link must point at Doppel: {body}"
    );
    assert!(
        !body.contains(&format!("http://127.0.0.1:{}/next", up.port)),
        "and not at the upstream: {body}"
    );
    // A host that merely contains the upstream's is a different host, and is left
    // exactly as the upstream wrote it.
    assert!(
        body.contains(&format!("http://cdn.127.0.0.1:{}/logo.png", up.port)),
        "a different host must survive: {body}"
    );
}
