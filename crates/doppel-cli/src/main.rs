//! The `doppel` binary.

mod cli;
mod commands;
mod control;

use std::process::ExitCode;

use clap::Parser;

use crate::cli::{Cli, Command, ConfigCommand, ServeArgs};

/// No `#[tokio::main]` here: `serve` is the one subcommand whose runtime
/// needs sizing from the configuration (`server.workers`), and that requires
/// the config to exist before the runtime that will serve it does. So the
/// config is opened synchronously, on the plain thread `main` starts on,
/// before any tokio runtime is built -- see `run_serve` below.
fn main() -> ExitCode {
    ExitCode::from(run(Cli::parse().command))
}

fn run(command: Command) -> u8 {
    match command {
        Command::Version => {
            println!("doppel {}", env!("CARGO_PKG_VERSION"));
            0
        }
        Command::Config {
            command: ConfigCommand::Validate(args),
        } => run_on_light_runtime(async move {
            commands::validate::print(&commands::validate::validate(&args).await)
        }),
        Command::Config {
            command: ConfigCommand::Reload(args),
        } => run_on_light_runtime(async move { commands::reload::reload(&args).await }),
        Command::Serve(args) => run_serve(args),
    }
}

/// Open the store (a single, synchronous parse of the configuration file --
/// see `StoreArgs::open`), size a multi-threaded runtime from
/// `config.server.workers` when it is set, and run `serve` on it. Doing the
/// open before the runtime exists is what lets `server.workers` reach the
/// runtime that is about to serve traffic, rather than being read and
/// silently discarded while tokio picked its own default
/// (`available_parallelism`) regardless of what the operator configured.
fn run_serve(args: ServeArgs) -> u8 {
    let (store, config) = match args.store.open() {
        Ok(opened) => opened,
        Err(err) => {
            eprintln!("{err}");
            return err.exit_code();
        }
    };

    let runtime = match build_runtime(config.server.workers) {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("cannot build the tokio runtime: {err}");
            return 1;
        }
    };

    runtime.block_on(async move {
        match commands::serve::serve(store, config).await {
            Ok(()) => 0,
            Err(err) => {
                eprintln!("{err}");
                err.exit_code()
            }
        }
    })
}

/// `version`, `config validate` and `config reload` each do one bounded
/// piece of work and exit; none of them benefit from a worker pool, so a
/// single-threaded runtime is enough and avoids spinning up threads that
/// would never be used.
fn run_on_light_runtime<F: std::future::Future<Output = u8>>(future: F) -> u8 {
    match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime.block_on(future),
        Err(err) => {
            eprintln!("cannot build the tokio runtime: {err}");
            1
        }
    }
}

/// Build the runtime `serve` runs its listener on. `worker_threads` mirrors
/// `config.server.workers` when the operator set it; leaving the call out
/// (the `None` branch) keeps tokio's own default, `available_parallelism`,
/// exactly as the spec documents.
fn build_runtime(workers: Option<usize>) -> std::io::Result<tokio::runtime::Runtime> {
    let mut builder = tokio::runtime::Builder::new_multi_thread();
    if let Some(workers) = workers {
        builder.worker_threads(workers);
    }
    builder.enable_all().build()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The property `config.server.workers` exists for: an explicit count
    /// must reach the runtime that actually serves traffic, not just get
    /// validated and then ignored while tokio picks its own default.
    #[test]
    fn an_explicit_worker_count_sizes_the_runtime() {
        let runtime = build_runtime(Some(1)).unwrap();
        assert_eq!(runtime.handle().metrics().num_workers(), 1);

        let runtime = build_runtime(Some(3)).unwrap();
        assert_eq!(runtime.handle().metrics().num_workers(), 3);
    }

    #[test]
    fn an_absent_worker_count_leaves_tokios_own_default() {
        let runtime = build_runtime(None).unwrap();
        let expected = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
        assert_eq!(runtime.handle().metrics().num_workers(), expected);
    }
}
