//! Accepting and serving control commands.

use std::future::Future;
use std::os::unix::fs::{DirBuilderExt, FileTypeExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use doppel_core::store::ConfigStore;
use doppel_core::{Runtime, RuntimeHolder, StoreError};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, Take};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::Mutex;

use super::{ControlRequest, ControlResponse};

/// A control command line longer than this without a terminating newline
/// cannot be a well-formed request. Capping the read keeps a client that
/// never sends `\n` from growing the buffer without bound; 64 KiB is far
/// more than any real command needs.
const MAX_REQUEST_LINE_BYTES: u64 = 64 * 1024;

#[derive(Debug)]
pub struct ControlServer {
    listener: UnixListener,
    path: PathBuf,
}

impl ControlServer {
    /// Bind the socket at `path`, replacing a stale file left by an unclean
    /// shutdown but refusing to take over a live one, and never leaving two
    /// servers silently sharing (or fighting over) the same control channel.
    ///
    /// The socket is bound inside a private directory created with mode
    /// 0700 at creation time (`DirBuilder::mode`, so there is no window in
    /// which the directory itself is more permissive than that) and only
    /// then published at `path` with `std::fs::hard_link`, which is atomic
    /// and exclusive: it fails with `AlreadyExists` if `path` is already
    /// occupied instead of silently overwriting whatever is there. That is
    /// the property this design cannot give up. An earlier version of this
    /// function published with `std::fs::rename` instead, which is atomic
    /// but *not* exclusive: had a second `bind` published its own socket at
    /// `path` in the window between this function's liveness check and its
    /// publish step, a `rename`-based publish would have silently clobbered
    /// it -- no error to either side, and the loser's listener left open
    /// but unreachable. `hard_link` turns that same race into a loud,
    /// unambiguous failure (`AlreadyExists`) for whichever `bind` loses it,
    /// which is what a check-then-act race is allowed to do; staying silent
    /// is not. See `hard_linking_a_bound_socket_works_and_refuses_to_overwrite`
    /// in this module's tests for the platform verification this relies on:
    /// hard-linking a bound Unix domain socket is not guaranteed by POSIX to
    /// work on every filesystem, only observed to work here.
    ///
    /// Whether `path` may be claimed is checked as late as possible --
    /// immediately before the publish -- to keep the window between the
    /// check and the publish as small as reasonably achievable. That window
    /// is not zero: a concurrent `bind` racing this one inside it is a
    /// check-then-act race inherent to this approach. What changed from the
    /// pre-hard-link design is not the size of that window but what happens
    /// inside it: landing in it now means this `bind` (or the racing one,
    /// whichever loses) fails loudly with `AlreadyExists`, never a silent
    /// takeover in either direction.
    pub fn bind(path: &Path) -> std::io::Result<Self> {
        let temp_dir = temp_socket_dir(path);
        // Clean up a leftover from a previous failed attempt at this exact
        // temporary name; astronomically unlikely (it is derived from the
        // process id and a nanosecond timestamp) but cheap to guard against.
        let _ = std::fs::remove_dir_all(&temp_dir);

        std::fs::DirBuilder::new().mode(0o700).create(&temp_dir)?;
        let temp = temp_dir.join("ctl.sock");

        let listener = match UnixListener::bind(&temp) {
            Ok(listener) => listener,
            Err(err) => {
                let _ = std::fs::remove_dir_all(&temp_dir);
                return Err(err);
            }
        };

        if let Err(err) =
            std::fs::set_permissions(&temp, std::os::unix::fs::PermissionsExt::from_mode(0o600))
        {
            let _ = std::fs::remove_dir_all(&temp_dir);
            return Err(err);
        }

        if let Err(err) = clear_for_bind(path) {
            let _ = std::fs::remove_dir_all(&temp_dir);
            return Err(err);
        }

        if let Err(err) = std::fs::hard_link(&temp, path) {
            let _ = std::fs::remove_dir_all(&temp_dir);
            return Err(err);
        }

        let _ = std::fs::remove_dir_all(&temp_dir);

        Ok(Self {
            listener,
            path: path.to_path_buf(),
        })
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Serve until `shutdown` resolves, then remove the socket file.
    pub async fn run(
        self,
        holder: Arc<RuntimeHolder>,
        store: Arc<dyn ConfigStore>,
        shutdown: impl Future<Output = ()> + Send,
    ) {
        tracing::info!(path = %self.path().display(), "control channel listening");

        // Serializes the reload critical section (load, validate, compile,
        // swap, and the response write) across connections, so the
        // revision reported to a client is the revision actually installed
        // at the moment it is told, never one a concurrent reload on
        // another connection has not applied yet.
        let reload_lock = Arc::new(Mutex::new(()));

        let accept = async {
            loop {
                match self.listener.accept().await {
                    Ok((stream, _)) => {
                        let holder = Arc::clone(&holder);
                        let store = Arc::clone(&store);
                        let reload_lock = Arc::clone(&reload_lock);
                        tokio::spawn(async move {
                            if let Err(err) =
                                serve_connection(stream, holder, store, reload_lock).await
                            {
                                tracing::warn!(error = %err, "control connection failed");
                            }
                        });
                    }
                    Err(err) => {
                        tracing::warn!(error = %err, "control accept failed");
                    }
                }
            }
        };

        tokio::select! {
            () = accept => {}
            () = shutdown => {}
        }

        if let Err(err) = std::fs::remove_file(&self.path)
            && err.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(error = %err, path = %self.path.display(), "cannot remove control socket");
        }
    }
}

