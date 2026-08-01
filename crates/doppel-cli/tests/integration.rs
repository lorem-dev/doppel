//! End-to-end tests: a real binary, a real upstream, real signals.
//!
//! These start the actual `doppel` binary as a child process and talk to it
//! over a real TCP socket and a real Unix control socket. That is slower and
//! more failure-prone than a unit test, so most of the code below exists to
//! keep the suite deterministic: every wait is a poll against an observable
//! condition with a deadline, every failure carries enough context to explain
//! itself, and every child process is torn down even when a test panics.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// Ask the OS for a free port, then release it before handing the number to
/// the config file `doppel serve` will read.
///
/// This is a genuine time-of-check-to-time-of-use race: another process can
/// take the port between the `drop` here and `doppel`'s own `bind`. The
/// obvious fix -- let the server bind an ephemeral port (0) and discover
/// which one it got -- does not work here: validation rule V1 rejects a `0`
/// port outright, and the port has to be a concrete number already written
/// into the config file before `doppel` (a separate process) even starts, so
/// there is no way to hand it an already-open listener without a
/// socket-activation mechanism this codebase does not have. That leaves the
/// race in place for the server's own port; the mitigation actually taken is
/// in `wait_until_ready` below, which turns a lost race into an immediate,
/// specific failure (the child's own stderr, surfaced) instead of a bare
/// ten-second timeout or a mysterious "connection refused".
///
/// The admin port is allocated the same way in `config()` below, but nothing
/// binds to it in this phase (the admin server does not exist yet), so a
/// collision there cannot manifest as a test failure at all.
fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

struct Upstream {
    port: u16,
    _handle: std::thread::JoinHandle<()>,
}

/// A blocking upstream that answers every request with its path.
///
/// Unlike `free_port`, this has no discard-and-reuse step: the listener is
/// bound once, synchronously, right here, and the same listener is moved
/// into the background thread -- so there is no window in which another
/// process could steal the port between choosing it and using it.
fn upstream() -> Upstream {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let request = read_request_head(&mut stream);
            let path = request.split_whitespace().nth(1).unwrap_or("/").to_owned();
            let body = format!("upstream saw {path}");
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    Upstream {
        port,
        _handle: handle,
    }
}

/// Read until the blank line that ends the request headers (`\r\n\r\n`),
/// rather than trusting a single fixed-size `read` to return the whole
/// request.
///
/// A GET request from `reqwest` is small enough that one `read` almost
/// always returns it in one piece, but "almost always" is exactly the kind
/// of assumption that turns into a rare, unreproducible flake under load or
/// in CI. There is a second, sharper reason to drain fully rather than
/// simply: every request in this suite is a bodyless GET, so once the blank
/// line has arrived there is nothing left to read, and it is safe to close
/// the connection immediately afterwards. Responding while bytes the client
/// already sent are still sitting unread in the kernel's receive queue can
/// make the OS send a `RST` instead of a clean close when the socket is
/// dropped, which would truncate the very response the client is waiting
/// for -- a second, independent source of flakiness beyond a short read.
fn read_request_head(stream: &mut TcpStream) -> String {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 1024];
    loop {
        let read = stream.read(&mut chunk).unwrap_or(0);
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if buffer.windows(4).any(|w| w == b"\r\n\r\n") || buffer.len() > 64 * 1024 {
            break;
        }
    }
    String::from_utf8_lossy(&buffer).into_owned()
}

fn config(server_port: u16, upstream_port: u16, socket: &Path, templates: &Path) -> String {
    format!(
        r#"
server:
  host: "127.0.0.1"
  port: {server_port}
logging:
  level: info
  format: json
control:
  socket: {socket}
templates:
  dir: {templates}
admin:
  host: "127.0.0.1"
  port: {}
  tokens:
    - name: user1
      group: admin
      token: {SECRET_TOKEN}
  access: {{}}
  upload:
    limit: 1M
proxies:
  - name: p1
    type: http
    url: "http://127.0.0.1:{upstream_port}/"
    resolve:
      type: default
"#,
        free_port(),
        socket = socket.display(),
        templates = templates.display(),
    )
}

/// A recognizable token value, so a test can assert it never reaches stdout.
const SECRET_TOKEN: &str = "b6e1f0c2-secret-do-not-log";

