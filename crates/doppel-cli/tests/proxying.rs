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

/// The system variables, rendered by a mock in a running binary.
///
/// `real_ip` is the one worth driving through a socket rather than a unit test:
/// the chain is `X-Real-IP`, then the leftmost `X-Forwarded-For`, then the peer,
/// and only a real connection has a peer to fall back to.
#[test]
fn a_mock_renders_the_system_variables() {
    let up = upstream();
    let server = Server::start_with(up.port, |ports, socket, templates| {
        format!(
            r#"
server:
  host: "127.0.0.1"
  port: {proxy}
admin:
  host: "127.0.0.1"
  port: {admin}
  tokens: []
  access: {{}}
  upload:
    limit: 1Mi
control:
  socket: {socket}
templates:
  dir: {templates}
proxies:
  - name: p1
    type: http
    url: "http://127.0.0.1:{upstream}/"
    mocks:
      - name: who
        request:
          method: GET
          url: ^/who$
        response:
          status: 200
          json: '{{"proxy": "{{{{ proxy_name }}}}", "mock": "{{{{ mock_name }}}}", "version": "{{{{ doppel_version }}}}", "real_ip": "{{{{ real_ip }}}}", "peer_ip": "{{{{ peer_ip }}}}", "method": "{{{{ method }}}}", "path": "{{{{ path }}}}", "host": "{{{{ host }}}}", "request_id": "{{{{ request_id }}}}"}}'
"#,
            proxy = ports.server,
            admin = ports.admin,
            upstream = ports.upstream,
            socket = socket.display(),
            templates = templates.display(),
        )
    });

    // Nothing in front of this, so `real_ip` falls back to the peer -- and the
    // peer is loopback, because that is where the test client is.
    let (status, body) = server.get("/who");
    assert_eq!(status, 200, "{body}");
    let plain: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(plain["proxy"], "p1");
    assert_eq!(plain["mock"], "who");
    assert_eq!(plain["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(plain["method"], "GET");
    assert_eq!(plain["path"], "/who");
    assert_eq!(plain["peer_ip"], "127.0.0.1");
    assert_eq!(plain["real_ip"], "127.0.0.1", "with no header, the peer");
    assert!(
        !plain["request_id"].as_str().unwrap().is_empty(),
        "a request id is always bound, minted when the client sends none"
    );
    assert!(
        plain["host"].as_str().unwrap().starts_with("127.0.0.1:"),
        "host is what the client asked for: {}",
        plain["host"]
    );

    // With a proxy in front, `real_ip` is what that proxy says.
    let url = format!("http://127.0.0.1:{}/who", server.port());
    let response = reqwest::blocking::Client::new()
        .get(&url)
        .header("x-real-ip", "203.0.113.7")
        .header("x-forwarded-for", "198.51.100.1, 10.0.0.8")
        .send()
        .unwrap();
    let claimed: serde_json::Value = serde_json::from_str(&response.text().unwrap()).unwrap();
    assert_eq!(claimed["real_ip"], "203.0.113.7", "X-Real-IP comes first");
    assert_eq!(
        claimed["peer_ip"], "127.0.0.1",
        "and the peer is still the socket's own, which nobody can claim"
    );

    // Without X-Real-IP, the leftmost X-Forwarded-For entry: the original client
    // rather than the hop next to us.
    let forwarded = reqwest::blocking::Client::new()
        .get(&url)
        .header("x-forwarded-for", "198.51.100.1, 10.0.0.8")
        .send()
        .unwrap();
    let chained: serde_json::Value = serde_json::from_str(&forwarded.text().unwrap()).unwrap();
    assert_eq!(chained["real_ip"], "198.51.100.1");
}
