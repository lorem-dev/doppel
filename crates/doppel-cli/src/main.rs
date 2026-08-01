// The control channel's public API (`ControlServer::bind`/`run`, `client::send`)
// has no caller yet: `main` is wired up to it in Task 16. Until then the plain
// (non-test) binary target has no path that reaches any of it, which `cargo
// clippy --all-targets -- -D warnings` would otherwise report as dead code.
#[allow(dead_code)]
mod control;

fn main() {
    println!("doppel");
}
