//! Command line surface. The full flag set is defined now, including the
//! PostgreSQL options, so that phase 4 adds behaviour rather than syntax.

use std::path::PathBuf;
use std::sync::Arc;

use clap::{Args, Parser, Subcommand, ValueEnum};
use doppel_core::Config;
use doppel_core::store::{ConfigStore, FileStore, StoreError};
use doppel_store_postgres::PostgresStore;

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
    /// Manage admin tokens.
    Token {
        #[command(subcommand)]
        command: TokenCommand,
    },
    /// Print the version.
    Version,
}

#[derive(Debug, Subcommand)]
pub enum TokenCommand {
    /// Issue a new admin token on a running server.
    ///
    /// The value is generated, stored, and printed once. There is no command
    /// to read it back, because nothing stores it in a form this could read:
    /// it is in the configuration, and that is the copy to guard.
    Add(TokenAddArgs),
}

#[derive(Debug, Args)]
pub struct TokenAddArgs {
    /// What to call the token.
    #[arg(long)]
    pub name: doppel_core::config::Name,
    /// The group it belongs to. Defaults to `user`.
    ///
    /// Not `admin`: a command that hands out administrative rights by default
    /// is one mistyped invocation away from an incident.
    #[arg(long)]
    pub group: Option<doppel_core::config::Name>,
    /// Control socket path. Defaults to the value in the configuration.
    #[arg(long)]
    pub socket: Option<PathBuf>,
    #[command(flatten)]
    pub store: StoreArgs,
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// Validate the configuration and report every violation.
    Validate(StoreArgs),
    /// Ask a running server to reload its configuration.
    Reload(ReloadArgs),
    /// Write a YAML configuration into the database.
    ///
    /// The two ends are named separately rather than through `--store`: this
    /// command always reads a file and always writes a database, so a store
    /// selector here would be a flag with one legal value and a misleading
    /// name.
    Push(PushArgs),
    /// Read a configuration out of the database as YAML.
    Pull(PullArgs),
    /// Apply any database migrations the configured database has not seen.
    ///
    /// Its own command, and never done on startup: a process that silently
    /// alters a shared schema when it boots turns a rollback into data loss,
    /// and the operator who rolled back is the one least expecting it.
    Migrate(MigrateArgs),
}

#[derive(Args)]
pub struct PushArgs {
    /// The YAML document to write.
    #[arg(long, env = "DOPPEL_CONFIG_PATH", default_value = "./main.yaml")]
    pub config: PathBuf,
    #[arg(long, env = "DOPPEL_DATABASE_URL")]
    pub database_url: String,
    #[arg(long, env = "DOPPEL_CONFIG_NAME", default_value = "default")]
    pub config_name: String,
    /// Write only if the stored configuration is still at this revision.
    ///
    /// Without it the write is unconditional, which is what provisioning
    /// wants. With it, `push` is the same compare-and-swap the admin API
    /// uses, so a scripted push cannot silently overwrite a change someone
    /// made in between.
    #[arg(long)]
    pub if_revision: Option<String>,
}

/// Hand-written, like `StoreArgs`'s, so `{:?}` cannot print the password.
impl std::fmt::Debug for PushArgs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PushArgs")
            .field("config", &self.config)
            .field("database_url", &mask_dsn(&self.database_url))
            .field("config_name", &self.config_name)
            .field("if_revision", &self.if_revision)
            .finish()
    }
}

#[derive(Args)]
pub struct PullArgs {
    #[arg(long, env = "DOPPEL_DATABASE_URL")]
    pub database_url: String,
    #[arg(long, env = "DOPPEL_CONFIG_NAME", default_value = "default")]
    pub config_name: String,
    /// Where to write the document. Absent means stdout, so `pull` composes
    /// with a pipe.
    #[arg(long)]
    pub output: Option<PathBuf>,
}

impl std::fmt::Debug for PullArgs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PullArgs")
            .field("database_url", &mask_dsn(&self.database_url))
            .field("config_name", &self.config_name)
            .field("output", &self.output)
            .finish()
    }
}

