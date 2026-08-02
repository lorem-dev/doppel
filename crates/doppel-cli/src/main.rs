//! The `doppel` binary.

mod cli;
mod commands;
mod control;

use std::process::ExitCode;

use clap::Parser;

use crate::cli::{Cli, Command, ConfigCommand, ServeArgs};

/// No `#[tokio::main]` here: `serve` sizes its runtime from `--workers`,
/// which the other subcommands have no use for, so each one builds the
/// runtime it needs -- see `run_serve` and `run_on_light_runtime` below.
///
/// Stream convention, followed by every command in this crate (`serve`,
/// `config validate`, `config reload`): a command's actual output -- a
/// violations list, `config reload`'s result, `config validate`'s
/// "configuration is valid" -- goes to stdout, because a script or a human
/// piping the output wants exactly that and nothing else. A failure that is
/// not itself the command's output -- being unable to open the configured
/// store, reach the control socket, or build the tokio runtime -- goes to
/// stderr instead, same as any other tool's diagnostics.
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
        Command::Config {
            command: ConfigCommand::Migrate(args),
        } => run_on_light_runtime(async move {
            match commands::migrate::migrate(&args).await {
                Ok(report) => {
                    println!("{report}");
                    0
                }
                Err(err) => {
                    eprintln!("{err}");
                    err.exit_code()
                }
            }
        }),
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
///
/// Validation runs synchronously, here, before `build_runtime`: every
/// semantic rule -- including V3, `server.workers` must be at least 1 --
/// has to be checked before anything acts on the value it governs.
/// `build_runtime` passes `workers` straight into
/// `tokio::runtime::Builder::worker_threads`, which panics on `0` rather
/// than returning an error, so a config that reached `build_runtime`
/// unvalidated would take the process down with exit code 101 instead of
/// failing cleanly with the documented "config rejected" code 1.
/// `doppel_core::validate::validate` is a plain synchronous function over
/// an already-parsed `Config`, so it runs here on the same plain thread
/// that opened the store, before any tokio runtime exists. `serve` below
/// validates again once it is handed the config; that second check is
/// harmless (the config already passed) and keeps `serve` correct on its
/// own if it is ever called from anywhere else.
fn run_serve(args: ServeArgs) -> u8 {
    // The runtime is built first, before anything touches the store. That
    // ordering is forced by the PostgreSQL store: an `sqlx` pool is bound to
    // the runtime that created it, so the store cannot be opened on one
    // runtime and used on another, and it cannot be opened with no runtime at
    // all. `--workers` rather than a configuration field is what makes the
    // ordering possible -- see `ServeArgs::workers`.
    let runtime = match build_runtime(args.workers) {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("cannot build the tokio runtime: {err}");
            return 1;
        }
    };

    runtime.block_on(async move {
        let (store, config) = match args.store.open().await {
            Ok(opened) => opened,
            Err(err) => {
                eprintln!("{err}");
                return err.exit_code();
            }
        };

        if let Err(violations) = doppel_core::validate::validate(&config) {
            // The violations list is the command's actual output, like
            // `config validate`'s -- not a failure to open or reach anything
            // -- so it goes to stdout, per the stream convention above.
            for violation in &violations {
                println!("{violation}");
            }
            return 1;
        }

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
/// `--workers` when the operator set it; leaving the call out (the `None`
/// branch) keeps tokio's own default, `available_parallelism`, exactly as the
/// specification documents.
fn build_runtime(
    workers: Option<std::num::NonZeroUsize>,
) -> std::io::Result<tokio::runtime::Runtime> {
    let mut builder = tokio::runtime::Builder::new_multi_thread();
    if let Some(workers) = workers {
        builder.worker_threads(workers.get());
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
        let runtime = build_runtime(std::num::NonZeroUsize::new(1)).unwrap();
        assert_eq!(runtime.handle().metrics().num_workers(), 1);

        let runtime = build_runtime(std::num::NonZeroUsize::new(3)).unwrap();
        assert_eq!(runtime.handle().metrics().num_workers(), 3);
    }

    #[test]
    fn an_absent_worker_count_leaves_tokios_own_default() {
        let runtime = build_runtime(None).unwrap();
        let expected = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
        assert_eq!(runtime.handle().metrics().num_workers(), expected);
    }
}