/// `sockaddr_un.sun_path` has a small, fixed capacity: 104 bytes including
/// the terminating NUL on macOS/BSD, 108 on Linux. `ControlServer::bind`
/// (see `crates/doppel-cli/src/control/server.rs`) additionally stages the
/// socket in a sibling directory (`.ctl-<hex>/<file>`) before linking it onto
/// the final path, so the path the kernel actually sees during a bind is
/// longer than `socket` alone. Check the tighter (macOS) budget up front,
/// with a generous margin for that staging directory, so a too-deep
/// `$TMPDIR` fails right here with a message that names the exact cause --
/// an earlier task in this codebase lost time to this precise limit
/// surfacing as an opaque bind failure instead.
fn assert_socket_path_has_headroom(socket: &Path) {
    const SUN_PATH_LIMIT: usize = 104;
    const STAGING_OVERHEAD: usize = 24; // ".ctl-XXXXXXXX/" plus a safety margin
    let len = socket.as_os_str().len();
    assert!(
        len + STAGING_OVERHEAD < SUN_PATH_LIMIT,
        "control socket path `{}` is {len} bytes; that leaves no headroom \
         under the {SUN_PATH_LIMIT}-byte sockaddr_un.sun_path limit once \
         ControlServer's staging directory is accounted for. TMPDIR ({:?}) \
         is probably too deep for this suite to run here.",
        socket.display(),
        std::env::var_os("TMPDIR"),
    );
}

struct Server {
    // `Option` rather than a bare `Child`: `Server` implements `Drop` (to kill
    // a stray child even when a test panics), and safe Rust does not allow
    // moving a field out of a type that implements `Drop` -- so the two
    // signal-sending tests, which need to hand the child to
    // `wait`/`wait_with_output` by value, go through `into_child` below
    // rather than reaching into this field directly.
    child: Option<Child>,
    port: u16,
    socket: PathBuf,
    config_path: PathBuf,
    _dir: tempfile::TempDir,
}

impl Server {
    fn start(upstream_port: u16) -> Self {
        let dir = tempfile::tempdir().unwrap();
        // Kept short deliberately: see `assert_socket_path_has_headroom`.
        let socket = dir.path().join("d.sock");
        let templates = dir.path().join("templates");
        let config_path = dir.path().join("main.yaml");
        assert_socket_path_has_headroom(&socket);

        let port = free_port();
        std::fs::write(
            &config_path,
            config(port, upstream_port, &socket, &templates),
        )
        .unwrap();

        let mut child = Command::new(env!("CARGO_BIN_EXE_doppel"))
            .args(["serve", "--config"])
            .arg(&config_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();

        wait_until_ready(&mut child, port, &socket);

        Self {
            child: Some(child),
            port,
            socket,
            config_path,
            _dir: dir,
        }
    }

    fn get(&self, path: &str) -> (u16, String) {
        let url = format!("http://127.0.0.1:{}{path}", self.port);
        let response = reqwest::blocking::get(url).unwrap();
        let status = response.status().as_u16();
        (status, response.text().unwrap())
    }

    fn reload(&self) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_doppel"))
            .args(["config", "reload", "--socket"])
            .arg(&self.socket)
            .output()
            .unwrap()
    }

    fn pid(&self) -> u32 {
        self.child.as_ref().expect("child already taken").id()
    }

