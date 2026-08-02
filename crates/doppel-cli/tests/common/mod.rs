//! Shared harness for the end-to-end test binaries under `tests/`.
//!
//! This module is compiled independently into each top-level test file that
//! declares `mod common;` -- that is how Rust integration tests share code,
//! since every file directly under `tests/` is its own binary crate. Each of
//! those binaries only exercises a subset of what lives here (`proxying.rs`
//! never touches `ChildGuard`, `shutdown.rs` never touches `config_validate`
//! helpers, and so on), so `-D warnings` would flag the untouched parts as
//! `dead_code` in every binary except the one that happens to use them all.
//! That is a property of how cargo links integration tests, not a sign that
//! anything here is actually unused, so the blanket allow below is the
//! correct fix -- scattering per-item allows would just hide the same
//! non-problem item by item, and deleting helpers to silence it would break
//! the binaries that do use them.
#![allow(dead_code)]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
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
/// Both the proxy port and the admin port are drawn this way, and both are
/// bound by the child, so a test now has two chances to lose the race per
/// server it starts.
///
/// The port is free when this returns and can be taken by anything before the
/// child binds it -- the listener is dropped here, which is the only way to
/// let the child have it. `Server::start_with` closes that race by retrying
/// with a fresh port when the child reports `Address already in use`, rather
/// than by pretending the window does not exist.
/// How many times `Server::start_with` will re-draw a port after losing the
/// race to bind it. Three independent losses in a row is not a race, it is
/// something else, and reporting that beats retrying forever.
const PORT_RACE_ATTEMPTS: usize = 3;

pub fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

pub struct Upstream {
    pub port: u16,
    _handle: std::thread::JoinHandle<()>,
}

/// A blocking upstream that answers every request with its path.
///
/// Unlike `free_port`, this has no discard-and-reuse step: the listener is
/// bound once, synchronously, right here, and the same listener is moved
/// into the background thread -- so there is no window in which another
/// process could steal the port between choosing it and using it.
pub fn upstream() -> Upstream {
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

/// The three ports a configuration has to agree on.
///
/// Passed as one value rather than as three `u16` arguments: the two the
/// child binds and the one it connects to are easy to transpose, and a
/// transposition produces a server that starts and never works.
#[derive(Debug, Clone, Copy)]
pub struct Ports {
    pub server: u16,
    pub admin: u16,
    pub upstream: u16,
}

pub fn config(ports: Ports, socket: &Path, templates: &Path) -> String {
    let Ports {
        server: server_port,
        admin: admin_port,
        upstream: upstream_port,
    } = ports;
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
  port: {admin_port}
  tokens:
    - name: user1
      group: admin
      token: {SECRET_TOKEN}
  access: {{}}
  upload:
    limit: 1Mi
proxies:
  - name: p1
    type: http
    url: "http://127.0.0.1:{upstream_port}/"
    resolve:
      type: default
"#,
        socket = socket.display(),
        templates = templates.display(),
    )
}

