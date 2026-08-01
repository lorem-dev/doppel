//! `doppel serve`.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use doppel_core::store::ConfigStore;
use doppel_core::{Runtime, RuntimeHolder};
use doppel_proxy::{ProxyState, serve as serve_proxy};

use crate::cli::{CliError, ServeArgs};
use crate::control::ControlServer;

/// How long in-flight requests get to finish after a shutdown signal.
const DRAIN_TIMEOUT: Duration = Duration::from_secs(30);

pub async fn serve(args: &ServeArgs) -> Result<(), CliError> {
    let store: Arc<dyn ConfigStore> = args.store.open()?;
    let (config, revision) = store
        .load()
        .await
        .map_err(|err| CliError::Failed(err.to_string()))?;

    doppel_telemetry::init_logging(config.logging.level, config.logging.format)
        .map_err(|err| CliError::Failed(err.to_string()))?;

    preflight(&config).map_err(CliError::Failed)?;

    let addr = SocketAddr::new(config.server.host, config.server.port);
    let socket_path = config.control.socket.clone();
    let config = Arc::new(config);

    let runtime = Runtime::compile(Arc::clone(&config), revision)
        .map_err(|err| CliError::Failed(err.message))?;
    let holder = Arc::new(RuntimeHolder::new(runtime));

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|err| CliError::Failed(format!("cannot bind {addr}: {err}")))?;
    let control = ControlServer::bind(&socket_path).map_err(|err| {
        CliError::Failed(format!(
            "cannot bind control socket {}: {err}",
            socket_path.display()
        ))
    })?;

    tracing::info!(
        %addr,
        control_socket = %socket_path.display(),
        proxies = config.proxies.len(),
        "doppel started"
    );

    let (shutdown_proxy, proxy_rx) = tokio::sync::oneshot::channel();
    let (shutdown_control, control_rx) = tokio::sync::oneshot::channel();

    let control_task = tokio::spawn({
        let holder = Arc::clone(&holder);
        let store = Arc::clone(&store);
        async move {
            control
                .run(holder, store, async {
                    let _ = control_rx.await;
                })
                .await;
        }
    });

    let proxy_task = tokio::spawn(serve_proxy(
        ProxyState::new(Arc::clone(&holder)),
        listener,
        async {
            let _ = proxy_rx.await;
        },
    ));

    wait_for_signal().await;
    tracing::info!("shutdown signal received, draining");
    let _ = shutdown_proxy.send(());
    let _ = shutdown_control.send(());

    // A second signal, or an overrunning drain, must not hang the process.
    let drain = async {
        let _ = proxy_task.await;
        let _ = control_task.await;
    };
    tokio::select! {
        () = drain => tracing::info!("drained cleanly"),
        () = tokio::time::sleep(DRAIN_TIMEOUT) => tracing::warn!("drain timed out"),
        () = wait_for_signal() => tracing::warn!("second signal, exiting now"),
    }

    Ok(())
}

/// Checks that depend on the machine rather than the config. Deliberately not
/// part of validation: `config validate` must behave identically everywhere.
fn preflight(config: &doppel_core::Config) -> Result<(), String> {
    std::fs::create_dir_all(&config.templates.dir).map_err(|err| {
        format!(
            "cannot create templates directory {}: {err}",
            config.templates.dir.display()
        )
    })?;

    let parent = config
        .control
        .socket
        .parent()
        .unwrap_or(std::path::Path::new("."));
    if !parent.is_dir() {
        return Err(format!(
            "control socket directory {} does not exist",
            parent.display()
        ));
    }
    Ok(())
}

async fn wait_for_signal() {
    let mut term = match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
        Ok(term) => term,
        Err(err) => {
            tracing::warn!(error = %err, "cannot listen for SIGTERM");
            let _ = tokio::signal::ctrl_c().await;
            return;
        }
    };
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = term.recv() => {}
    }
}
