//! `config push` and `config pull`, driven through the built binary.
//!
//! Through the binary rather than the library, because what these commands
//! promise is about files and streams: that a pulled document written to a
//! file is one a push would accept unchanged, and that a refusal reaches
//! stderr rather than the document's stream.

use doppel_store_postgres::test_support::{TestSchema, require_database};
use std::process::Command;

fn doppel(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_doppel"))
        .args(args)
        .env_remove("DOPPEL_DATABASE_URL")
        .env_remove("DOPPEL_CONFIG_NAME")
        .env_remove("DOPPEL_CONFIG_PATH")
        .output()
        .expect("doppel runs")
}

async fn migrated(url: &str) -> TestSchema {
    let schema = TestSchema::create(url).await;
    schema.migrate().await;
    schema
}

/// The repository's reference configuration: the fixture with the widest
/// coverage of the schema, and the one a newcomer copies.
fn reference() -> String {
    std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../main.example.yaml"
    ))
    .expect("the reference configuration must be readable")
}

/// What the reference document looks like once parsed and rendered back --
/// which is what the database stores, comments and layout being no part of it.
fn canonical(yaml: &str) -> String {
    let config = doppel_core::config::load_from_str(yaml).expect("the fixture parses");
    doppel_core::config::to_yaml(&config).expect("it renders")
}

#[tokio::test]
async fn push_then_pull_returns_the_document_unchanged() {
    // The property that makes these two a pair: a pulled document is one a
    // push would accept, byte for byte. Anything less and moving a
    // configuration between the two stores is lossy in a way nobody notices
    // until a revision changes for no reason.
    let Some(url) = require_database() else {
        return;
    };
    let schema = migrated(&url).await;
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("main.yaml");
    std::fs::write(&source, reference()).unwrap();

    let pushed = doppel(&[
        "config",
        "push",
        "--config",
        source.to_str().unwrap(),
        "--database-url",
        &schema.url(),
    ]);
    assert!(
        pushed.status.success(),
        "{}",
        String::from_utf8_lossy(&pushed.stderr)
    );

    let pulled = doppel(&["config", "pull", "--database-url", &schema.url()]);
    assert!(
        pulled.status.success(),
        "{}",
        String::from_utf8_lossy(&pulled.stderr)
    );

    assert_eq!(
        String::from_utf8_lossy(&pulled.stdout),
        canonical(&reference()),
        "a pulled document must be exactly what was stored"
    );

    schema.drop().await;
}

#[tokio::test]
async fn a_pulled_file_can_be_pushed_back_at_the_same_revision() {
    // The revision is computed over the canonical serialization, so a document
    // that survived the trip must produce the number it left with. A
    // reformatting anywhere in the chain would report a change to an operator
    // who made none.
    let Some(url) = require_database() else {
        return;
    };
    let schema = migrated(&url).await;
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("main.yaml");
    let round_tripped = dir.path().join("pulled.yaml");
    std::fs::write(&source, reference()).unwrap();

    let first = doppel(&[
        "config",
        "push",
        "--config",
        source.to_str().unwrap(),
        "--database-url",
        &schema.url(),
    ]);
    let first_report = String::from_utf8_lossy(&first.stdout).into_owned();

    let pulled = doppel(&[
        "config",
        "pull",
        "--database-url",
        &schema.url(),
        "--output",
        round_tripped.to_str().unwrap(),
    ]);
    assert!(
        pulled.status.success(),
        "{}",
        String::from_utf8_lossy(&pulled.stderr)
    );

    let second = doppel(&[
        "config",
        "push",
        "--config",
        round_tripped.to_str().unwrap(),
        "--database-url",
        &schema.url(),
    ]);
    let second_report = String::from_utf8_lossy(&second.stdout).into_owned();

    assert!(second.status.success(), "{second_report}");
    // The revision is parsed out of a human-readable report, so its shape is
    // checked before the two are compared. Without this, a reworded report
    // would make both sides equal-and-wrong -- two empty strings compare fine
    // -- and the test would go on passing while measuring nothing.
    let revision_of = |report: &str| {
        let value = report
            .rsplit(' ')
            .next()
            .expect("the report ends with the revision")
            .trim()
            .to_owned();
        assert!(
            value.len() == 16 && value.chars().all(|c| c.is_ascii_hexdigit()),
            "expected a revision at the end of the report, got {value:?} from {report:?}"
        );
        value
    };
    assert_eq!(
        revision_of(&first_report),
        revision_of(&second_report),
        "the round trip must not move the revision"
    );

    schema.drop().await;
}