/// A recognizable token value, so a test can assert it never reaches stdout.
pub const SECRET_TOKEN: &str = "b6e1f0c2-secret-do-not-log";

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
pub fn assert_socket_path_has_headroom(socket: &Path) {
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

pub struct Server {
    // `Option` rather than a bare `Child`: `Server` implements `Drop` (to kill
    // a stray child even when a test panics), and safe Rust does not allow
    // moving a field out of a type that implements `Drop` -- so the two
    // signal-sending tests, which need to hand the child to
    // `wait`/`wait_with_output` by value, go through `into_child` below
    // rather than reaching into this field directly.
    child: Option<Child>,
    port: u16,
    admin_port: u16,
    pub socket: PathBuf,
    pub config_path: PathBuf,
    pub templates: PathBuf,
    _dir: tempfile::TempDir,
}

impl Server {
    pub fn start(upstream_port: u16) -> Self {
        Self::start_with(upstream_port, config)
    }

    /// Start a server from a caller-supplied configuration.
    ///
    /// The builder receives the four values only this harness knows -- the
    /// port it allocated, the upstream's port, the control socket path and the
    /// templates directory -- and returns the whole config document. It is
    /// `Fn` rather than `FnOnce` because a lost port race re-draws the port
    /// and asks for the document again. This
    /// exists so a suite can exercise a configuration this module has no
    /// business knowing about, such as one carrying mocks, without every other
    /// suite paying for those fields.
    pub fn start_with(
        upstream_port: u16,
        build_config: impl Fn(Ports, &Path, &Path) -> String,
    ) -> Self {
        let dir = tempfile::tempdir().unwrap();
        // Kept short deliberately: see `assert_socket_path_has_headroom`.
        let socket = dir.path().join("d.sock");
        let templates = dir.path().join("templates");
        let config_path = dir.path().join("main.yaml");
        assert_socket_path_has_headroom(&socket);

        // `free_port` can only report a port that was free a moment ago; the
        // child binds it later, and in a parallel test run something else can
        // take it in between. That is a property of the OS API, not a bug to
        // fix in place, so a lost race is retried with a fresh port. Anything
        // else that stops the child from starting is reported as-is -- a
        // retry loop that swallowed real failures would turn a broken binary
        // into a ten-second timeout.
        let mut last: Option<String> = None;
        for _ in 0..PORT_RACE_ATTEMPTS {
            let ports = Ports {
                server: free_port(),
                admin: free_port(),
                upstream: upstream_port,
            };
            let port = ports.server;
            std::fs::write(&config_path, build_config(ports, &socket, &templates)).unwrap();

            let mut child = Command::new(env!("CARGO_BIN_EXE_doppel"))
                .args(["serve", "--config"])
                .arg(&config_path)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();

            match try_wait_until_ready(&mut child, port, &socket) {
                Ok(()) => {
                    return Self {
                        child: Some(child),
                        port,
                        admin_port: ports.admin,
                        socket,
                        config_path,
                        templates,
                        _dir: dir,
                    };
                }
                Err(message) if message.contains("Address already in use") => {
                    last = Some(message);
                }
                Err(message) => panic!("{message}"),
            }
        }
        panic!(
            "lost the port race {PORT_RACE_ATTEMPTS} times in a row, which is not a race any \
             more; last failure:\n{}",
            last.unwrap_or_default()
        );
    }

    /// The port this server is listening on, for a suite that needs to build
    /// a request this harness does not model.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// The admin listener's port.
    pub fn admin_port(&self) -> u16 {
        self.admin_port
    }

    /// Place a template file at `<templates.dir>/<proxy>/<file>`.
    ///
    /// Safe to call after the server is up: templates are read per request,
    /// not at startup, precisely so a later phase can upload them at runtime.
    pub fn write_template(&self, proxy: &str, file: &str, contents: &str) {
        let dir = self.templates.join(proxy);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(file), contents).unwrap();
    }

    /// Send a request with a body and return the status and body of the reply.
    pub fn request(&self, method: &str, path: &str, body: &str) -> (u16, String) {
        let url = format!("http://127.0.0.1:{}{path}", self.port);
        let response = reqwest::blocking::Client::new()
            .request(method.parse().unwrap(), url)
            .body(body.to_owned())
            .send()
            .unwrap();
        let status = response.status().as_u16();
        (status, response.text().unwrap())
    }

    pub fn get(&self, path: &str) -> (u16, String) {
        let url = format!("http://127.0.0.1:{}{path}", self.port);
        let response = reqwest::blocking::get(url).unwrap();
        let status = response.status().as_u16();
        (status, response.text().unwrap())
    }

    /// A GET carrying one header, for the suites that resolve a proxy by
    /// header rather than by the default.
    pub fn get_with_header(&self, path: &str, name: &str, value: &str) -> (u16, String) {
        let url = format!("http://127.0.0.1:{}{path}", self.port);
        let response = reqwest::blocking::Client::new()
            .get(url)
            .header(name, value)
            .send()
            .unwrap();
        let status = response.status().as_u16();
        (status, response.text().unwrap())
    }

    pub fn reload(&self) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_doppel"))
            .args(["config", "reload", "--socket"])
            .arg(&self.socket)
            .output()
            .unwrap()
    }

    pub fn pid(&self) -> u32 {
        self.child.as_ref().expect("child already taken").id()
    }

    /// Take ownership of the child process, for the one thing that needs it
    /// by value (`wait`/`wait_with_output`). `Drop` below treats an
    /// already-taken child as nothing left to clean up, so this is safe to
    /// call at any point before `server` goes out of scope.
    pub fn into_child(mut self) -> Child {
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

/// The same guarantee `Server`'s `Drop` gives every other live-server test in
/// this file, for the one test (`admin_token_values_never_reach_the_logs_at_trace_level`)
/// that spawns its `doppel` process directly instead of going through
/// `Server::start`: without this, a panic between `spawn` and the post-signal
/// `wait` would leak the child process and leave it holding the port and the
/// control socket for the rest of the run.
pub struct ChildGuard(Option<Child>);

impl ChildGuard {
    pub fn new(child: Child) -> Self {
        Self(Some(child))
    }

    pub fn as_mut(&mut self) -> &mut Child {
        self.0.as_mut().expect("child already taken")
    }

    /// Take ownership of the child, for `wait_after_signal` below, which
    /// needs it by value. Mirrors `Server::into_child` for the same reason:
    /// safe Rust does not allow moving a field out of a type that
    /// implements `Drop`.
    pub fn into_child(mut self) -> Child {
        self.0.take().expect("child already taken")
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
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
pub fn wait_until_ready(child: &mut Child, port: u16, socket: &Path) {
    if let Err(message) = try_wait_until_ready(child, port, socket) {
        panic!("{message}");
    }
}

/// The same wait, reporting instead of panicking, so `Server::start_with` can
/// tell a lost port race from a real startup failure and retry only the
/// former.
pub fn try_wait_until_ready(child: &mut Child, port: u16, socket: &Path) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = child.try_wait().expect("waiting on the child process") {
            let (stdout, stderr) = drain_output(child);
            return Err(format!(
                "doppel exited early ({status}) while waiting to become ready on port \
                 {port}\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}"
            ));
        }
        if TcpStream::connect(("127.0.0.1", port)).is_ok() && socket.exists() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            // Kill and reap before reading the pipes to EOF, so `drain_output`
            // cannot block on a still-running process.
            let _ = child.kill();
            let _ = child.wait();
            let (stdout, stderr) = drain_output(child);
            return Err(format!(
                "doppel did not become ready on port {port} (socket {}) within 10s\n\
                 --- stdout ---\n{stdout}\n--- stderr ---\n{stderr}",
                socket.display()
            ));
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
pub fn send_sigterm(pid: u32) {
    let status = Command::new("kill")
        .args(["-s", "TERM", &pid.to_string()])
        .status()
        .expect("the `kill` utility must be available on this platform");
    assert!(
        status.success(),
        "`kill -s TERM {pid}` itself failed: {status:?}"
    );
}

/// The deadline every post-signal wait below uses.
///
/// `serve.rs`'s own shutdown path (`DRAIN_TIMEOUT` in
/// `crates/doppel-cli/src/commands/serve.rs`) bounds the entire shutdown
/// sequence at 30 seconds: that branch of its `tokio::select!` fires
/// unconditionally once a signal has been received, whether or not any
/// request was in flight at the time. None of the four tests that wait on a
/// child after `send_sigterm` actually has a request in flight at the
/// moment of the signal -- `Server::get` is synchronous and returns only
/// after the response is fully read, so by the time SIGTERM is sent the one
/// request each test issues has already completed. That means there is no
/// real distinction here between "a test that exercises a real drain" and
/// "a test where the server has nothing in flight": every one of the four
/// goes through the exact same `wait_for_signal -> drain` code path, bounded
/// by the exact same 30-second constant, regardless of traffic history. So
/// rather than inventing a second, shorter deadline that is not backed by
/// any actual difference in behaviour (and would just be a second arbitrary
/// number to tune later), all four tests share one deadline: the real
/// 30-second bound plus a flat margin for process teardown and scheduling
/// jitter on a loaded machine.
pub const SIGNAL_WAIT_DEADLINE: Duration = Duration::from_secs(35);

/// Wait for a child to exit after it has already been sent a signal, with a
/// deadline -- so a regressed signal handler (the exact failure class this
/// suite exists to catch) fails the test with a diagnostic instead of
/// hanging the run.
///
/// Both pipes are drained concurrently on their own threads for the entire
/// wait, not read afterwards: this is what makes the wait safe from
/// deadlock. If stdout or stderr filled its OS pipe buffer while nothing
/// was reading it, the child's own `write` would block, which would in turn
/// stop the child from ever reaching the point where it exits -- turning a
/// wait for exit into a wait that can never complete. Reading continuously
/// from the moment the child is handed in means neither buffer can fill
/// while this function is polling `try_wait`, so there is nothing for the
/// child to block on.
///
/// On the happy path (`try_wait` reports an exit before the deadline), the
/// exit status is returned to the caller so it can distinguish a clean exit
/// from a dirty one with its own assertion and message. On the timeout
/// path, the child is killed and reaped first -- which is also what lets
/// the reader threads observe EOF and `join` rather than blocking forever
/// on a child that is still running -- and this function panics itself,
/// with wording that names the timeout specifically ("did not exit within
/// ... of ...") so it can never be confused with the caller's own "exited
/// with status X" message for a dirty exit.
pub fn wait_after_signal(
    mut child: Child,
    reason: &str,
    deadline: Duration,
) -> (ExitStatus, String, String) {
    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();

    let stdout_handle = std::thread::spawn(move || {
        let mut buffer = String::new();
        if let Some(mut pipe) = stdout_pipe {
            let _ = pipe.read_to_string(&mut buffer);
        }
        buffer
    });
    let stderr_handle = std::thread::spawn(move || {
        let mut buffer = String::new();
        if let Some(mut pipe) = stderr_pipe {
            let _ = pipe.read_to_string(&mut buffer);
        }
        buffer
    });

    let started = Instant::now();
    let mut timed_out = false;
    let status = loop {
        if let Some(status) = child.try_wait().expect("waiting on the child process") {
            break status;
        }
        if started.elapsed() >= deadline {
            // Still running: kill and reap it so this function never
            // leaves a zombie behind. Killing it also closes its
            // stdout/stderr, which is what lets the reader threads below
            // reach EOF and `join` instead of blocking on a process that
            // will never produce more output.
            timed_out = true;
            let _ = child.kill();
            break child.wait().expect("reaping the killed child");
        }
        std::thread::sleep(Duration::from_millis(50));
    };

    let stdout = stdout_handle.join().unwrap_or_default();
    let stderr = stderr_handle.join().unwrap_or_default();

    assert!(
        !timed_out,
        "doppel did not exit within {deadline:?} of {reason} (killed after the deadline)\n\
         --- stdout so far ---\n{stdout}\n--- stderr so far ---\n{stderr}"
    );

    (status, stdout, stderr)
}
