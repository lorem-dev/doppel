//! `doppel config reload`.

use std::path::PathBuf;

use crate::cli::{CliError, ReloadArgs};
use crate::control::{ControlRequest, ControlResponse, client};

/// Send a reload command and return the process exit code.
///
/// Stream convention (see `main.rs`): the reload result -- success or
/// rejected, with its violations -- is this command's actual output, so it
/// goes to stdout. Failing to even reach the control socket is not that
/// output, so it goes to stderr.
pub async fn reload(args: &ReloadArgs) -> u8 {
    let socket = match resolve_socket(args).await {
        Ok(socket) => socket,
        Err(err) => {
            eprintln!("{err}");
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
            eprintln!("{err}");
            1
        }
    }
}

/// An explicit `--socket` wins and needs no store at all: nothing about
/// finding it depends on how the configuration is stored.
///
/// Otherwise the socket path comes from the configuration, which is where the
/// running server got it from -- so it is read through the store, whichever
/// store that is. One path for both, rather than a file-shaped shortcut beside
/// a database-shaped one, which is how the two would come to disagree about
/// which configuration they mean.
async fn resolve_socket(args: &ReloadArgs) -> Result<PathBuf, CliError> {
    if let Some(socket) = &args.socket {
        return Ok(socket.clone());
    }
    args.store
        .open()
        .await
        .map(|(_, config)| config.control.socket)
        .map_err(|err| CliError::Failed(format!("cannot determine the control socket: {err}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{StoreArgs, StoreKind};

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
    async fn no_socket_and_a_postgres_store_with_no_url_is_refused() {
        // Without `--socket` the path has to come from the configuration, and
        // reading that needs a store that can be opened. A postgres store with
        // nothing to connect to cannot be, and the refusal has to name why
        // rather than time out looking for a socket that was never resolved.
        let args = ReloadArgs {
            socket: None,
            store: store_args(StoreKind::Postgres),
        };
        assert_eq!(reload(&args).await, 1);
    }
}
