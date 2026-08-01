//! `doppel config reload`.

use std::path::PathBuf;

use crate::cli::{CliError, ReloadArgs, StoreKind, mask_dsn};
use crate::control::{ControlRequest, ControlResponse, client};

/// Send a reload command and return the process exit code.
pub async fn reload(args: &ReloadArgs) -> u8 {
    let socket = match resolve_socket(args) {
        Ok(socket) => socket,
        Err(err) => {
            println!("{err}");
            return err.exit_code();
        }
    };

    match client::send(&socket, ControlRequest::Reload).await {
        Ok(ControlResponse::Ok {
            revision,
            proxies,
            unapplied,
        }) => {
            println!("reloaded: revision {revision}, {proxies} proxies");
            if !unapplied.is_empty() {
                println!(
                    "note: these sections changed but only take effect after a restart: {}",
                    unapplied.join(", ")
                );
            }
            0
        }
        Ok(ControlResponse::Error { code, errors }) => {
            println!("reload rejected: {}", code.as_str());
            for violation in errors {
                println!("{violation}");
            }
            1
        }
        Err(err) => {
            println!("{err}");
            1
        }
    }
}

/// An explicit `--socket` wins and needs no store at all: nothing about
/// finding it depends on how the configuration is stored. Otherwise the
/// socket path has to come from the configuration, which is where the
/// running server got it from, and reading that configuration needs a
/// working store -- so with no `--socket` and `--store postgres`, this is
/// refused the same way `StoreArgs::open()` refuses it everywhere else,
/// rather than only when a command actually calls `open()`.
fn resolve_socket(args: &ReloadArgs) -> Result<PathBuf, CliError> {
    if let Some(socket) = &args.socket {
        return Ok(socket.clone());
    }
    match args.store.store {
        StoreKind::Postgres => Err(CliError::StoreUnavailable {
            dsn: args.store.database_url.as_deref().map(mask_dsn),
        }),
        StoreKind::File => doppel_core::config::load_from_path(&args.store.config)
            .map(|config| config.control.socket)
            .map_err(|err| CliError::Failed(format!("cannot determine the control socket: {err}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::StoreArgs;

    fn store_args(store: StoreKind) -> StoreArgs {
        StoreArgs {
            store,
            config: "./main.yaml".into(),
            config_name: "default".to_owned(),
            database_url: None,
        }
    }

    #[tokio::test]
    async fn an_explicit_socket_bypasses_the_store_even_when_it_is_postgres() {
        // The store is never consulted when `--socket` is given, so a
        // missing socket file is a connection failure (exit 1) rather than
        // the store refusal (exit 2) that `--store postgres` would
        // otherwise produce.
        let dir = tempfile::tempdir().unwrap();
        let args = ReloadArgs {
            socket: Some(dir.path().join("does-not-exist.sock")),
            store: store_args(StoreKind::Postgres),
        };
        assert_eq!(reload(&args).await, 1);
    }

    #[tokio::test]
    async fn no_socket_and_a_postgres_store_is_refused_with_exit_code_2() {
        let args = ReloadArgs {
            socket: None,
            store: store_args(StoreKind::Postgres),
        };
        assert_eq!(reload(&args).await, 2);
    }
}