    /// Take ownership of the child process, for the one thing that needs it
    /// by value (`wait`/`wait_with_output`). `Drop` below treats an
    /// already-taken child as nothing left to clean up, so this is safe to
    /// call at any point before `server` goes out of scope.
    fn into_child(mut self) -> Child {
        self.child.take().expect("child already taken")
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        // A panicking test must never leave a stray process holding the
        // port or the socket file: `kill` plus `wait` reaps it unconditionally,
        // even if some earlier step in the test already failed. A `None`
        // here just means a signal test already took the child via
        // `into_child`, so there is nothing left to do.
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Poll for readiness against a deadline -- the right shape for "wait until a
/// condition holds" -- but also watch for the child exiting on its own.
/// Without that second check, a `doppel` that failed to bind (for instance
/// because `free_port`'s race was lost) would simply sit there "not ready"
/// for the entire timeout, indistinguishable from a slow machine. Checking
/// `try_wait` turns that into an immediate, specific failure that includes
/// the child's own stderr, which is exactly what would explain a lost race
/// or any other startup failure.
fn wait_until_ready(child: &mut Child, port: u16, socket: &Path) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = child.try_wait().expect("waiting on the child process") {
            let (stdout, stderr) = drain_output(child);
            panic!(
                "doppel exited early ({status}) while waiting to become ready on port \
                 {port}\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}"
            );
        }
        if TcpStream::connect(("127.0.0.1", port)).is_ok() && socket.exists() {
            return;
        }
        if Instant::now() >= deadline {
            // Kill and reap before reading the pipes to EOF, so `drain_output`
            // cannot block on a still-running process.
            let _ = child.kill();
            let _ = child.wait();
            let (stdout, stderr) = drain_output(child);
            panic!(
                "doppel did not become ready on port {port} (socket {}) within 10s\n\
                 --- stdout ---\n{stdout}\n--- stderr ---\n{stderr}",
                socket.display()
            );
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Read whatever remains of the child's stdout/stderr. Only ever called after
/// the child has already exited (or been killed and reaped) above, so both
/// pipes are guaranteed to be at EOF and this cannot block the test.
fn drain_output(child: &mut Child) -> (String, String) {
    let mut stdout = String::new();
    let mut stderr = String::new();
    if let Some(mut out) = child.stdout.take() {
        let _ = out.read_to_string(&mut stdout);
    }
    if let Some(mut err) = child.stderr.take() {
        let _ = err.read_to_string(&mut stderr);
    }
    (stdout, stderr)
}

/// Send `SIGTERM` to a running child.
///
/// The obvious way to do this is `libc::kill`, which the task brief for this
/// suite used directly -- but that requires an `unsafe` block, and this
/// workspace forbids `unsafe_code` everywhere, including this test target:
/// `doppel-cli`'s `Cargo.toml` inherits `[lints] workspace = true`, and Cargo
/// turns a workspace `forbid` into a command-line lint flag applied to every
/// target in the package, tests included. A command-line `forbid` cannot be
/// downgraded by a source-level `#![allow(unsafe_code)]` in this test file --
/// verified directly: rustc rejects that with E0453
/// ("allow(unsafe_code) incompatible with previous forbid"), because Cargo
/// has no mechanism to scope `[lints]` to a single target within a package.
/// So a "narrowly scoped exception in the test target" is not actually an
/// option Cargo offers here; the only ways to get `libc::kill` to compile
/// would be to either weaken the workspace-wide lint (rejected -- that is
/// exactly the "quietly weakening" this task warns against) or add a
/// target-specific carve-out that does not exist.
///
/// Sending a signal by pid does not actually require `libc` or `unsafe` at
/// all, though: the system `kill` utility does the exact same thing. Shelling
/// out to it sends the identical signal with zero exceptions to the
/// workspace's `unsafe_code = "forbid"`, which is a strictly better outcome
/// than any scoped exception would have been.
fn send_sigterm(pid: u32) {
    let status = Command::new("kill")
        .args(["-s", "TERM", &pid.to_string()])
        .status()
        .expect("the `kill` utility must be available on this platform");
    assert!(
        status.success(),
        "`kill -s TERM {pid}` itself failed: {status:?}"
    );
}

#[test]
fn proxies_a_request_end_to_end() {
    let up = upstream();
    let server = Server::start(up.port);
    let (status, body) = server.get("/hello");
    assert_eq!(status, 200);
    assert_eq!(body, "upstream saw /hello");
}

#[test]
fn reload_applies_a_changed_config() {
    let up = upstream();
    let server = Server::start(up.port);

    let text = std::fs::read_to_string(&server.config_path).unwrap();
    std::fs::write(
        &server.config_path,
        text.replace(
            "      type: default",
            "      type: default\n    loss:\n      percentage: 1.0\n      status: 503",
        ),
    )
    .unwrap();

    let output = server.reload();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("reloaded"));

    let (status, _) = server.get("/hello");
    assert_eq!(status, 503, "the new loss setting must be in effect");
}

#[test]
fn reload_of_an_invalid_config_is_rejected_and_traffic_keeps_flowing() {
    let up = upstream();
    let server = Server::start(up.port);

    let text = std::fs::read_to_string(&server.config_path).unwrap();
    std::fs::write(
        &server.config_path,
        text.replace("percentage", "percentaje"),
    )
    .unwrap();
    std::fs::write(
        &server.config_path,
        std::fs::read_to_string(&server.config_path)
            .unwrap()
            .replace("    resolve:", "    timeout: 0\n    resolve:"),
    )
    .unwrap();

    let output = server.reload();
    assert!(
        !output.status.success(),
        "an invalid config must fail the reload"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("CONFIG_INVALID"), "got: {stdout}");
    assert!(stdout.contains("proxies[0].timeout"), "got: {stdout}");

    let (status, body) = server.get("/still-here");
    assert_eq!(status, 200, "the previous config must still be serving");
    assert_eq!(body, "upstream saw /still-here");
}

#[test]
fn config_validate_exits_zero_on_a_good_config() {
    let up = upstream();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("main.yaml");
    std::fs::write(
        &path,
        config(
            free_port(),
            up.port,
            &dir.path().join("s.sock"),
            &dir.path().join("t"),
        ),
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_doppel"))
        .args(["config", "validate", "--config"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("configuration is valid"));
}

#[test]
fn config_validate_exits_one_and_lists_violations() {
    let up = upstream();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("main.yaml");
    let text = config(
        free_port(),
        up.port,
        &dir.path().join("s.sock"),
        &dir.path().join("t"),
    )
    .replace("    resolve:", "    timeout: 0\n    resolve:");
    std::fs::write(&path, text).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_doppel"))
        .args(["config", "validate", "--config"])
        .arg(&path)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stdout).contains("proxies[0].timeout"));
}

#[test]
fn an_environment_variable_supplies_the_config_path() {
    let up = upstream();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("from-env.yaml");
    std::fs::write(
        &path,
        config(
            free_port(),
            up.port,
            &dir.path().join("s.sock"),
            &dir.path().join("t"),
        ),
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_doppel"))
        .args(["config", "validate"])
        .env("DOPPEL_CONFIG_PATH", &path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn a_cli_flag_beats_the_environment_variable() {
    let up = upstream();
    let dir = tempfile::tempdir().unwrap();

    let good = dir.path().join("good.yaml");
    std::fs::write(
        &good,
        config(
            free_port(),
            up.port,
            &dir.path().join("s.sock"),
            &dir.path().join("t"),
        ),
    )
    .unwrap();

    let bad = dir.path().join("bad.yaml");
    std::fs::write(
        &bad,
        config(
            free_port(),
            up.port,
            &dir.path().join("s2.sock"),
            &dir.path().join("t2"),
        )
        .replace("    resolve:", "    timeout: 0\n    resolve:"),
    )
    .unwrap();

    // The environment points at the invalid config; the flag must win.
    let output = Command::new(env!("CARGO_BIN_EXE_doppel"))
        .args(["config", "validate", "--config"])
        .arg(&good)
        .env("DOPPEL_CONFIG_PATH", &bad)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "the flag must override DOPPEL_CONFIG_PATH, got: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn the_store_can_be_selected_by_environment_variable() {
    let output = Command::new(env!("CARGO_BIN_EXE_doppel"))
        .args(["serve"])
        .env("DOPPEL_CONFIG_STORE", "postgres")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn postgres_store_exits_two() {
    let output = Command::new(env!("CARGO_BIN_EXE_doppel"))
        .args(["serve", "--store", "postgres"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("not available in this build"));
}

#[test]
fn sigterm_drains_and_removes_the_socket() {
    let up = upstream();
    let server = Server::start(up.port);
    let socket = server.socket.clone();
    let pid = server.pid();

    send_sigterm(pid);

    let status = server.into_child().wait().unwrap();
    assert!(status.success(), "expected a clean exit, got {status:?}");
    assert!(
        !socket.exists(),
        "the control socket must be removed on shutdown"
    );
}

#[test]
fn admin_token_values_never_reach_the_logs() {
    let up = upstream();
    let server = Server::start(up.port);
    server.get("/anything");

    send_sigterm(server.pid());
    let output = server.into_child().wait_with_output().unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stdout.contains(SECRET_TOKEN),
        "an admin token leaked into stdout"
    );
    assert!(
        !stderr.contains(SECRET_TOKEN),
        "an admin token leaked into stderr"
    );
}

#[test]
fn admin_token_values_never_reach_the_logs_at_trace_level() {
    let up = upstream();
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("d.sock");
    let config_path = dir.path().join("main.yaml");
    assert_socket_path_has_headroom(&socket);
    let port = free_port();
    std::fs::write(
        &config_path,
        config(port, up.port, &socket, &dir.path().join("templates")),
    )
    .unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_doppel"))
        .args(["serve", "--config"])
        .arg(&config_path)
        .env("RUST_LOG", "trace")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    wait_until_ready(&mut child, port, &socket);

    send_sigterm(child.id());
    let output = child.wait_with_output().unwrap();
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !combined.is_empty(),
        "trace level should produce some output"
    );
    assert!(
        !combined.contains(SECRET_TOKEN),
        "an admin token leaked at trace level"
    );
}

#[test]
fn logs_are_json_and_carry_the_documented_fields() {
    let up = upstream();
    let server = Server::start(up.port);
    server.get("/logged");

    send_sigterm(server.pid());
    let output = server.into_child().wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);

    let line = stdout
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .find(|value| value["fields"]["message"] == "request proxied")
        .expect("expected a JSON log line for the proxied request");

    for field in [
        "request_id",
        "proxy",
        "method",
        "path",
        "status",
        "duration_ms",
        "upstream_contacted",
        "upstream_status",
        "upstream_duration_ms",
        "loss_injected",
        "latency_injected_ms",
    ] {
        assert!(
            !line["fields"][field].is_null(),
            "missing field `{field}` in {line}"
        );
    }
    assert_eq!(line["fields"]["path"], "/logged");
    assert_eq!(line["fields"]["proxy"], "p1");
}
