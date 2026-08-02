//! End-to-end: reload applying a changed config, and reload rejecting an
//! invalid one while traffic keeps flowing.

mod common;

use common::{Server, upstream};

#[test]
fn reload_applies_a_changed_config() {
    let up = upstream();
    let server = Server::start(up.port);

    let text = std::fs::read_to_string(&server.config_path).unwrap();
    std::fs::write(
        &server.config_path,
        text.replace(
            "      type: default",
            "      type: default\n    loss:\n      percentage: 1.0\n      status: 503",
        ),
    )
    .unwrap();

    let output = server.reload();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("reloaded"));

    let (status, _) = server.get("/hello");
    assert_eq!(status, 503, "the new loss setting must be in effect");
}

#[test]
fn reload_of_an_invalid_config_is_rejected_and_traffic_keeps_flowing() {
    let up = upstream();
    let server = Server::start(up.port);

    let text = std::fs::read_to_string(&server.config_path).unwrap();
    std::fs::write(
        &server.config_path,
        text.replace("percentage", "percentaje"),
    )
    .unwrap();
    std::fs::write(
        &server.config_path,
        std::fs::read_to_string(&server.config_path)
            .unwrap()
            .replace("    resolve:", "    timeout: 0\n    resolve:"),
    )
    .unwrap();

    let output = server.reload();
    assert!(
        !output.status.success(),
        "an invalid config must fail the reload"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("CONFIG_INVALID"), "got: {stdout}");
    // The message locates the proxy and the line rather than a config path:
    // `timeout: 0` is refused by `TimeoutSeconds` while the document is being
    // parsed, so there is no rule to attribute it to. What matters for a
    // reload is unchanged -- it is rejected, it says which proxy, and it says
    // what to do.
    assert!(stdout.contains("proxies[0]"), "got: {stdout}");
    assert!(
        stdout.contains("would mean no timeout at all"),
        "got: {stdout}"
    );

    let (status, body) = server.get("/still-here");
    assert_eq!(status, 200, "the previous config must still be serving");
    assert_eq!(body, "upstream saw /still-here");
}