#[derive(Args)]
pub struct MigrateArgs {
    /// The database to migrate. Required: there is nothing to migrate without
    /// one, and defaulting to some local guess would let a mistyped
    /// environment migrate the wrong database.
    #[arg(long, env = "DOPPEL_DATABASE_URL")]
    pub database_url: String,
}

/// Hand-written, like `StoreArgs`'s, so `{:?}` cannot print the password.
impl std::fmt::Debug for MigrateArgs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MigrateArgs")
            .field("database_url", &mask_dsn(&self.database_url))
            .finish()
    }
}

#[derive(Debug, Args)]
pub struct ServeArgs {
    /// Tokio worker threads. Defaults to the machine's available
    /// parallelism.
    ///
    /// `NonZeroUsize` rather than a validated `usize`: `worker_threads`
    /// panics on 0 instead of returning an error, so a 0 that got that far
    /// would take the process down with exit code 101. Making it
    /// unrepresentable turns that into a usage message.
    #[arg(long, env = "DOPPEL_WORKERS")]
    pub workers: Option<std::num::NonZeroUsize>,
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
    #[error("{0}")]
    Failed(String),
    /// The configuration itself is unreadable or malformed.
    ///
    /// Separate from `Failed` for one reason: it is a finding about the
    /// document, so `config validate` prints it to stdout with the
    /// violations, while an unreachable store goes to stderr. Most bad values
    /// now fail here rather than as a rule -- the types refuse them while the
    /// document is being parsed -- so a validate that wrote them to stderr
    /// would leave anything reading its stdout seeing an empty findings list
    /// for a broken configuration.
    #[error("{0}")]
    BadConfig(String),
}