#[tokio::test]
async fn an_invalid_document_writes_nothing_and_reports_every_violation() {
    let Some(url) = require_database() else {
        return;
    };
    let schema = migrated(&url).await;
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("main.yaml");
    // Two faults, so "every violation" is a claim the test can actually check.
    std::fs::write(
        &source,
        reference()
            .replace("timeout: 60", "timeout: 0")
            .replace("limit: 1M", "limit: 0"),
    )
    .unwrap();

    let pushed = doppel(&[
        "config",
        "push",
        "--config",
        source.to_str().unwrap(),
        "--database-url",
        &schema.url(),
    ]);

    let stderr = String::from_utf8_lossy(&pushed.stderr);
    assert!(
        !pushed.status.success(),
        "an invalid document must not push"
    );
    assert!(stderr.contains("timeout"), "{stderr}");
    assert!(stderr.contains("upload.limit"), "{stderr}");
    assert_eq!(
        schema.count("configurations").await,
        0,
        "a refused push must leave nothing behind"
    );

    schema.drop().await;
}

#[tokio::test]
async fn a_stale_if_revision_refuses_and_changes_nothing() {
    let Some(url) = require_database() else {
        return;
    };
    let schema = migrated(&url).await;
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("main.yaml");
    std::fs::write(&source, reference()).unwrap();

    let first = doppel(&[
        "config",
        "push",
        "--config",
        source.to_str().unwrap(),
        "--database-url",
        &schema.url(),
    ]);
    assert!(first.status.success());

    std::fs::write(
        &source,
        reference().replace(
            "https://external-service.com/api/v1/",
            "https://moved.example.com/",
        ),
    )
    .unwrap();
    let refused = doppel(&[
        "config",
        "push",
        "--config",
        source.to_str().unwrap(),
        "--database-url",
        &schema.url(),
        "--if-revision",
        "0123456789abcdef",
    ]);

    let stderr = String::from_utf8_lossy(&refused.stderr);
    assert!(
        !refused.status.success(),
        "a stale revision must be refused"
    );
    assert!(stderr.to_lowercase().contains("revision"), "{stderr}");

    // And the first push is still what is stored.
    let pulled = doppel(&["config", "pull", "--database-url", &schema.url()]);
    assert!(
        String::from_utf8_lossy(&pulled.stdout).contains("external-service.com"),
        "the refused push must not have landed"
    );

    schema.drop().await;
}

#[tokio::test]
async fn a_malformed_if_revision_is_refused_before_anything_is_read() {
    // Not a mismatch -- nothing was compared. Telling an operator their copy
    // is stale would send them to re-read when the fix is to correct what they
    // typed.
    let Some(url) = require_database() else {
        return;
    };
    let schema = migrated(&url).await;
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("main.yaml");
    std::fs::write(&source, reference()).unwrap();

    let refused = doppel(&[
        "config",
        "push",
        "--config",
        source.to_str().unwrap(),
        "--database-url",
        &schema.url(),
        "--if-revision",
        "not-a-revision",
    ]);

    let stderr = String::from_utf8_lossy(&refused.stderr);
    assert!(!refused.status.success());
    assert!(stderr.contains("--if-revision"), "{stderr}");
    assert!(
        !stderr.to_lowercase().contains("mismatch"),
        "a typo is not a stale copy: {stderr}"
    );

    schema.drop().await;
}

#[tokio::test]
async fn pulling_a_name_that_is_not_there_fails_rather_than_writing_an_empty_file() {
    // `doppel config pull --output main.yaml` on a wrong name must not leave a
    // truncated file behind: the next thing to read it would be a server.
    let Some(url) = require_database() else {
        return;
    };
    let schema = migrated(&url).await;
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("out.yaml");

    let pulled = doppel(&[
        "config",
        "pull",
        "--database-url",
        &schema.url(),
        "--config-name",
        "no-such-config",
        "--output",
        target.to_str().unwrap(),
    ]);

    assert!(!pulled.status.success());
    assert!(
        !target.exists(),
        "nothing must be written when there is nothing to write"
    );
    assert!(String::from_utf8_lossy(&pulled.stdout).is_empty());

    schema.drop().await;
}

#[tokio::test]
async fn a_failure_never_prints_the_password() {
    let output = doppel(&[
        "config",
        "pull",
        "--database-url",
        "postgres://user:hunter2@127.0.0.1:1/nope",
    ]);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(!output.status.success());
    assert!(!stderr.contains("hunter2"), "the password leaked: {stderr}");
    assert!(stderr.contains("***"), "expected a masked dsn: {stderr}");
}
