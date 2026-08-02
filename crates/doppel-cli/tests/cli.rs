//! End-to-end: `config validate` exit codes, config-path environment
//! variables, `serve` startup validation, and the postgres-store refusals.

mod common;

use common::{Ports, config, free_port, upstream};
use std::process::Command;

#[test]
fn config_validate_exits_zero_on_a_good_config() {
    let up = upstream();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("main.yaml");
    std::fs::write(
        &path,
        config(
            Ports {
                server: free_port(),
                admin: free_port(),
                upstream: up.port,
            },
            &dir.path().join("s.sock"),
            &dir.path().join("t"),
        ),
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_doppel"))
        .args(["config", "validate", "--config"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("configuration is valid"));
}

#[test]
fn config_validate_exits_one_and_lists_violations() {
    let up = upstream();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("main.yaml");
    let text = config(
        Ports {
            server: free_port(),
            admin: free_port(),
            upstream: up.port,
        },
        &dir.path().join("s.sock"),
        &dir.path().join("t"),
    )
    .replace("    resolve:", "    timeout: 0\n    resolve:");
    std::fs::write(&path, text).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_doppel"))
        .args(["config", "validate", "--config"])
        .arg(&path)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stdout).contains("proxies[0].timeout"));
}

#[test]
fn an_environment_variable_supplies_the_config_path() {
    let up = upstream();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("from-env.yaml");
    std::fs::write(
        &path,
        config(
            Ports {
                server: free_port(),
                admin: free_port(),
                upstream: up.port,
            },
            &dir.path().join("s.sock"),
            &dir.path().join("t"),
        ),
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_doppel"))
        .args(["config", "validate"])
        .env("DOPPEL_CONFIG_PATH", &path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn a_cli_flag_beats_the_environment_variable() {
    let up = upstream();
    let dir = tempfile::tempdir().unwrap();

    let good = dir.path().join("good.yaml");
    std::fs::write(
        &good,
        config(
            Ports {
                server: free_port(),
                admin: free_port(),
                upstream: up.port,
            },
            &dir.path().join("s.sock"),
            &dir.path().join("t"),
        ),
    )
    .unwrap();

    let bad = dir.path().join("bad.yaml");
    std::fs::write(
        &bad,
        config(
            Ports {
                server: free_port(),
                admin: free_port(),
                upstream: up.port,
            },
            &dir.path().join("s2.sock"),
            &dir.path().join("t2"),
        )
        .replace("    resolve:", "    timeout: 0\n    resolve:"),
    )
    .unwrap();

    // The environment points at the invalid config; the flag must win.
    let output = Command::new(env!("CARGO_BIN_EXE_doppel"))
        .args(["config", "validate", "--config"])
        .arg(&good)
        .env("DOPPEL_CONFIG_PATH", &bad)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "the flag must override DOPPEL_CONFIG_PATH, got: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn the_store_can_be_selected_by_environment_variable() {
    // Selected, and then refused for want of a URL -- which is the observable
    // proof that the variable was read at all. Before the PostgreSQL store
    // existed this asserted a blanket refusal; the selection is the part that
    // was ever being tested.
    let output = Command::new(env!("CARGO_BIN_EXE_doppel"))
        .args(["serve"])
        .env("DOPPEL_CONFIG_STORE", "postgres")
        .env_remove("DOPPEL_DATABASE_URL")
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(
        stderr.contains("--database-url"),
        "the refusal must name what is missing, got: {stderr}"
    );
}

#[test]
fn a_postgres_store_with_no_database_url_is_refused_by_name() {
    // Guessing a local default would let a mistyped environment talk to the
    // wrong database, which is not a mistake anyone notices in time.
    let output = Command::new(env!("CARGO_BIN_EXE_doppel"))
        .args(["serve", "--store", "postgres"])
        .env_remove("DOPPEL_DATABASE_URL")
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(stderr.contains("--database-url"), "{stderr}");
}

#[test]
fn config_reload_without_a_socket_reports_the_refusal_on_stderr_not_stdout() {
    // No `--socket`, so the path has to come from the configuration, which is
    // read through the store -- and a postgres store with nothing to connect
    // to cannot be opened. That is a failure to reach something rather than
    // this command's output, so it belongs on stderr, per the stream
    // convention in `main.rs`.
    let output = Command::new(env!("CARGO_BIN_EXE_doppel"))
        .args(["config", "reload", "--store", "postgres"])
        .env_remove("DOPPEL_DATABASE_URL")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--database-url"),
        "got stdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stdout.is_empty(),
        "a failure to reach the store is not this command's output: {stdout}"
    );
}

#[test]
fn config_validate_of_a_missing_file_reports_the_failure_on_stderr_not_stdout() {
    // `args.open()` failing to find the file is not `config validate`'s
    // output (a violations list) -- it is a failure to reach the store, so
    // it belongs on stderr.
    let dir = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_doppel"))
        .args(["config", "validate", "--config"])
        .arg(dir.path().join("absent.yaml"))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("absent.yaml"),
        "got stdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        !stdout.contains("absent.yaml"),
        "got stdout: {stdout}\nstderr: {stderr}"
    );
}

#[test]
fn zero_workers_is_a_usage_error_not_a_panic() {
    // `build_runtime` calls `tokio::runtime::Builder::worker_threads`, which
    // panics on 0 -- exit code 101, not a catchable error. The guard used to
    // be validation rule V3 over `server.workers`; now that the value is an
    // argument parsed as a non-zero integer, zero cannot be represented at
    // all and never reaches the builder. The protection moved; this test
    // moved with it rather than being deleted.
    let output = Command::new(env!("CARGO_BIN_EXE_doppel"))
        .args(["serve", "--workers", "0"])
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(2),
        "a bad argument is a usage error: {stderr}"
    );
    assert!(
        stderr.contains("workers"),
        "the message must name the argument, got: {stderr}"
    );
    assert!(
        !stderr.contains("panicked"),
        "must be a usage error, not a panic: {stderr}"
    );
}