/// A sibling *directory* of `path` (same parent, so later linking onto
/// `path` stays on one filesystem) with a name derived from the process id
/// and a timestamp, unique enough that two `bind` calls never collide on it
/// in practice. The directory, not just the socket file inside it, is what
/// is made private: see `bind`'s doc comment.
///
/// Kept deliberately short: `sockaddr_un.sun_path` has a small, fixed
/// capacity (on the order of 100 bytes), this directory sits between
/// `path`'s parent and the socket file inside it, and every byte here comes
/// out of that shared budget. A verbose name (the process id and a
/// nanosecond timestamp spelled out in decimal) was tried first and pushed a
/// path already close to that limit over it in this crate's own tests,
/// which run under a deeply nested system temporary directory.
fn temp_socket_dir(path: &Path) -> PathBuf {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let unique = std::process::id() ^ (nanos as u32);
    parent.join(format!(".ctl-{unique:x}"))
}

/// Decide whether `path` may be claimed by a bind that is about to publish a
/// new socket onto it.
///
/// - Nothing there: fine, nothing to do.
/// - A file that is not a Unix socket at all (a stray regular file left by,
///   say, a fat-fingered redirect, or a directory): nothing socket-shaped
///   could be listening on it, so `remove_file` is attempted. For a plain
///   file that succeeds and clears the way. For anything `remove_file`
///   itself refuses to remove (a directory, for instance, which needs
///   `remove_dir`/`remove_dir_all` instead), that call's own error is
///   returned and `bind` fails -- a safe outcome, just not the "always
///   safe to remove" this comment used to claim; the code does not attempt
///   anything beyond a plain `remove_file`.
/// - A socket that accepts a connection: something is listening. Left
///   alone, and reported as an error -- this is what stops a bind from
///   silently taking over another instance's control channel.
/// - A socket that refuses connections: the process that owned it is gone,
///   an unclean shutdown left it behind, and removing it is safe.
/// - Any other connect error (permission denied, and the like): this cannot
///   tell a live socket from a dead one, so it fails closed rather than
///   guess. A stale socket left by a crash always refuses connections, so
///   nothing genuine is lost by refusing an ambiguous result instead of
///   assuming it is safe.
fn clear_for_bind(path: &Path) -> std::io::Result<()> {
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err),
    };

    if !metadata.file_type().is_socket() {
        std::fs::remove_file(path)?;
        return Ok(());
    }

    match std::os::unix::net::UnixStream::connect(path) {
        Ok(_) => Err(std::io::Error::new(
            std::io::ErrorKind::AddrInUse,
            format!(
                "another process is listening on {}; refusing to take over its control socket",
                path.display()
            ),
        )),
        Err(err) if err.kind() == std::io::ErrorKind::ConnectionRefused => {
            std::fs::remove_file(path)?;
            Ok(())
        }
        Err(err) => Err(std::io::Error::new(
            err.kind(),
            format!(
                "cannot tell whether {} is a live control socket ({err}); refusing to bind over it",
                path.display()
            ),
        )),
    }
}

