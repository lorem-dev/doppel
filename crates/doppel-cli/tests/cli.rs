//! End-to-end: `config validate` exit codes, config-path environment
//! variables, `serve` startup validation, and the postgres-store refusals.

mod common;

use common::{config, free_port, upstream};
use std::process::Command;

#[test]
fn config_validate_exits_zero_on_a_good_config() {
    let up = upstream();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("main.yaml");
    std::fs::write(
        &path,
        config(
            free_port(),
            up.port,
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
        free_port(),
        up.port,
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
            free_port(),
            up.port,
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
            free_port(),
            up.port,
            &dir.path().join("s.sock"),
            &dir.path().join("t"),
        ),
    )
    .unwrap();

    let bad = dir.path().join("bad.yaml");
    std::fs::write(
        &bad,
        config(
            free_port(),
            up.port,
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
    let output = Command::new(env!("CARGO_BIN_EXE_doppel"))
        .args(["serve"])
        .env("DOPPEL_CONFIG_STORE", "postgres")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn postgres_store_exits_two() {
    let output = Command::new(env!("CARGO_BIN_EXE_doppel"))
        .args(["serve", "--store", "postgres"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("not available in this build"));
}

#[test]
fn config_reload_without_a_socket_reports_the_refusal_on_stderr_not_stdout() {
    // No `--socket` and a postgres store: `resolve_socket` fails before ever
    // reaching a control socket, which is a failure to reach something, not
    // this command's output -- so it belongs on stderr, per the stream
    // convention in `main.rs`.
    let output = Command::new(env!("CARGO_BIN_EXE_doppel"))
        .args(["config", "reload", "--store", "postgres"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("not available in this build"),
        "got stdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        !stdout.contains("not available in this build"),
        "got stdout: {stdout}\nstderr: {stderr}"
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
fn serve_rejects_zero_workers_with_exit_code_one_not_a_panic() {
    // `server.workers: 0` must be caught by validation rule V3 before
    // anything acts on it. `build_runtime` calls
    // `tokio::runtime::Builder::worker_threads`, which panics (exit code
    // 101, not a catchable error) if the config is read before it is
    // validated -- exactly the ordering bug this test guards against. A
    // config typo must fail with exit code 1 and a message naming the
    // rule, never take the process down with a panic.
    let up = upstream();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("main.yaml");
    let text = config(
        free_port(),
        up.port,
        &dir.path().join("s.sock"),
        &dir.path().join("t"),
    )
    .replace("\nlogging:", "\n  workers: 0\nlogging:");
    std::fs::write(&path, text).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_doppel"))
        .args(["serve", "--config"])
        .arg(&path)
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(1),
        "expected exit code 1, got {:?}\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}",
        output.status.code()
    );
    // The violations list is `serve`'s startup-check output, not a failure
    // to reach or open anything, so it is on stdout -- same stream
    // `config validate` reports violations on.
    assert!(
        stdout.contains("server.workers"),
        "expected the error to name the rule, got: {stdout}"
    );
    assert!(
        !stderr.contains("panicked"),
        "must fail validation, not panic: {stderr}"
    );
}