impl CliError {
    /// Exit code 1 for every failure this type carries.
    ///
    /// There used to be a code 2 for "the postgres store is not available in
    /// this build". It is gone with the refusal it described: keeping a code
    /// nothing can produce would leave the CLI reference promising a
    /// behaviour, and a script branching on it waiting for something that
    /// never comes. Clap still exits 2 on a usage error, which is its
    /// convention and not this type's business.
    #[must_use]
    pub fn exit_code(&self) -> u8 {
        match self {
            Self::Failed(_) | Self::BadConfig(_) => 1,
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
    pub async fn open(&self) -> Result<(Arc<dyn ConfigStore>, Config), CliError> {
        match self.store {
            StoreKind::Postgres => {
                let url = self.database_url.as_deref().ok_or_else(|| {
                    CliError::Failed(
                        "--database-url is required with --store postgres; there is nothing to \
                         connect to without it, and guessing a local default would let a \
                         mistyped environment talk to the wrong database"
                            .to_owned(),
                    )
                })?;

                // The templates directory is a chicken-and-egg: it lives in
                // the configuration, and the configuration lives behind the
                // store. Opened with a placeholder, then reopened once the
                // real value is known -- one extra connect at startup, which
                // is the cheapest way to keep `templates.dir` a configuration
                // field rather than a second command-line argument nobody
                // would remember to keep in step with it.
                let probe = PostgresStore::connect(url, &self.config_name, ".")
                    .await
                    .map_err(store_failed(url))?;
                let (config, _) = probe.load().await.map_err(|err| match err {
                    // A stored configuration that no longer parses is the
                    // same kind of finding as a malformed file, and reaches
                    // the operator through the same channel.
                    doppel_core::store::StoreError::Invalid(_) => {
                        CliError::BadConfig(format!("{}: {err}", mask_dsn(url)))
                    }
                    other => store_failed(url)(other),
                })?;
                drop(probe);

                let store =
                    PostgresStore::connect(url, &self.config_name, config.templates.dir.clone())
                        .await
                        .map_err(store_failed(url))?;
                Ok((Arc::new(store), config))
            }
            StoreKind::File => {
                let config = doppel_core::config::load_from_path(&self.config).map_err(|err| {
                    match err {
                        // A document that does not parse is a finding about
                        // the configuration. A file that is absent or
                        // unreadable is the file store failing to answer at
                        // all, which is the other kind of problem -- there is
                        // no configuration to have found anything about.
                        doppel_core::config::ConfigError::Parse(_) => {
                            CliError::BadConfig(err.to_string())
                        }
                        doppel_core::config::ConfigError::NotFound(_)
                        | doppel_core::config::ConfigError::Io { .. } => {
                            CliError::Failed(err.to_string())
                        }
                    }
                })?;
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

/// A store failure on its way to stderr, with the DSN masked.
///
/// The message reaches stderr on every path that prints a `CliError`, none of
/// which can be relied on to have logging initialised, so the masking happens
/// here rather than at a call site that might not run.
fn store_failed(url: &str) -> impl Fn(StoreError) -> CliError + '_ {
    move |err| CliError::Failed(format!("{err} (database url: {})", mask_dsn(url)))
}

impl From<StoreError> for CliError {
    fn from(err: StoreError) -> Self {
        Self::Failed(err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn workers_defaults_to_absent_meaning_available_parallelism() {
        let cli = Cli::parse_from(["doppel", "serve"]);
        let Command::Serve(args) = cli.command else {
            panic!("expected serve")
        };
        assert_eq!(args.workers, None);
    }

    #[test]
    fn workers_is_taken_from_the_command_line() {
        let cli = Cli::parse_from(["doppel", "serve", "--workers", "4"]);
        let Command::Serve(args) = cli.command else {
            panic!("expected serve")
        };
        assert_eq!(args.workers.map(std::num::NonZeroUsize::get), Some(4));
    }

    #[test]
    fn zero_workers_is_unrepresentable_rather_than_validated() {
        // `Builder::worker_threads` panics on 0 rather than returning an
        // error, so a 0 that reached it would take the process down with exit
        // code 101. `NonZeroUsize` makes it a parse failure with a usage
        // message instead of a rule someone has to remember to write.
        let parsed = Cli::try_parse_from(["doppel", "serve", "--workers", "0"]);
        assert!(parsed.is_err(), "0 workers must not parse");
    }

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

    #[tokio::test]
    async fn a_postgres_store_without_a_database_url_is_refused_by_name() {
        // There used to be a blanket refusal here, with its own exit code,
        // because the store did not exist. It exists now, so the only thing
        // left to refuse is the case where there is nothing to connect to --
        // and guessing a local default would let a mistyped environment talk
        // to the wrong database.
        let args = StoreArgs {
            store: StoreKind::Postgres,
            config: "./main.yaml".into(),
            config_name: "default".to_owned(),
            database_url: None,
        };
        // `Arc<dyn ConfigStore>` is not `Debug`, so `unwrap_err` (which
        // requires the `Ok` type to be `Debug`) does not apply here.
        let err = match args.open().await {
            Ok(_) => panic!("expected a refusal with no database url"),
            Err(err) => err,
        };
        assert_eq!(err.exit_code(), 1);
        assert!(
            err.to_string().contains("--database-url"),
            "the message must name the missing argument, got: {err}"
        );
    }

    #[tokio::test]
    async fn an_unreachable_database_is_reported_with_the_password_masked() {
        // The DSN reaches stderr on this path, and it is the one place an
        // operator's password could be printed by the command they ran to
        // diagnose a connection problem.
        let args = StoreArgs {
            store: StoreKind::Postgres,
            config: "./main.yaml".into(),
            config_name: "default".to_owned(),
            database_url: Some("postgres://u:hunter2@127.0.0.1:1/db".to_owned()),
        };
        let err = match args.open().await {
            Ok(_) => panic!("port 1 cannot be a database"),
            Err(err) => err,
        };
        let message = err.to_string();
        assert!(
            !message.contains("hunter2"),
            "the password leaked: {message}"
        );
        assert!(
            message.contains("127.0.0.1"),
            "the host must survive: {message}"
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
