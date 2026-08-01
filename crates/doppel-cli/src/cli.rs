//! Command line surface. The full flag set is defined now, including the
//! PostgreSQL options, so that phase 4 adds behaviour rather than syntax.

use std::path::PathBuf;
use std::sync::Arc;

use clap::{Args, Parser, Subcommand, ValueEnum};
use doppel_core::Config;
use doppel_core::store::{ConfigStore, FileStore};

#[derive(Debug, Parser)]
#[command(name = "doppel", version, about = "A doppelganger for your backend")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Run the proxy.
    Serve(ServeArgs),
    /// Inspect or reload the configuration.
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// Print the version.
    Version,
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// Validate the configuration and report every violation.
    Validate(StoreArgs),
    /// Ask a running server to reload its configuration.
    Reload(ReloadArgs),
}

#[derive(Debug, Args)]
pub struct ServeArgs {
    #[command(flatten)]
    pub store: StoreArgs,
}

#[derive(Debug, Args)]
pub struct ReloadArgs {
    /// Control socket path. Defaults to the value in the configuration.
    #[arg(long)]
    pub socket: Option<PathBuf>,
    #[command(flatten)]
    pub store: StoreArgs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "lowercase")]
pub enum StoreKind {
    File,
    Postgres,
}

/// Where the configuration lives. Connection settings come from here and from
/// the environment only -- never from the configuration document, which would
/// be circular.
#[derive(Args)]
pub struct StoreArgs {
    #[arg(long, value_enum, env = "DOPPEL_CONFIG_STORE", default_value = "file")]
    pub store: StoreKind,

    #[arg(long, env = "DOPPEL_CONFIG_PATH", default_value = "./main.yaml")]
    pub config: PathBuf,

    #[arg(long, env = "DOPPEL_CONFIG_NAME", default_value = "default")]
    pub config_name: String,

    #[arg(long, env = "DOPPEL_DATABASE_URL")]
    pub database_url: Option<String>,
}

/// A hand-written `Debug` impl, rather than `#[derive(Debug)]`, because the
/// derived one would format `database_url` verbatim: anyone who ever `{:?}`s
/// a `StoreArgs` (in a log line, a panic message, anywhere) must not be able
/// to make the password come out in the clear.
impl std::fmt::Debug for StoreArgs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StoreArgs")
            .field("store", &self.store)
            .field("config", &self.config)
            .field("config_name", &self.config_name)
            .field("database_url", &self.database_url.as_deref().map(mask_dsn))
            .finish()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CliError {
    /// `dsn` is `None` when no `--database-url` was given, and already masked
    /// otherwise. This text reaches stderr on every path that prints a
    /// `CliError` -- `serve`'s error path, and `config validate`'s and
    /// `config reload`'s printed messages -- none of which can be relied on
    /// to have logging initialised, so the DSN is masked here rather than at
    /// a call site that might not run.
    #[error("the postgres store is not available in this build{}", dsn_suffix(dsn))]
    StoreUnavailable { dsn: Option<String> },
    #[error("{0}")]
    Failed(String),
}

fn dsn_suffix(dsn: &Option<String>) -> String {
    match dsn {
        Some(dsn) => format!(" (database url: {dsn})"),
        None => String::new(),
    }
}

impl CliError {
    #[must_use]
    pub fn exit_code(&self) -> u8 {
        match self {
            Self::StoreUnavailable { .. } => 2,
            Self::Failed(_) => 1,
        }
    }
}

impl StoreArgs {
    /// Build the store and return the configuration this single parse
    /// produced, so a caller that also needs a value out of the config (the
    /// worker thread count `serve` sizes its runtime with, for instance)
    /// reads the same bytes the store was built from rather than parsing the
    /// file a second time. Two separate parses used to be how the store's
    /// `templates.dir` could end up disagreeing with the config the server
    /// actually ran: the first parse (here) silently fell back to a default
    /// on any error, and a second parse later on (`store.load()`) was the one
    /// that actually surfaced a bad config. Now there is exactly one parse,
    /// and its error -- a missing file, bad YAML, anything -- is reported
    /// directly rather than swallowed.
    pub fn open(&self) -> Result<(Arc<dyn ConfigStore>, Config), CliError> {
        match self.store {
            StoreKind::Postgres => Err(CliError::StoreUnavailable {
                dsn: self.database_url.as_deref().map(mask_dsn),
            }),
            StoreKind::File => {
                let config = doppel_core::config::load_from_path(&self.config)
                    .map_err(|err| CliError::Failed(err.to_string()))?;
                let store = FileStore::new(self.config.clone(), config.templates.dir.clone());
                Ok((Arc::new(store), config))
            }
        }
    }
}

