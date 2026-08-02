//! The admin HTTP API: proxy CRUD, template upload, reload, status, metrics.

pub mod access;
pub mod openapi;
pub mod proxies;
pub mod response;
pub mod state;
pub mod status;
pub mod templates;

pub use access::{Action, Caller, authorize, caller_from_headers};
pub use response::ApiError;
pub use state::AdminState;

/// The admin router, ready to serve.
///
/// A router rather than a server: `serve` owns the listener and the shutdown
/// signal, and the tests drive this directly without binding a port.
pub fn router(state: AdminState) -> axum::Router {
    axum::Router::new()
        .merge(proxies::routes())
        .merge(templates::routes())
        .merge(status::routes())
        .merge(openapi::routes())
        .with_state(state)
}
