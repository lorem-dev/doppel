//! The store conformance suite, against `FileStore`.
//!
//! The same suite runs against `PostgresStore` in that crate. Two callers of
//! one set of assertions is the point: it is the trait's contract, and testing
//! one implementation while assuming the other is how two implementations come
//! to mean different things by it.

use std::sync::Arc;

use doppel_core::store::{ConfigStore, FileStore};

#[tokio::test]
async fn the_file_store_satisfies_the_contract() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let store: Arc<dyn ConfigStore> = Arc::new(FileStore::new(
        dir.path().join("main.yaml"),
        dir.path().join("templates"),
    ));

    doppel_core::conformance::run_all(store.as_ref()).await;
}
