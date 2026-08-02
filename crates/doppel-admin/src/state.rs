//! What every admin handler needs.

use std::sync::Arc;

use doppel_core::store::ConfigStore;

/// Shared handler state.
///
/// The store is the only thing in it, and that is the point: the API reads
/// and writes configuration through `ConfigStore` and never touches the
/// filesystem itself, which is what makes swapping in the PostgreSQL store a
/// matter of constructing a different `Arc` rather than rewriting handlers.
#[derive(Clone)]
pub struct AdminState {
    store: Arc<dyn ConfigStore>,
}

impl AdminState {
    #[must_use]
    pub fn new(store: Arc<dyn ConfigStore>) -> Self {
        Self { store }
    }

    #[must_use]
    pub fn store(&self) -> &dyn ConfigStore {
        self.store.as_ref()
    }
}
