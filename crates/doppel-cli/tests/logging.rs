//! End-to-end: JSON log field assertions, and the admin-token leak tests at
//! both the default and trace log levels.

mod common;

use common::{
    ChildGuard, Ports, SECRET_TOKEN, SIGNAL_WAIT_DEADLINE, Server, assert_socket_path_has_headroom,
    config, free_port, send_sigterm, upstream, wait_after_signal, wait_until_ready,
};
use std::process::{Command, Stdio};

#[test]
fn admin_token_values_never_reach_the_logs() {
    let up = upstream();
    let server = Server::start(up.port);
    server.get("/anything");

    send_sigterm(server.pid());
    let (_status, stdout, stderr) =
        wait_after_signal(server.into_child(), "SIGTERM", SIGNAL_WAIT_DEADLINE);

    assert!(
        !stdout.contains(SECRET_TOKEN),
        "an admin token leaked into stdout"
    );
    assert!(
        !stderr.contains(SECRET_TOKEN),
        "an admin token leaked into stderr"
    );
}

#[test]
fn admin_token_values_never_reach_the_logs_at_trace_level() {
    let up = upstream();
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("d.sock");
    let config_path = dir.path().join("main.yaml");
    assert_socket_path_has_headroom(&socket);
    let port = free_port();
    std::fs::write(
        &config_path,
        config(
            Ports {
                server: port,
                admin: free_port(),
                upstream: up.port,
            },
            &socket,
            &dir.path().join("templates"),
        ),
    )
    .unwrap();

    let mut child = ChildGuard::new(
        Command::new(env!("CARGO_BIN_EXE_doppel"))
            .args(["serve", "--config"])
            .arg(&config_path)
            .env("RUST_LOG", "trace")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );

    wait_until_ready(child.as_mut(), port, &socket);

    send_sigterm(child.as_mut().id());
    let (_status, stdout, stderr) =
        wait_after_signal(child.into_child(), "SIGTERM", SIGNAL_WAIT_DEADLINE);
    let combined = format!("{stdout}{stderr}");
    assert!(
        !combined.is_empty(),
        "trace level should produce some output"
    );
    assert!(
        !combined.contains(SECRET_TOKEN),
        "an admin token leaked at trace level"
    );
}

#[test]
fn logs_are_json_and_carry_the_documented_fields() {
    let up = upstream();
    let server = Server::start(up.port);
    server.get("/logged");

    send_sigterm(server.pid());
    let (_status, stdout, _stderr) =
        wait_after_signal(server.into_child(), "SIGTERM", SIGNAL_WAIT_DEADLINE);

    let line = stdout
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .find(|value| value["fields"]["message"] == "request proxied")
        .expect("expected a JSON log line for the proxied request");

    for field in [
        "request_id",
        "proxy",
        "method",
        "path",
        "status",
        "duration_ms",
        "upstream_contacted",
        "upstream_status",
        "upstream_duration_ms",
        "loss_injected",
        "latency_injected_ms",
    ] {
        assert!(
            !line["fields"][field].is_null(),
            "missing field `{field}` in {line}"
        );
    }
    assert_eq!(line["fields"]["path"], "/logged");
    assert_eq!(line["fields"]["proxy"], "p1");
}

/// The userinfo of a Sentry DSN is the key that authorises sending events,
/// so it is a secret in exactly the way an admin token is.
const SENTRY_DSN: &str = "https://s3cr3tsentrykey@sentry.invalid/42";

#[test]
fn a_configured_sentry_dsn_never_reaches_the_logs() {
    let up = upstream();
    let server = Server::start_with(up.port, |ports, socket, templates| {
        let base = config(ports, socket, templates);
        let with_sentry = base.replace(
            "proxies:",
            &format!("sentry:\n  dsn: \"{SENTRY_DSN}\"\nproxies:"),
        );
        assert_ne!(
            with_sentry, base,
            "the fixture must actually add a sentry section, or this test proves nothing"
        );
        with_sentry
    });
    server.get("/anything");

    send_sigterm(server.pid());
    let (_status, stdout, stderr) =
        wait_after_signal(server.into_child(), "SIGTERM", SIGNAL_WAIT_DEADLINE);

    let both = format!("{stdout}{stderr}");
    assert!(
        !both.contains("s3cr3tsentrykey"),
        "the sentry key leaked into the logs: {both}"
    );
    // The host survives, because the startup line naming Sentry is what tells
    // an operator the setting was seen at all.
    assert!(
        both.contains("sentry"),
        "startup should say something about sentry: {both}"
    );
}

