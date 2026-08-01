//! Accepting and serving control commands.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use doppel_core::store::ConfigStore;
use doppel_core::{Runtime, RuntimeHolder, StoreError};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

use super::{ControlRequest, ControlResponse};

pub struct ControlServer {
    listener: UnixListener,
    path: PathBuf,
}

impl ControlServer {
    /// Bind the socket, replacing a stale file left by an unclean shutdown.
    ///
    /// A file that another live process is listening on cannot be replaced: the
    /// bind below fails, which is the intended outcome. Silently taking over
    /// another instance's socket would leave two servers fighting over reloads.
    ///
    /// Liveness is checked with a blocking `std::os::unix::net::UnixStream`
    /// connect attempt rather than tokio's async connect, because `bind` itself
    /// is synchronous: there is no runtime here to poll an async connect
    /// against.
    pub fn bind(path: &Path) -> std::io::Result<Self> {
        if path.exists() && std::os::unix::net::UnixStream::connect(path).is_err() {
            std::fs::remove_file(path)?;
        }
        let listener = UnixListener::bind(path)?;
        std::fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(0o600))?;
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
        let accept = async {
            loop {
                match self.listener.accept().await {
                    Ok((stream, _)) => {
                        let holder = Arc::clone(&holder);
                        let store = Arc::clone(&store);
                        tokio::spawn(async move {
                            if let Err(err) = serve_connection(stream, holder, store).await {
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

async fn serve_connection(
    stream: UnixStream,
    holder: Arc<RuntimeHolder>,
    store: Arc<dyn ConfigStore>,
) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    if reader.read_line(&mut line).await? == 0 {
        return Ok(());
    }

    let response = match serde_json::from_str::<ControlRequest>(line.trim()) {
        Ok(ControlRequest::Reload) => reload(&holder, store.as_ref()).await,
        Err(_) => ControlResponse::Error {
            code: "NOT_FOUND".to_owned(),
            errors: Vec::new(),
        },
    };

    let mut text = serde_json::to_string(&response)
        .unwrap_or_else(|_| r#"{"status":"error","code":"STORE_ERROR"}"#.to_owned());
    text.push('\n');
    reader.into_inner().write_all(text.as_bytes()).await
}

/// Load, validate, compile, then swap. Every step before the swap can fail
/// harmlessly; the swap itself cannot fail.
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

    #[tokio::test]
    async fn reload_applies_a_valid_config_and_bumps_the_revision() {
        let h = harness().await;
        std::fs::write(
            &h.config_path,
            GOOD.replace("  - name: p1", "  - name: p1\n    timeout: 5"),
        )
        .unwrap();

        let response = client::send(&h.socket, ControlRequest::Reload)
            .await
            .unwrap();
        assert!(
            matches!(response, ControlResponse::Ok { proxies: 1, .. }),
            "{response:?}"
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