async fn serve_connection(
    stream: UnixStream,
    holder: Arc<RuntimeHolder>,
    store: Arc<dyn ConfigStore>,
    reload_lock: Arc<Mutex<()>>,
) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.take(MAX_REQUEST_LINE_BYTES));
    let mut line = String::new();
    if reader.read_line(&mut line).await? == 0 {
        return Ok(());
    }

    match serde_json::from_str::<ControlRequest>(line.trim()) {
        Ok(ControlRequest::Reload) => {
            // Held through the response write in `write_response` below, so
            // the whole "load, validate, compile, swap, respond" sequence
            // for one reload finishes before the next one starts.
            let _guard = reload_lock.lock().await;
            let response = reload(&holder, store.as_ref()).await;
            write_response(reader, &response).await
        }
        Err(_) => {
            let response = ControlResponse::Error {
                code: "NOT_FOUND".to_owned(),
                errors: Vec::new(),
            };
            write_response(reader, &response).await
        }
    }
}

async fn write_response(
    reader: BufReader<Take<UnixStream>>,
    response: &ControlResponse,
) -> std::io::Result<()> {
    let mut text = serde_json::to_string(response)
        .unwrap_or_else(|_| r#"{"status":"error","code":"STORE_ERROR"}"#.to_owned());
    text.push('\n');
    reader
        .into_inner()
        .into_inner()
        .write_all(text.as_bytes())
        .await
}

/// Load, validate, compile, then swap. Every step before the swap can fail
/// harmlessly; the swap itself cannot fail. Callers reload concurrently only
/// while holding `reload_lock` (see `run`), so this function does not need
/// one of its own.
async fn reload(holder: &RuntimeHolder, store: &dyn ConfigStore) -> ControlResponse {
    let (config, revision) = match store.load().await {
        Ok(loaded) => loaded,
        Err(StoreError::Invalid(errors)) => {
            return ControlResponse::Error {
                code: "CONFIG_INVALID".to_owned(),
                errors,
            };
        }
        Err(err) => {
            return ControlResponse::Error {
                code: "STORE_ERROR".to_owned(),
                errors: vec![doppel_core::Violation::new("", err.to_string())],
            };
        }
    };

    // The revision comes from the stored config's content, so a reload that
    // changes nothing reports the same number it did before.
    match Runtime::compile(Arc::new(config), revision) {
        Ok(runtime) => {
            let proxies = runtime.proxies.len();
            holder.store(runtime);
            tracing::info!(revision = revision.0, proxies, "config reloaded");
            ControlResponse::Ok {
                revision: revision.0,
                proxies,
            }
        }
        Err(err) => ControlResponse::Error {
            code: err.code.as_str().to_owned(),
            errors: vec![doppel_core::Violation::new("", err.message)],
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::{ControlRequest, ControlResponse, client};
    use doppel_core::store::{ConfigStore, FileStore};
    use doppel_core::{Runtime, RuntimeHolder};
    use std::sync::Arc;

    const GOOD: &str = r#"
server:
  host: "127.0.0.1"
  port: 8080
admin:
  host: "127.0.0.1"
  port: 8081
  tokens: []
  access: {}
  upload:
    limit: 1M
proxies:
  - name: p1
    type: http
    url: "https://example.com/"
"#;

    /// Poll a real connection attempt instead of sleeping a guessed delay.
    /// A stray probe connection is harmless: with nothing written to it, the
    /// server's `read_line` sees EOF and `serve_connection` returns.
    async fn wait_until_accepting(socket: &std::path::Path) {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            if tokio::net::UnixStream::connect(socket).await.is_ok() {
                return;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "control server did not start accepting connections in time"
            );
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    }

    struct Harness {
        _dir: tempfile::TempDir,
        socket: std::path::PathBuf,
        config_path: std::path::PathBuf,
        holder: Arc<RuntimeHolder>,
        shutdown: tokio::sync::oneshot::Sender<()>,
    }

    async fn harness() -> Harness {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("main.yaml");
        std::fs::write(&config_path, GOOD).unwrap();
        let socket = dir.path().join("doppel.sock");

        let store: Arc<dyn ConfigStore> = Arc::new(FileStore::new(
            config_path.clone(),
            dir.path().join("templates"),
        ));
        let (config, revision) = store.load().await.unwrap();
        let holder = Arc::new(RuntimeHolder::new(
            Runtime::compile(Arc::new(config), revision).unwrap(),
        ));

        let server = ControlServer::bind(&socket).unwrap();
        let (tx, rx) = tokio::sync::oneshot::channel();
        let run_holder = Arc::clone(&holder);
        tokio::spawn(async move {
            server
                .run(run_holder, store, async {
                    let _ = rx.await;
                })
                .await;
        });
        // The socket file exists as soon as `bind` returns, but the spawned
        // task's accept loop starts on its own schedule; wait for a real
        // connection to succeed instead of guessing how long that takes.
        wait_until_accepting(&socket).await;

        Harness {
            _dir: dir,
            socket,
            config_path,
            holder,
            shutdown: tx,
        }
    }

    /// A store whose `load` counts each call and then blocks on `gate`
    /// until the test releases it, so a test can force two `Reload`
    /// connections' critical sections to overlap deterministically: no
    /// timing guesses, no change to production code, both connections
    /// driven through ordinary `client::send` calls.
    ///
    /// `ConfigStore` is defined with `#[async_trait::async_trait]` in
    /// `doppel-core`, which is what makes `Arc<dyn ConfigStore>` -- used
    /// throughout this module -- possible: a plain `async fn` in a trait
    /// cannot be called through a trait object. `doppel-cli` does not (and,
    /// per this task's constraints, must not) depend on the `async-trait`
    /// crate itself, so this impl is written by hand in the desugared form
    /// `async_trait` would otherwise generate -- a plain method returning a
    /// boxed, pinned future -- rather than pulling in the macro.
    struct GatedStore {
        inner: FileStore,
        enter_count: Arc<std::sync::atomic::AtomicUsize>,
        gate: Arc<tokio::sync::Notify>,
    }

    // `async_trait`'s expansion ties every reference parameter's lifetime to
    // a common trailing lifetime via `where 'pN: 'out` (making them all
    // early-bound), rather than the late-bound lifetimes plain elision
    // (`'_`) would produce; an impl using `'_` fails to unify with the
    // trait's real signature even though the visible types match. The
    // explicit lifetime parameters and bounds below exist to reproduce that
    // structure by hand, not for their names, which do not need to match
    // the macro's own choice of names -- only the early-bound shape does.
    impl ConfigStore for GatedStore {
        fn load<'s, 'out>(
            &'s self,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = Result<
                            (doppel_core::Config, doppel_core::Revision),
                            doppel_core::StoreError,
                        >,
                    > + Send
                    + 'out,
            >,
        >
        where
            's: 'out,
            Self: 'out,
        {
            Box::pin(async move {
                self.enter_count
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                self.gate.notified().await;
                self.inner.load().await
            })
        }

        fn save<'s, 'p1, 'p2, 'out>(
            &'s self,
            _config: &'p1 doppel_core::Config,
            _expected: Option<doppel_core::Revision>,
            _actor: Option<&'p2 str>,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = Result<doppel_core::Revision, doppel_core::StoreError>,
                    > + Send
                    + 'out,
            >,
        >
        where
            's: 'out,
            'p1: 'out,
            'p2: 'out,
            Self: 'out,
        {
            Box::pin(async { unimplemented!("not exercised by this test") })
        }

        fn load_templates<'s, 'p1, 'out>(
            &'s self,
            _proxy: &'p1 str,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = Result<
                            Vec<doppel_core::store::TemplateFile>,
                            doppel_core::StoreError,
                        >,
                    > + Send
                    + 'out,
            >,
        >
        where
            's: 'out,
            'p1: 'out,
            Self: 'out,
        {
            Box::pin(async { unimplemented!("not exercised by this test") })
        }

        fn save_template<'s, 'p1, 'p2, 'p3, 'out>(
            &'s self,
            _proxy: &'p1 str,
            _file: &'p2 str,
            _bytes: &'p3 [u8],
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<Output = Result<(), doppel_core::StoreError>> + Send + 'out,
            >,
        >
        where
            's: 'out,
            'p1: 'out,
            'p2: 'out,
            'p3: 'out,
            Self: 'out,
        {
            Box::pin(async { unimplemented!("not exercised by this test") })
        }

        fn delete_template<'s, 'p1, 'p2, 'out>(
            &'s self,
            _proxy: &'p1 str,
            _file: &'p2 str,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<Output = Result<bool, doppel_core::StoreError>>
                    + Send
                    + 'out,
            >,
        >
        where
            's: 'out,
            'p1: 'out,
            'p2: 'out,
            Self: 'out,
        {
            Box::pin(async { unimplemented!("not exercised by this test") })
        }

        fn retain_templates<'s, 'p1, 'p2, 'out>(
            &'s self,
            _proxy: &'p1 str,
            _keep: &'p2 [String],
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<Output = Result<(), doppel_core::StoreError>> + Send + 'out,
            >,
        >
        where
            's: 'out,
            'p1: 'out,
            'p2: 'out,
            Self: 'out,
        {
            Box::pin(async { unimplemented!("not exercised by this test") })
        }
    }

    /// Poll for `counter` to reach `expected` instead of sleeping a guessed
    /// delay, with a deadline so a real failure still reports promptly.
    async fn wait_for_count(counter: &std::sync::atomic::AtomicUsize, expected: usize) {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
        while counter.load(std::sync::atomic::Ordering::SeqCst) < expected {
            assert!(
                tokio::time::Instant::now() < deadline,
                "timed out waiting for load() to be entered {expected} time(s)"
            );
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    }

    #[tokio::test]
    async fn a_reload_does_not_start_loading_until_the_previous_ones_response_is_sent() {
        // Exercises exactly the property `reload_lock` exists for: the
        // second connection's `store.load()` must not be entered until the
        // first connection's whole critical section -- load, validate,
        // compile, swap, and the response write -- has finished, not just
        // until the swap itself has happened.
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("main.yaml");
        std::fs::write(&config_path, GOOD).unwrap();
        let socket = dir.path().join("doppel.sock");

        let inner = FileStore::new(config_path.clone(), dir.path().join("templates"));
        let (config, revision) = inner.load().await.unwrap();
        let holder = Arc::new(RuntimeHolder::new(
            Runtime::compile(Arc::new(config), revision).unwrap(),
        ));

        let enter_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let gate = Arc::new(tokio::sync::Notify::new());
        let store: Arc<dyn ConfigStore> = Arc::new(GatedStore {
            inner,
            enter_count: Arc::clone(&enter_count),
            gate: Arc::clone(&gate),
        });

        let server = ControlServer::bind(&socket).unwrap();
        let (tx, rx) = tokio::sync::oneshot::channel();
        let run_holder = Arc::clone(&holder);
        tokio::spawn(async move {
            server
                .run(run_holder, store, async {
                    let _ = rx.await;
                })
                .await;
        });
        wait_until_accepting(&socket).await;

        let socket_a = socket.clone();
        let first =
            tokio::spawn(async move { client::send(&socket_a, ControlRequest::Reload).await });
        wait_for_count(&enter_count, 1).await;

        let socket_b = socket.clone();
        let second =
            tokio::spawn(async move { client::send(&socket_b, ControlRequest::Reload).await });

        // There is no event to poll for an absence, only time to give the
        // scheduler a fair chance to prove the bug if it exists: in the
        // correct implementation the second connection is genuinely
        // blocked on `reload_lock`, so `enter_count` staying at 1 holds no
        // matter how long this waits, rather than racing a clock. If
        // serialization were broken, the second connection would reach its
        // own `load()` (and bump the count) within microseconds of being
        // scheduled, well inside this window.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(
            enter_count.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "the second reload's load() must not start before the first reload's response is sent"
        );

        gate.notify_one();
        let first_response = first.await.unwrap().unwrap();
        assert!(
            matches!(first_response, ControlResponse::Ok { .. }),
            "{first_response:?}"
        );

        wait_for_count(&enter_count, 2).await;
        gate.notify_one();
        let second_response = second.await.unwrap().unwrap();
        assert!(
            matches!(second_response, ControlResponse::Ok { .. }),
            "{second_response:?}"
        );

        let _ = tx.send(());
    }

    #[tokio::test]
    async fn reload_applies_a_valid_config_and_reports_the_new_revision() {
        let h = harness().await;
        let before = h.holder.load().revision;
        std::fs::write(
            &h.config_path,
            GOOD.replace("  - name: p1", "  - name: p1\n    timeout: 5"),
        )
        .unwrap();

        let response = client::send(&h.socket, ControlRequest::Reload)
            .await
            .unwrap();
        let ControlResponse::Ok { revision, proxies } = response else {
            panic!("expected an ok response, got {response:?}");
        };
        assert_eq!(proxies, 1);
        assert_ne!(
            revision, before.0,
            "the config changed, so its content-derived revision must too"
        );
        assert_eq!(
            h.holder.load().revision.0,
            revision,
            "the reported revision must be the one actually installed"
        );
        assert_eq!(
            h.holder.load().proxies[0].timeout,
            std::time::Duration::from_secs(5)
        );
        let _ = h.shutdown.send(());
    }

    #[tokio::test]
    async fn reload_rejects_an_invalid_config_and_keeps_serving_the_old_one() {
        let h = harness().await;
        let before = h.holder.load().revision;
        std::fs::write(&h.config_path, GOOD.replace("port: 8081", "port: 8080")).unwrap();

        let response = client::send(&h.socket, ControlRequest::Reload)
            .await
            .unwrap();
        let ControlResponse::Error { code, errors } = response else {
            panic!("expected an error response");
        };
        assert_eq!(code, "CONFIG_INVALID");
        assert!(errors.iter().any(|v| v.path == "admin.port"));
        assert_eq!(h.holder.load().revision, before, "runtime must not change");
        let _ = h.shutdown.send(());
    }

    #[tokio::test]
    async fn an_unknown_command_is_reported_as_not_found() {
        let h = harness().await;
        let raw = client::send_raw(&h.socket, "{\"command\":\"explode\"}")
            .await
            .unwrap();
        assert!(raw.contains("NOT_FOUND"), "got: {raw}");
        let _ = h.shutdown.send(());
    }

    #[tokio::test]
    async fn a_stale_socket_file_is_replaced_at_bind_time() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("stale.sock");
        std::fs::write(&socket, b"not a socket").unwrap();
        assert!(ControlServer::bind(&socket).is_ok());
    }

    #[test]
    fn hard_linking_a_bound_socket_works_and_refuses_to_overwrite() {
        // `bind`'s publish step depends on two properties of
        // `std::fs::hard_link` applied to a bound Unix domain socket, and
        // neither is guaranteed by POSIX for every filesystem, so both are
        // checked here rather than assumed:
        //
        // 1. Linking a bound socket to a second path preserves the
        //    listener: a client connecting through the link reaches it,
        //    just as if it had been bound there directly.
        // 2. Linking onto a path that is already occupied fails loudly
        //    (`AlreadyExists`) instead of silently replacing whatever is
        //    there -- the property `rename` lacked, which is why `bind`
        //    no longer uses it.
        let dir = tempfile::tempdir().unwrap();
        let temp = dir.path().join("new.sock");
        let published = dir.path().join("pub.sock");
        let listener = std::os::unix::net::UnixListener::bind(&temp).unwrap();

        std::fs::hard_link(&temp, &published).unwrap();
        std::os::unix::net::UnixStream::connect(&published)
            .expect("a connection through the hard link must reach the original listener");
        drop(listener);

        let temp2 = dir.path().join("new2.sock");
        let _listener2 = std::os::unix::net::UnixListener::bind(&temp2).unwrap();
        let err = std::fs::hard_link(&temp2, &published).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists, "got {err:?}");
    }

    #[tokio::test]
    async fn a_live_socket_is_not_taken_over() {
        // This is the single property that stops two servers from fighting
        // over reloads: a live listener at `path` must block `bind` outright
        // rather than being silently replaced.
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("live.sock");
        let _listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();

        let err = ControlServer::bind(&socket).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::AddrInUse, "got {err:?}");
    }

    #[tokio::test]
    async fn a_socket_that_cannot_be_probed_is_left_alone() {
        use std::os::unix::fs::PermissionsExt;

        // Simulates a connect failure that is neither "accepted" nor
        // "refused" by denying permission on the socket file itself. This
        // assumes the test process is not running as root: root bypasses
        // file permission checks, so the connect would simply succeed and
        // this test would not exercise the ambiguous-error path at all.
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("locked.sock");
        {
            // Bind and drop: the file is a real socket, but nothing is
            // listening on it by the time `bind` probes it, same as a stale
            // socket left by a crash -- the only difference is the mode.
            let _listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        }
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o000)).unwrap();

        let err = ControlServer::bind(&socket).unwrap_err();
        assert_eq!(
            err.kind(),
            std::io::ErrorKind::PermissionDenied,
            "got {err:?}"
        );
    }

    #[tokio::test]
    async fn the_socket_is_removed_on_shutdown() {
        let h = harness().await;
        assert!(h.socket.exists());
        let _ = h.shutdown.send(());

        // The server task notices the shutdown signal and removes the file
        // on its own schedule; poll for that instead of sleeping a guessed
        // delay, so this only ever waits as long as the real cleanup takes.
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
        while h.socket.exists() {
            assert!(
                tokio::time::Instant::now() < deadline,
                "shutdown must clean up the socket"
            );
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    }

    #[tokio::test]
    async fn the_socket_is_not_world_accessible() {
        use std::os::unix::fs::PermissionsExt;
        let h = harness().await;
        let mode = std::fs::metadata(&h.socket).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "got {mode:o}");
        let _ = h.shutdown.send(());
    }

    #[tokio::test]
    async fn the_client_reports_a_missing_socket_clearly() {
        let dir = tempfile::tempdir().unwrap();
        let err = client::send(&dir.path().join("absent.sock"), ControlRequest::Reload)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("is doppel running"), "got: {err}");
    }
}
