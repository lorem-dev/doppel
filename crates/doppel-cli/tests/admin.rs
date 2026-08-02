//! End-to-end through the built binary: the admin listener alongside the
//! proxy one.
//!
//! These go over a real socket to a real process, which is what the
//! `doppel-admin` suites deliberately do not do -- they drive the router
//! directly. What is proved here is the wiring: that `serve` binds both
//! ports, shares one runtime and one reload lock between them, and shuts them
//! down together.

mod common;

use common::{Ports, SECRET_TOKEN, Server, config, upstream};

/// A configuration whose one proxy has a mock declaring a template file, so
/// the upload-render-delete round trip has something to work with.
fn config_with_a_mock(
    ports: Ports,
    socket: &std::path::Path,
    templates: &std::path::Path,
) -> String {
    let base = config(ports, socket, templates);
    let with_mock = base.replace(
        "    resolve:\n      type: default\n",
        "    resolve:\n      type: default\n    mocks:\n      - name: greeting\n        \
         request:\n          method: GET\n          url: /greeting/\n        response:\n          \
         status: 200\n          template: greeting.json.j2\n",
    );
    assert_ne!(
        with_mock, base,
        "the fixture must actually add a mock, or these tests prove nothing"
    );
    with_mock
}

struct Admin {
    port: u16,
}

impl Admin {
    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }

    fn request(
        &self,
        method: &str,
        path: &str,
        token: Option<&str>,
        body: Option<&str>,
    ) -> (u16, String) {
        let mut request =
            reqwest::blocking::Client::new().request(method.parse().unwrap(), self.url(path));
        if let Some(token) = token {
            request = request.header("X-Proxy-Authorization", format!("Bearer {token}"));
        }
        if let Some(body) = body {
            request = request
                .header("Content-Type", "application/json")
                .body(body.to_owned());
        }
        let response = request.send().expect("the admin listener answers");
        let status = response.status().as_u16();
        (status, response.text().unwrap())
    }

    fn get(&self, path: &str) -> (u16, String) {
        self.request("GET", path, Some(SECRET_TOKEN), None)
    }

    fn anonymous_get(&self, path: &str) -> (u16, String) {
        self.request("GET", path, None, None)
    }
}

fn start() -> (Server, Admin, common::Upstream) {
    let up = upstream();
    let server = Server::start_with(up.port, config_with_a_mock);
    let admin = Admin {
        port: server.admin_port(),
    };
    (server, admin, up)
}

#[test]
fn both_listeners_are_up_and_serve_different_things() {
    let (server, admin, _up) = start();

    // The proxy port proxies.
    server.get("/anything");

    // The admin port answers the admin API, on a port the proxy's
    // catch-all fallback never sees.
    let (status, body) = admin.get("/api/v1/proxies");
    assert_eq!(status, 200, "{body}");
    assert!(body.contains("\"p1\""), "{body}");
}

#[test]
fn status_and_metrics_are_scrapeable_without_a_token() {
    let (server, admin, _up) = start();
    server.get("/anything");

    let (status, body) = admin.anonymous_get("/status");
    assert_eq!(status, 200, "{body}");
    assert!(body.contains("\"revision\""), "{body}");
    assert!(body.contains("\"p1\""), "{body}");

    let (status, body) = admin.anonymous_get("/metrics");
    assert_eq!(status, 200, "{body}");
    // The request above went through the pipeline, so the proxy histogram
    // must have it. An empty exposition here would mean the recorder the
    // handler renders from is not the one the pipeline records into.
    assert!(
        body.contains("doppel_proxy_request_duration_seconds"),
        "{body}"
    );
    assert!(body.contains("proxy=\"p1\""), "{body}");
    // The cardinality rule, checked against a real request's real path.
    assert!(!body.contains("/anything"), "{body}");
}

#[test]
fn a_write_without_a_token_is_refused() {
    let (_server, admin, _up) = start();
    let (status, body) = admin.request(
        "POST",
        "/api/v1/proxies",
        None,
        Some(r#"{"proxy":{"name":"p2","type":"http","url":"https://example.com/"}}"#),
    );

    assert_eq!(status, 401, "{body}");
    assert!(body.contains("UNAUTHORIZED"), "{body}");
}

#[test]
fn a_proxy_created_over_http_serves_traffic_after_a_reload() {
    let (server, admin, up) = start();

    // `p1` is the default resolver, so the new one resolves by header.
    let created = format!(
        r#"{{"proxy":{{"name":"p2","type":"http","url":"http://127.0.0.1:{}/",
           "resolve":{{"type":"header","header":"X-Proxy-Name"}}}}}}"#,
        up.port
    );
    let (status, body) = admin.request(
        "POST",
        "/api/v1/proxies",
        Some(SECRET_TOKEN),
        Some(&created),
    );
    assert_eq!(status, 201, "{body}");

    // Written to the store, but not yet running.
    let (_, before) = admin.anonymous_get("/status");
    assert!(!before.contains("\"p2\""), "{before}");

    let (status, body) = admin.request("POST", "/api/v1/config/reload", Some(SECRET_TOKEN), None);
    assert_eq!(status, 200, "{body}");
    assert!(body.contains("\"proxies\":2"), "{body}");

    let (_, after) = admin.anonymous_get("/status");
    assert!(after.contains("\"p2\""), "{after}");

    // And it actually serves: the header picks it.
    let (status, body) = server.get_with_header("/anything", "X-Proxy-Name", "p2");
    assert_eq!(status, 200, "{body}");
}