/// Hide the password before a DSN reaches a log line or an error message. An
/// unparseable value is replaced wholesale rather than echoed, since it might
/// still contain a secret.
#[must_use]
pub fn mask_dsn(dsn: &str) -> String {
    match reqwest::Url::parse(dsn) {
        Ok(mut url) => {
            if url.password().is_some() {
                let _ = url.set_password(Some("***"));
            }
            url.to_string()
        }
        Err(_) => "<unparseable dsn>".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn serve_defaults_to_the_file_store_and_main_yaml() {
        let cli = Cli::parse_from(["doppel", "serve"]);
        let Command::Serve(args) = cli.command else {
            panic!("expected serve")
        };
        assert_eq!(args.store.store, StoreKind::File);
        assert_eq!(args.store.config, std::path::PathBuf::from("./main.yaml"));
        assert_eq!(args.store.config_name, "default");
    }

    #[test]
    fn config_validate_and_reload_parse() {
        let cli = Cli::parse_from(["doppel", "config", "validate"]);
        assert!(matches!(
            cli.command,
            Command::Config {
                command: ConfigCommand::Validate(_)
            }
        ));

        let cli = Cli::parse_from(["doppel", "config", "reload", "--socket", "/tmp/x.sock"]);
        let Command::Config {
            command: ConfigCommand::Reload(args),
        } = cli.command
        else {
            panic!("expected reload")
        };
        assert_eq!(args.socket, Some(std::path::PathBuf::from("/tmp/x.sock")));
    }

    #[test]
    fn an_explicit_flag_is_taken_verbatim() {
        let cli = Cli::try_parse_from(["doppel", "serve", "--config", "/from/flag.yaml"]).unwrap();
        let Command::Serve(args) = cli.command else {
            panic!("expected serve")
        };
        assert_eq!(
            args.store.config,
            std::path::PathBuf::from("/from/flag.yaml")
        );
    }

    #[test]
    fn postgres_store_parses_so_it_can_be_refused_with_a_clear_message() {
        let cli = Cli::parse_from(["doppel", "serve", "--store", "postgres"]);
        let Command::Serve(args) = cli.command else {
            panic!("expected serve")
        };
        assert_eq!(args.store.store, StoreKind::Postgres);
    }

    #[test]
    fn masks_the_password_in_a_dsn() {
        assert_eq!(
            mask_dsn("postgres://user:secret@host:5432/doppel"),
            "postgres://user:***@host:5432/doppel"
        );
    }

    #[test]
    fn masks_a_dsn_with_no_password_unchanged() {
        assert_eq!(
            mask_dsn("postgres://user@host:5432/doppel"),
            "postgres://user@host:5432/doppel"
        );
    }

    #[test]
    fn masking_leaves_a_non_url_alone_rather_than_leaking_it() {
        assert_eq!(mask_dsn("not a url"), "<unparseable dsn>");
    }

    #[test]
    fn opening_a_postgres_store_is_refused_with_exit_code_2() {
        let args = StoreArgs {
            store: StoreKind::Postgres,
            config: "./main.yaml".into(),
            config_name: "default".to_owned(),
            database_url: Some("postgres://u:p@h/db".to_owned()),
        };
        // `Arc<dyn ConfigStore>` is not `Debug`, so `unwrap_err` (which
        // requires the `Ok` type to be `Debug`) does not apply here.
        let err = match args.open() {
            Ok(_) => panic!("expected the postgres store to be refused"),
            Err(err) => err,
        };
        assert_eq!(err.exit_code(), 2);
        assert!(err.to_string().contains("not available in this build"));
        assert!(
            !err.to_string().contains(":p@"),
            "the dsn must not leak into the message"
        );
        // The negative check above (no raw password) was trivially true
        // before `dsn` even existed on this variant. Assert the positive
        // too: the masked form the caller supplied must actually show up in
        // the message, not merely "no password leaked because nothing about
        // the dsn appears at all".
        assert!(
            err.to_string().contains("postgres://u:***@h/db"),
            "the masked dsn must appear in the message, got: {err}"
        );
    }

    #[test]
    fn store_args_debug_masks_the_database_url_password() {
        let args = StoreArgs {
            store: StoreKind::Postgres,
            config: "./main.yaml".into(),
            config_name: "default".to_owned(),
            database_url: Some("postgres://u:p@h/db".to_owned()),
        };
        let debug = format!("{args:?}");
        assert!(
            !debug.contains(":p@"),
            "the dsn must not leak into Debug output: {debug}"
        );
    }
}
