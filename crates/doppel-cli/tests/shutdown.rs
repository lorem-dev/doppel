//! End-to-end: SIGTERM draining the server and removing the control socket.

mod common;

use common::{SIGNAL_WAIT_DEADLINE, Server, send_sigterm, upstream, wait_after_signal};

#[test]
fn sigterm_drains_and_removes_the_socket() {
    let up = upstream();
    let server = Server::start(up.port);
    let socket = server.socket.clone();
    let pid = server.pid();

    send_sigterm(pid);

    let (status, _stdout, _stderr) =
        wait_after_signal(server.into_child(), "SIGTERM", SIGNAL_WAIT_DEADLINE);
    assert!(status.success(), "expected a clean exit, got {status:?}");
    assert!(
        !socket.exists(),
        "the control socket must be removed on shutdown"
    );
}
