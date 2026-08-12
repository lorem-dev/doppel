//! `doppel serve`.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use doppel_admin::AdminState;
use doppel_core::store::ConfigStore;
use doppel_core::{Config, Revision, Runtime, RuntimeHolder};
use doppel_proxy::{ProxyState, serve as serve_proxy};
use tokio::sync::Mutex;

use crate::cli::CliError;
use crate::control::ControlServer;

/// How long in-flight requests get to finish after a shutdown signal.
const DRAIN_TIMEOUT: Duration = Duration::from_secs(30);

/// `store` and `config` are the ones `StoreArgs::open()` already produced
/// from its single parse of the configuration file -- `main`'s entry point
/// opens the store synchronously, before this crate's tokio runtime is even
/// built, because sizing that runtime from `config.server.workers` requires
/// the config to exist first. Accepting them as parameters rather than
/// re-deriving them here (via a second `open()`/`store.load()` pair) is what
/// keeps that one parse the only parse: the worker count the runtime was
/// already sized with and the config compiled into the `Runtime` below are
/// guaranteed to come from the exact same read of the file.
pub async fn serve(store: Arc<dyn ConfigStore>, config: Config) -> Result<(), CliError> {
    doppel_core::validate::validate(&config).map_err(|violations| {
        let text = violations
            .iter()
            .map(std::string::ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        CliError::Failed(format!("configuration is invalid:\n{text}"))
    })?;

    doppel_telemetry::init_logging(config.logging.level, config.logging.format)
        .map_err(|err| CliError::Failed(err.to_string()))?;

    // After logging, so that what this reports -- including the warning a
    // build without the `sentry` feature emits for a configured DSN -- is
    // visible. The tracing layer resolves the Sentry hub per event, so
    // installing it before the client exists loses nothing.
    //
    // Bound to a name, not `_`: the guard flushes and stops reporting when
    // dropped, and `let _ = ...` would drop it here.
    let _sentry = doppel_telemetry::sentry::init(config.sentry.as_ref())
        .map_err(|err| CliError::Failed(err.to_string()))?;

    // Checked before anything binds, and a bad value fails startup. A
    // malformed token variable that was logged and skipped would leave an
    // operator believing they had provisioned access, and finding out
    // otherwise at the moment they needed it.
    let env_tokens = Arc::new(
        doppel_core::config::EnvTokens::from_env()
            .map_err(|err| CliError::Failed(err.to_string()))?,
    );
    if !env_tokens.is_empty() {
        // Names only. The count and the names are operational facts worth a
        // line; the values are not.
        tracing::info!(
            tokens = ?env_tokens.names().map(doppel_core::config::Name::as_str).collect::<Vec<_>>(),
            "admin tokens supplied by the environment"
        );
        for shadowed in config
            .admin
            .tokens
            .iter()
            .filter(|token| env_tokens.shadows(&token.name))
        {
            tracing::warn!(
                name = shadowed.name.as_str(),
                "a configured admin token is shadowed by one from the environment; \
                 the configured value will not authenticate"
            );
        }
    }

    // Where clients reach this Doppel, for rewriting an upstream's redirects.
    // The variable wins over the document, and a malformed one fails startup for
    // the reason a malformed token variable does: an operator who set it and got
    // no error believes redirects name that address.
    let external_url = match doppel_core::config::external_url_from_env()
        .map_err(|err| CliError::Failed(err.to_string()))?
    {
        Some(from_env) => {
            tracing::info!(url = from_env.as_str(), "external url from the environment");
            Some(from_env)
        }
        None => {
            let derived = config.server.public_url();
            if let Some(url) = &derived {
                // Worth a line, because it is a guess: behind a port mapping or
                // an ingress the client used neither this address nor this port,
                // and this is where an operator sees which address their
                // rewritten redirects will name.
                tracing::info!(
                    url = url.as_str(),
                    "external url from server.host and server.port"
                );
            }
            derived
        }
    };

    // After logging is up, so these reach wherever the operator is looking,
    // and only here: `doppel config validate` runs in CI loops, and a warning
    // repeated on every run is a warning nobody reads. Startup is also the
    // only moment at which the ports these concern are about to be bound.
    for note in doppel_core::validate::startup_advisories(&config) {
        tracing::warn!("{note}");
    }

    preflight(&config).map_err(CliError::Failed)?;

    // Only when something can read them. `/metrics` on the admin listener is
    // the sole reader, so with the listener off a recorder would accumulate
    // series for nobody. The `metrics` crate falls back to a no-op recorder
    // when none is installed, so the pipeline's recording calls stay correct
    // and cost nothing -- "admin off" then means off rather than off-ish.
    //
    // Installed before the first request either way, so no traffic goes
    // unrecorded, and before the admin listener exists, since `/metrics`
    // renders from the handle this returns.
    //
    // One decision, read once, for both the recorder and the listener below.
    // They were two separate reads of the same field, coupled only by a `zip`
    // that silently dropped the listener if the other said no -- so removing
    // one guard would have left a bound port with nothing serving it, and a
    // mutation flipping one guard was undone by the other.
    let admin_enabled = config.admin.enable;

    let metrics = if admin_enabled {
        Some(
            doppel_core::metrics::install()
                .map_err(|err| CliError::Failed(format!("cannot install metrics: {err}")))?,
        )
    } else {
        None
    };
    let started_at = std::time::Instant::now();

    let addr = SocketAddr::new(config.server.host, config.server.port.get());
    let admin_addr = SocketAddr::new(config.admin.host, config.admin.port.get());
    let socket_path = config.control.socket.clone();
    let revision = Revision::of_config(&config);
    let config = Arc::new(config);

    let runtime = Runtime::compile(Arc::clone(&config), revision)
        .map_err(|err| CliError::Failed(err.message))?;
    let holder = Arc::new(RuntimeHolder::new(runtime));

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|err| CliError::Failed(format!("cannot bind {addr}: {err}")))?;
    // Bound before anything is served, and a failure here fails startup.
    // Serving proxy traffic with no admin listener would be a process that
    // looks healthy and cannot be administered, and the operator would find
    // out at the moment they needed it. Unless they asked for exactly that,
    // in which case the port is never touched -- a disabled listener must not
    // hold a port, or "disabled" would still collide with whatever else wants
    // it.
    let admin_listener = if admin_enabled {
        Some(
            tokio::net::TcpListener::bind(admin_addr)
                .await
                .map_err(|err| {
                    CliError::Failed(format!("cannot bind admin port {admin_addr}: {err}"))
                })?,
        )
    } else {
        None
    };
    let control = ControlServer::bind(&socket_path).map_err(|err| {
        CliError::Failed(format!(
            "cannot bind control socket {}: {err}",
            socket_path.display()
        ))
    })?;

    // One mutex, shared. The control socket and the admin API both reload the
    // same runtime, and two reloads that interleave can swap in the wrong
    // order and leave the process running the older configuration. Handing
    // each its own would compile and read as correct.
    let reload_lock = Arc::new(Mutex::new(()));

    // The admin address is reported as absent rather than omitted when the
    // listener is off: a missing field reads as "nothing to say", and the
    // operator needs to see that the choice was made and honoured.
    tracing::info!(
        %addr,
        admin = ?admin_enabled.then_some(admin_addr),
        control_socket = %socket_path.display(),
        proxies = config.proxies.len(),
        "doppel started"
    );

    let (shutdown_proxy, proxy_rx) = tokio::sync::oneshot::channel();
    let (shutdown_admin, admin_rx) = tokio::sync::oneshot::channel();
    let (shutdown_control, control_rx) = tokio::sync::oneshot::channel();

    // `zip` would express the same thing, and would also quietly discard a
    // bound listener if the two ever disagreed. They cannot disagree now, and
    // this says so.
    let admin_task = admin_listener.map(|listener| {
        let metrics = metrics.expect("the recorder is installed whenever the listener is");
        let state = AdminState::new(
            Arc::clone(&store),
            Arc::clone(&holder),
            Arc::clone(&config),
            Arc::clone(&env_tokens),
            Arc::clone(&reload_lock),
            metrics,
            started_at,
        );
        tokio::spawn(async move {
            let result = doppel_admin::serve(state, listener, async {
                let _ = admin_rx.await;
            })
            .await;
            if let Err(err) = result {
                tracing::error!(error = %err, "admin listener stopped");
            }
        })
    });

    let control_task = tokio::spawn({
        let holder = Arc::clone(&holder);
        let store = Arc::clone(&store);
        let startup_config = Arc::clone(&config);
        let reload_lock = Arc::clone(&reload_lock);
        async move {
            control
                .run(
                    holder,
                    store,
                    startup_config,
                    env_tokens,
                    reload_lock,
                    async {
                        let _ = control_rx.await;
                    },
                )
                .await;
        }
    });

    let proxy_task = tokio::spawn(serve_proxy(
        ProxyState::new(Arc::clone(&holder))
            .with_external_url(external_url.map(doppel_core::config::ExternalUrl::into_url)),
        listener,
        async {
            let _ = proxy_rx.await;
        },
    ));

    wait_for_signal().await;
    tracing::info!("shutdown signal received, draining");
    let _ = shutdown_proxy.send(());
    let _ = shutdown_admin.send(());
    let _ = shutdown_control.send(());

    // A second signal, or an overrunning drain, must not hang the process.
    let drain = async {
        let _ = proxy_task.await;
        if let Some(admin_task) = admin_task {
            let _ = admin_task.await;
        }
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
