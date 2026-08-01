//! End-to-end: a real `doppel` binary proxying a request to a real upstream.

mod common;

use common::{Server, upstream};

#[test]
fn proxies_a_request_end_to_end() {
    let up = upstream();
    let server = Server::start(up.port);
    let (status, body) = server.get("/hello");
    assert_eq!(status, 200);
    assert_eq!(body, "upstream saw /hello");
}
