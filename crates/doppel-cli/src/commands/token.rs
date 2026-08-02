//! `doppel token add`.

use std::path::PathBuf;

use crate::cli::{CliError, TokenAddArgs};
use crate::control::{ControlRequest, ControlResponse, client, token};

/// Ask a running server to issue a token, and print it once.
///
/// Stream convention (see `main.rs`): the token and the outcome are this
/// command's output, so they go to stdout. Failing to reach the control
/// socket at all is not that output, so it goes to stderr.
///
/// The token is printed alone on its own line, with no label and nothing
/// after it, so `doppel token add --name ci | tail -1` is a usable way to
/// capture it without a parser.
pub async fn add(args: &TokenAddArgs) -> u8 {
    let socket = match resolve_socket(args).await {
        Ok(socket) => socket,
        Err(err) => {
            eprintln!("{err}");
            return err.exit_code();
        }
    };

    let request = ControlRequest::TokenAdd {
        name: args.name.clone(),
        group: args.group.clone().unwrap_or_else(token::default_group),
    };

    match client::send(&socket, request).await {
        Ok(ControlResponse::TokenAdded {
            name,
            group,
            token,
            revision,
        }) => {
            println!("token `{name}` issued to group `{group}`, revision {revision}");
            println!("it is in force now, and this is the only time it is shown:");
            println!("{}", token.as_str());
            0
        }
        Ok(ControlResponse::Error { code, errors }) => {
            println!("token not issued: {}", code.as_str());
            for violation in errors {
                println!("{violation}");
            }
            1
        }
        // The server answered a `token_add` with a reload result. Reported
        // rather than ignored: it means the two ends disagree about the
        // protocol, and silently exiting 0 would suggest a token exists.
        Ok(ControlResponse::Ok { .. }) => {
            eprintln!("the server answered a token request with a reload response");
            1
        }
        Err(err) => {
            eprintln!("{err}");
            1
        }
    }
}

/// The same resolution `config reload` uses: an explicit `--socket` wins and
/// needs no store, otherwise the path comes from the configuration, read
/// through whichever store holds it.
async fn resolve_socket(args: &TokenAddArgs) -> Result<PathBuf, CliError> {
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
    use doppel_core::config::Name;

    fn args(socket: Option<PathBuf>, store: StoreKind) -> TokenAddArgs {
        TokenAddArgs {
            name: Name::parse("ci").unwrap(),
            group: None,
            socket,
            store: StoreArgs {
                store,
                config: "./main.yaml".into(),
                config_name: "default".to_owned(),
                database_url: None,
            },
        }
    }

    #[test]
    fn the_default_group_is_user_and_not_admin() {
        // The whole reason the default is spelled out rather than left to a
        // clap `default_value`: it is a security decision, and it belongs
        // next to the reasoning for it.
        assert_eq!(token::default_group().as_str(), "user");
    }

    #[tokio::test]
    async fn an_explicit_socket_bypasses_the_store_even_when_it_is_postgres() {
        let dir = tempfile::tempdir().unwrap();
        let args = args(
            Some(dir.path().join("does-not-exist.sock")),
            StoreKind::Postgres,
        );
        assert_eq!(add(&args).await, 1);
    }

    #[tokio::test]
    async fn no_socket_and_a_postgres_store_with_no_url_is_refused() {
        assert_eq!(add(&args(None, StoreKind::Postgres)).await, 1);
    }
}
