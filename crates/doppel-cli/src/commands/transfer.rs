//! `doppel config push` and `doppel config pull`.
//!
//! How an operator moves a configuration between the two stores, and the only
//! way to seed a database that has never held one.

use doppel_core::store::Revision;
use doppel_store_postgres::PostgresStore;

use crate::cli::{CliError, PullArgs, PushArgs, mask_dsn};

/// Read the file, validate it, write it to the database.
///
/// Validation happens inside `save`, so an invalid document is refused with
/// every violation rather than the first one -- and refused before anything is
/// written, since the whole write is one transaction.
pub async fn push(args: &PushArgs) -> Result<String, CliError> {
    let config = doppel_core::config::load_from_path(&args.config)
        .map_err(|err| CliError::Failed(err.to_string()))?;

    let expected = args
        .if_revision
        .as_deref()
        .map(|text| {
            text.parse::<Revision>().map_err(|err| {
                CliError::Failed(format!("`--if-revision {text}` is not a revision: {err}"))
            })
        })
        .transpose()?;

    // The templates directory is irrelevant here -- `push` writes a
    // configuration and touches no template -- but the store needs one, so it
    // is given the value the document names rather than a placeholder that
    // would be wrong if anything ever did use it.
    let store = PostgresStore::connect(
        &args.database_url,
        &args.config_name,
        config.templates.dir.clone(),
    )
    .await
    .map_err(failed(&args.database_url))?;

    let revision = store
        .save_config(&config, expected)
        .await
        .map_err(failed(&args.database_url))?;

    Ok(format!(
        "pushed `{}` to `{}` at revision {revision}",
        args.config.display(),
        args.config_name
    ))
}

/// Read the database and render the document.
///
/// Canonical YAML, the same serialization the revision is computed over, so a
/// pulled document pushed straight back produces the same revision it came
/// from. A reformatted one would not, and the operator would be told their
/// configuration had changed when nothing about it had.
pub async fn pull(args: &PullArgs) -> Result<String, CliError> {
    let store = PostgresStore::connect(&args.database_url, &args.config_name, ".")
        .await
        .map_err(failed(&args.database_url))?;

    let (config, revision) = store
        .load_config()
        .await
        .map_err(failed(&args.database_url))?;

    let yaml = doppel_core::config::to_yaml(&config)
        .map_err(|err| CliError::Failed(format!("cannot render the configuration: {err}")))?;

    match &args.output {
        Some(path) => {
            std::fs::write(path, &yaml).map_err(|err| {
                CliError::Failed(format!("cannot write {}: {err}", path.display()))
            })?;
            Ok(format!(
                "pulled `{}` at revision {revision} into {}",
                args.config_name,
                path.display()
            ))
        }
        // The document itself is this command's output, so it goes to stdout
        // with nothing else on it -- `doppel config pull > main.yaml` has to
        // produce a file that parses.
        None => Ok(yaml),
    }
}

/// A store failure with the DSN masked.
///
/// The message reaches stderr on a path that cannot rely on logging being
/// initialised, so the masking happens here rather than at a call site that
/// might not run.
fn failed(url: &str) -> impl Fn(doppel_core::store::StoreError) -> CliError + '_ {
    move |err| CliError::Failed(format!("{err} (database url: {})", mask_dsn(url)))
}