#[test]
fn a_template_uploaded_over_http_is_rendered_without_a_reload() {
    // No reload, deliberately. The mock is already in the running
    // configuration; only its file was missing, and a template is read from
    // disk at render time rather than compiled into the runtime. Requiring a
    // reload here would mean the upload endpoint could not be used to fix a
    // broken template on a live process, which is most of the point of it.
    let (server, admin, _up) = start();

    // Before the upload the mock matches and then cannot render, which is
    // what makes the assertion afterwards about the upload rather than about
    // the mock existing.
    let (status, body) = server.get("/greeting/");
    assert_eq!(status, 500, "{body}");
    assert!(body.contains("TEMPLATE_NOT_FOUND"), "{body}");

    let (status, body) = admin.request(
        "POST",
        "/api/v1/proxies/p1/templates/greeting.json.j2",
        Some(SECRET_TOKEN),
        Some(r#"{"hello": "world"}"#),
    );
    assert_eq!(status, 204, "{body}");

    let (status, body) = admin.get("/api/v1/proxies/p1/templates");
    assert_eq!(status, 200, "{body}");
    assert!(body.contains("greeting.json.j2"), "{body}");

    let (status, body) = server.get("/greeting/");
    assert_eq!(status, 200, "{body}");
    assert!(body.contains("world"), "{body}");
}

#[test]
fn deleting_a_proxy_over_http_removes_its_templates_from_disk() {
    let (server, admin, _up) = start();

    let (status, body) = admin.request(
        "POST",
        "/api/v1/proxies/p1/templates/greeting.json.j2",
        Some(SECRET_TOKEN),
        Some("{}"),
    );
    assert_eq!(status, 204, "{body}");
    let on_disk = server.templates.join("p1").join("greeting.json.j2");
    assert!(on_disk.exists(), "the upload should have landed on disk");

    // Deleting the only proxy is refused by validation, so add a second one
    // first -- which also proves the two operations compose.
    let (status, body) = admin.request(
        "POST",
        "/api/v1/proxies",
        Some(SECRET_TOKEN),
        Some(
            r#"{"proxy":{"name":"p2","type":"http","url":"https://example.com/",
               "resolve":{"type":"header","header":"X-Proxy-Name"}}}"#,
        ),
    );
    assert_eq!(status, 201, "{body}");

    let (status, body) = admin.request("DELETE", "/api/v1/proxies/p1", Some(SECRET_TOKEN), None);
    assert_eq!(status, 204, "{body}");
    assert!(
        !on_disk.exists(),
        "deleting the proxy must take its templates with it"
    );
}

#[test]
fn the_openapi_document_and_swagger_ui_are_served() {
    let (_server, admin, _up) = start();

    let (status, body) = admin.anonymous_get("/openapi.json");
    assert_eq!(status, 200, "{body}");
    assert!(body.contains("\"/api/v1/proxies\""), "{body}");

    let (status, body) = admin.anonymous_get("/swagger-ui/index.html");
    assert_eq!(status, 200, "{body}");
    assert!(body.to_lowercase().contains("swagger"), "{body}");
}

#[test]
fn the_admin_port_refuses_to_start_when_it_is_already_taken() {
    // Half a process -- proxy traffic served, no way to administer it -- is
    // worse than a refusal the operator sees immediately.
    let up = upstream();
    let taken = std::net::TcpListener::bind("127.0.0.1:0").expect("bind a port to squat on");
    let squatted = taken.local_addr().unwrap().port();

    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("d.sock");
    let templates = dir.path().join("templates");
    let config_path = dir.path().join("main.yaml");
    std::fs::write(
        &config_path,
        config(
            Ports {
                server: common::free_port(),
                admin: squatted,
                upstream: up.port,
            },
            &socket,
            &templates,
        ),
    )
    .unwrap();

    // Spawned with a deadline rather than `output()`, which waits for exit.
    // The whole point of this test is that the process must NOT start, so if
    // the refusal ever regresses the process runs forever and `output()`
    // would hang the suite instead of failing it. A test whose failure mode
    // is "never finishes" reports nothing.
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_doppel"))
        .args(["serve", "--config"])
        .arg(&config_path)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("doppel runs");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let status = loop {
        match child.try_wait().expect("waiting on the child") {
            Some(status) => break status,
            None if std::time::Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                panic!(
                    "doppel was still running after 10s with admin port {squatted} already \
                     bound; it should have refused to start"
                );
            }
            None => std::thread::sleep(std::time::Duration::from_millis(50)),
        }
    };

    assert!(!status.success(), "startup should have failed");
    let output = child.wait_with_output().expect("collecting output");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(&squatted.to_string()),
        "the message must name the port that could not be bound: {stderr}"
    );
    assert!(
        stderr.contains("admin"),
        "the message must say which listener failed: {stderr}"
    );
    drop(taken);
}