#[test]
#[cfg_attr(feature = "sentry", ignore = "this build supports sentry")]
fn a_dsn_without_the_feature_is_reported_rather_than_ignored() {
    // A knob that reads as honoured and is not is the defect this project
    // already removed once, in `admin.workers`. The default build cannot
    // report to Sentry, so it has to say so.
    let up = upstream();
    let server = Server::start_with(up.port, |ports, socket, templates| {
        config(ports, socket, templates).replace(
            "proxies:",
            &format!("sentry:\n  dsn: \"{SENTRY_DSN}\"\nproxies:"),
        )
    });
    server.get("/anything");

    send_sigterm(server.pid());
    let (_status, stdout, stderr) =
        wait_after_signal(server.into_child(), "SIGTERM", SIGNAL_WAIT_DEADLINE);

    let both = format!("{stdout}{stderr}");
    assert!(
        both.contains("without the `sentry` feature"),
        "the build must say it cannot honour the DSN: {both}"
    );
    assert!(!both.contains("s3cr3tsentrykey"), "{both}");
}

/// The DSN from the environment, through the built binary.
///
/// The unit tests decide precedence; this is the half only a process can answer:
/// that the variable is read at all, that the startup line says which source won,
/// and that the key does not travel any further from the environment than it does
/// from the document.
#[test]
fn a_dsn_from_the_environment_is_read_and_its_source_named() {
    let up = upstream();
    // No `sentry` section in the document at all -- the case a deployment that
    // provisions credentials through the environment actually has.
    let server = Server::start_with_env(up.port, config, &[("DOPPEL_SENTRY_DSN", SENTRY_DSN)]);
    server.get("/anything");

    send_sigterm(server.pid());
    let (_status, stdout, stderr) =
        wait_after_signal(server.into_child(), "SIGTERM", SIGNAL_WAIT_DEADLINE);

    let both = format!("{stdout}{stderr}");
    assert!(
        !both.contains("s3cr3tsentrykey"),
        "the key leaked into the logs: {both}"
    );
    // Named the way an operator would grep for it, whether or not this build can
    // report: with the feature the line says reporting is enabled, without it that
    // nothing will be reported, and both carry the source.
    assert!(
        both.contains("DOPPEL_SENTRY_DSN"),
        "startup must say which source the dsn came from: {both}"
    );
    assert!(
        both.contains("sentry.invalid"),
        "the host survives redaction, which is what makes the line useful: {both}"
    );
}

/// An empty variable leaves the document's DSN alone.
///
/// The direction matters: `DOPPEL_SENTRY_DSN=${SENTRY_DSN}` with nothing behind
/// `SENTRY_DSN` is a compose file that means nothing by it, and silently turning
/// error reporting off is the worse reading of that.
#[test]
fn an_empty_variable_does_not_disable_a_configured_dsn() {
    let up = upstream();
    let server = Server::start_with_env(
        up.port,
        |ports, socket, templates| {
            config(ports, socket, templates).replace(
                "proxies:",
                &format!("sentry:\n  dsn: \"{SENTRY_DSN}\"\nproxies:"),
            )
        },
        &[("DOPPEL_SENTRY_DSN", "")],
    );
    server.get("/anything");

    send_sigterm(server.pid());
    let (_status, stdout, stderr) =
        wait_after_signal(server.into_child(), "SIGTERM", SIGNAL_WAIT_DEADLINE);

    let both = format!("{stdout}{stderr}");
    assert!(
        both.contains("sentry.dsn"),
        "the document's dsn must still be the one in force: {both}"
    );
    assert!(!both.contains("s3cr3tsentrykey"), "{both}");
}
