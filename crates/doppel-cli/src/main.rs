//! The `doppel` binary.

mod cli;
mod commands;
mod control;

use std::process::ExitCode;

use clap::Parser;

use crate::cli::{Cli, Command, ConfigCommand};

#[tokio::main]
async fn main() -> ExitCode {
    let code = match Cli::parse().command {
        Command::Version => {
            println!("doppel {}", env!("CARGO_PKG_VERSION"));
            0
        }
        Command::Config {
            command: ConfigCommand::Validate(args),
        } => commands::validate::print(&commands::validate::validate(&args).await),
        Command::Config {
            command: ConfigCommand::Reload(args),
        } => commands::reload::reload(&args).await,
        Command::Serve(args) => match commands::serve::serve(&args).await {
            Ok(()) => 0,
            Err(err) => {
                eprintln!("{err}");
                err.exit_code()
            }
        },
    };
    ExitCode::from(code)
}