#[test]
fn a_configuration_still_carrying_server_workers_is_rejected_by_name() {
    // Silently ignoring it would leave an operator believing the runtime is
    // sized when it is not -- the defect this project removed from
    // `admin.workers` in phase 3.
    let up = upstream();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("main.yaml");
    let text = config(
        Ports {
            server: free_port(),
            admin: free_port(),
            upstream: up.port,
        },
        &dir.path().join("s.sock"),
        &dir.path().join("t"),
    )
    .replace("\nlogging:", "\n  workers: 4\nlogging:");
    std::fs::write(&path, text).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_doppel"))
        .args(["serve", "--config"])
        .arg(&path)
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(
        stderr.contains("workers"),
        "the error must name the removed field, got: {stderr}"
    );
}

#[test]
fn config_migrate_masks_the_password_when_the_database_is_unreachable() {
    // The DSN reaches stderr on this path, and it is the one place an
    // operator's password could be printed by a command they ran to fix a
    // connection problem.
    let output = Command::new(env!("CARGO_BIN_EXE_doppel"))
        .args([
            "config",
            "migrate",
            "--database-url",
            "postgres://user:hunter2@127.0.0.1:1/nope",
        ])
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(!stderr.contains("hunter2"), "the password leaked: {stderr}");
    assert!(
        stderr.contains("***"),
        "expected a masked dsn, got: {stderr}"
    );
    // The host survives, because that is what the operator is checking.
    assert!(stderr.contains("127.0.0.1"), "{stderr}");
}

#[test]
fn config_migrate_requires_a_database_url() {
    // Defaulting to a local guess would let a mistyped environment migrate
    // the wrong database, which is not a mistake anyone notices in time.
    let output = Command::new(env!("CARGO_BIN_EXE_doppel"))
        .args(["config", "migrate"])
        .env_remove("DOPPEL_DATABASE_URL")
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("database-url"),
        "the message must name the missing argument"
    );
}
