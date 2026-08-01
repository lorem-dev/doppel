//! `doppel config reload`.

use std::path::PathBuf;

use crate::cli::ReloadArgs;
use crate::control::{ControlRequest, ControlResponse, client};

/// Send a reload command and return the process exit code.
pub async fn reload(args: &ReloadArgs) -> u8 {
    let socket = match resolve_socket(args) {
        Ok(socket) => socket,
        Err(message) => {
            println!("{message}");
            return 1;
        }
    };

    match client::send(&socket, ControlRequest::Reload).await {
        Ok(ControlResponse::Ok { revision, proxies }) => {
            println!("reloaded: revision {revision}, {proxies} proxies");
            0
        }
        Ok(ControlResponse::Error { code, errors }) => {
            println!("reload rejected: {code}");
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

/// An explicit `--socket` wins; otherwise read it from the configuration, which
/// is where the running server got it from.
fn resolve_socket(args: &ReloadArgs) -> Result<PathBuf, String> {
    if let Some(socket) = &args.socket {
        return Ok(socket.clone());
    }
    doppel_core::config::load_from_path(&args.store.config)
        .map(|config| config.control.socket)
        .map_err(|err| format!("cannot determine the control socket: {err}"))
}
