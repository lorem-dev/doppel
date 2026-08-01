//! `doppel config validate`.

use doppel_core::{StoreError, Violation};

use crate::cli::StoreArgs;

/// The outcome of a validation run, kept separate from printing so it is
/// testable.
#[derive(Debug, Default)]
pub struct Report {
    pub violations: Vec<Violation>,
    pub message: Option<String>,
}

impl Report {
    #[must_use]
    pub fn exit_code(&self) -> u8 {
        u8::from(!self.violations.is_empty() || self.message.is_some())
    }
}

pub async fn validate(args: &StoreArgs) -> Report {
    let store = match args.open() {
        Ok(store) => store,
        Err(err) => {
            return Report {
                violations: Vec::new(),
                message: Some(err.to_string()),
            };
        }
    };

    match store.load().await {
        Ok(_) => Report::default(),
        Err(StoreError::Invalid(violations)) => Report {
            violations,
            message: None,
        },
        Err(err) => Report {
            violations: Vec::new(),
            message: Some(err.to_string()),
        },
    }
}

/// Print a report to stdout and return the process exit code.
pub fn print(report: &Report) -> u8 {
    if let Some(message) = &report.message {
        println!("{message}");
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
}
