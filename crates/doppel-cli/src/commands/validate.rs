//! `doppel config validate`.

use doppel_core::Violation;

use crate::cli::StoreArgs;

/// The outcome of a validation run, kept separate from printing so it is
/// testable.
#[derive(Debug, Default)]
pub struct Report {
    pub violations: Vec<Violation>,
    pub message: Option<String>,
    /// Overrides the exit code that would otherwise be derived from
    /// `violations`/`message` (which can only ever be 0 or 1). A failure that
    /// carries its own exit code -- `StoreArgs::open()` refusing
    /// `--store postgres` with code 2, for instance -- must not be flattened
    /// down to the generic "invalid configuration" code 1, or a script
    /// branching on the exit code cannot tell the two apart.
    pub code: Option<u8>,
}

impl Report {
    #[must_use]
    pub fn exit_code(&self) -> u8 {
        self.code.unwrap_or(u8::from(
            !self.violations.is_empty() || self.message.is_some(),
        ))
    }
}

pub async fn validate(args: &StoreArgs) -> Report {
    // `open()` does the one parse this command needs; what remains is purely
    // the semantic rule checks, so there is no second read of the file the
    // way a `store.load()` call would add.
    let (_store, config) = match args.open() {
        Ok(opened) => opened,
        Err(err) => {
            return Report {
                violations: Vec::new(),
                message: Some(err.to_string()),
                code: Some(err.exit_code()),
            };
        }
    };

    match doppel_core::validate::validate(&config) {
        Ok(()) => Report::default(),
        Err(violations) => Report {
            violations,
            message: None,
            code: None,
        },
    }
}

/// Print a report and return the process exit code.
///
/// Stream convention (see `main.rs`): the violations list and the
/// "configuration is valid" message are this command's actual output, so
/// they go to stdout. `report.message` is only ever set when `args.open()`
/// failed to reach or open the store in the first place -- not this
/// command's output -- so it goes to stderr.
pub fn print(report: &Report) -> u8 {
    if let Some(message) = &report.message {
        eprintln!("{message}");
    }
    for violation in &report.violations {
        println!("{violation}");
    }
    if report.exit_code() == 0 {
        println!("configuration is valid");
    }
    report.exit_code()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{StoreArgs, StoreKind};

    const GOOD: &str = r#"
server:
  host: "127.0.0.1"
  port: 8080
admin:
  host: "127.0.0.1"
  port: 8081
  tokens: []
  access: {}
  upload:
    limit: 1M
proxies:
  - name: p1
    type: http
    url: "https://example.com/"
"#;

    fn args(dir: &std::path::Path, text: &str) -> StoreArgs {
        let path = dir.join("main.yaml");
        std::fs::write(&path, text).unwrap();
        StoreArgs {
            store: StoreKind::File,
            config: path,
            config_name: "default".to_owned(),
            database_url: None,
        }
    }

    #[tokio::test]
    async fn a_valid_config_reports_no_violations() {
        let dir = tempfile::tempdir().unwrap();
        let report = validate(&args(dir.path(), GOOD)).await;
        assert!(report.violations.is_empty());
        assert_eq!(report.exit_code(), 0);
    }

    #[tokio::test]
    async fn an_invalid_config_lists_every_violation_and_exits_1() {
        let dir = tempfile::tempdir().unwrap();
        let text = GOOD
            .replace("port: 8081", "port: 8080")
            .replace("limit: 1M", "limit: 0");
        let report = validate(&args(dir.path(), &text)).await;
        assert_eq!(report.exit_code(), 1);
        assert!(report.violations.iter().any(|v| v.path == "admin.port"));
        assert!(
            report
                .violations
                .iter()
                .any(|v| v.path == "admin.upload.limit")
        );
    }

    #[tokio::test]
    async fn a_missing_config_exits_1_with_a_message_naming_the_path() {
        let dir = tempfile::tempdir().unwrap();
        let mut args = args(dir.path(), GOOD);
        args.config = dir.path().join("absent.yaml");
        let report = validate(&args).await;
        assert_eq!(report.exit_code(), 1);
        assert!(report.message.as_deref().unwrap().contains("absent.yaml"));
    }

    #[tokio::test]
    async fn validation_ignores_a_missing_templates_directory() {
        // Preflight checks belong to `serve`, not to validation, so that
        // `config validate` answers the same on a laptop as in production.
        let dir = tempfile::tempdir().unwrap();
        let text = format!("{GOOD}\ntemplates:\n  dir: /definitely/not/here\n");
        let report = validate(&args(dir.path(), &text)).await;
        assert_eq!(report.exit_code(), 0);
    }

    #[tokio::test]
    async fn validating_a_postgres_store_exits_2_not_1() {
        // `StoreArgs::open()` refuses `--store postgres` with exit code 2;
        // `validate` must carry that code through rather than flattening
        // every failure down to 1, or a script cannot tell "your config is
        // wrong" apart from "this build cannot do that".
        let args = StoreArgs {
            store: StoreKind::Postgres,
            config: "./main.yaml".into(),
            config_name: "default".to_owned(),
            database_url: None,
        };
        let report = validate(&args).await;
        assert_eq!(report.exit_code(), 2);
    }
}
