//! What every `ConfigStore` must do, written once.
//!
//! Behind the `test-support` feature. It lives beside the trait rather than in
//! one implementation's test suite because it is the trait's contract, not
//! either implementation's behaviour -- and because the alternative, testing
//! one store and assuming the other, is how two implementations of one
//! interface come to mean different things by it.
//!
//! Every check leaves the store as it found it well enough for the next one:
//! they each provision the configuration they need rather than inheriting one.
//! The caller supplies a store and disposes of it; what a fresh store means
//! differs too much between a directory and a schema for this module to know.

use crate::store::{ConfigStore, Revision, StoreError};
use crate::{Config, config};

/// A configuration every store must be able to hold. Deliberately small: this
/// suite is about the trait's behaviour, and the wide fixture that exercises
/// every column belongs with the store that has columns.
const BASE: &str = r#"
server:
  host: "127.0.0.1"
  port: 18080
admin:
  host: "127.0.0.1"
  port: 18081
  tokens:
    - name: root
      group: admin
      token: root-token
  access: {}
  upload:
    limit: 1M
proxies:
  - name: alpha
    type: http
    url: "https://alpha.example.com/api/"
"#;

fn base() -> Config {
    config::load_from_str(BASE).expect("the conformance fixture parses")
}

fn moved() -> Config {
    config::load_from_str(&BASE.replace("alpha.example.com", "moved.example.com"))
        .expect("the conformance fixture parses")
}

/// Run every check against `store`.
///
/// One function rather than a list a caller iterates, so a caller cannot run
/// some of them and believe it ran the suite.
pub async fn run_all(store: &dyn ConfigStore) {
    load_returns_what_save_wrote(store).await;
    a_stale_expected_revision_is_refused_and_writes_nothing(store).await;
    an_unconditional_save_needs_no_revision(store).await;
    the_current_revision_is_accepted(store).await;
    templates_of_an_unknown_proxy_are_empty_not_an_error(store).await;
    delete_template_reports_whether_it_existed(store).await;
    retain_templates_keeps_only_what_is_named(store).await;
    retain_templates_with_an_empty_keep_removes_everything(store).await;
    an_unusable_template_name_is_refused(store).await;
    templates_are_listed_in_a_stable_order(store).await;
}

async fn load_returns_what_save_wrote(store: &dyn ConfigStore) {
    let config = base();
    let saved = store.save(&config, None).await.expect("provision");
    let (loaded, revision) = store.load().await.expect("load");

    assert_eq!(loaded, config, "load must return what save was given");
    assert_eq!(revision, saved, "load and save must agree on the revision");
    assert_eq!(
        saved,
        Revision::of_config(&config),
        "the revision must be the one derived from the content, so two stores \
         holding the same configuration report the same number"
    );
}

async fn a_stale_expected_revision_is_refused_and_writes_nothing(store: &dyn ConfigStore) {
    let config = base();
    let saved = store.save(&config, None).await.expect("provision");

    let err = store
        .save(&moved(), Some(Revision(saved.0 ^ 1)))
        .await
        .expect_err("a stale expected revision must be refused");
    assert!(
        matches!(err, StoreError::RevisionMismatch { .. }),
        "expected RevisionMismatch, got {err:?}"
    );

    let (loaded, revision) = store.load().await.expect("load");
    assert_eq!(loaded, config, "a refused save must change nothing");
    assert_eq!(revision, saved);
}

async fn an_unconditional_save_needs_no_revision(store: &dyn ConfigStore) {
    store.save(&base(), None).await.expect("provision");
    store
        .save(&moved(), None)
        .await
        .expect("an unconditional save must not need a revision");

    let (loaded, _) = store.load().await.expect("load");
    assert_eq!(loaded, moved());
}

async fn the_current_revision_is_accepted(store: &dyn ConfigStore) {
    let saved = store.save(&base(), None).await.expect("provision");
    let next = store
        .save(&moved(), Some(saved))
        .await
        .expect("the current revision must be accepted");

    assert_ne!(next, saved, "a changed configuration must change revision");
    let (loaded, _) = store.load().await.expect("load");
    assert_eq!(loaded, moved());
}

async fn templates_of_an_unknown_proxy_are_empty_not_an_error(store: &dyn ConfigStore) {
    // Having no templates is normal, so it cannot be reported the same way a
    // broken store is.
    let files = store
        .load_templates("no-such-proxy")
        .await
        .expect("an unknown proxy must not be an error");
    assert!(files.is_empty());
}

async fn delete_template_reports_whether_it_existed(store: &dyn ConfigStore) {
    store.save(&base(), None).await.expect("provision");
    store
        .save_template("alpha", "conformance-a.j2", b"x")
        .await
        .expect("save");

    assert!(
        store
            .delete_template("alpha", "conformance-a.j2")
            .await
            .expect("delete"),
        "deleting a template that was there must report true"
    );
    assert!(
        !store
            .delete_template("alpha", "conformance-a.j2")
            .await
            .expect("delete"),
        "deleting it again must report false"
    );
}

async fn retain_templates_keeps_only_what_is_named(store: &dyn ConfigStore) {
    store.save(&base(), None).await.expect("provision");
    store.save_template("alpha", "keep.j2", b"x").await.unwrap();
    store.save_template("alpha", "drop.j2", b"y").await.unwrap();

    store
        .retain_templates("alpha", &["keep.j2".to_owned()])
        .await
        .expect("retain");

    let names: Vec<_> = store
        .load_templates("alpha")
        .await
        .expect("list")
        .into_iter()
        .map(|file| file.name)
        .collect();
    assert_eq!(names, vec!["keep.j2".to_owned()]);
}

async fn retain_templates_with_an_empty_keep_removes_everything(store: &dyn ConfigStore) {
    store.save(&base(), None).await.expect("provision");
    store.save_template("alpha", "one.j2", b"x").await.unwrap();
    store.save_template("alpha", "two.j2", b"y").await.unwrap();

    store.retain_templates("alpha", &[]).await.expect("retain");

    assert!(
        store
            .load_templates("alpha")
            .await
            .expect("list")
            .is_empty(),
        "an empty keep must remove the proxy's storage entirely"
    );
}

async fn an_unusable_template_name_is_refused(store: &dyn ConfigStore) {
    store.save(&base(), None).await.expect("provision");
    for name in ["..", "a/b", "", ".hidden", "a..b"] {
        let err = store
            .save_template("alpha", name, b"x")
            .await
            .expect_err(&format!("`{name}` must be refused"));
        assert!(
            matches!(err, StoreError::BadTemplateName { .. }),
            "`{name}` must be refused as a bad name, not as {err:?}"
        );
    }
}

async fn templates_are_listed_in_a_stable_order(store: &dyn ConfigStore) {
    // Two stores that listed in different orders would make any caller that
    // compares listings -- the admin API's template endpoint, a test -- depend
    // on which store it happened to have.
    store.save(&base(), None).await.expect("provision");
    store.retain_templates("alpha", &[]).await.expect("clear");
    for name in ["c.j2", "a.j2", "b.j2"] {
        store.save_template("alpha", name, b"x").await.unwrap();
    }

    let names: Vec<_> = store
        .load_templates("alpha")
        .await
        .expect("list")
        .into_iter()
        .map(|file| file.name)
        .collect();
    assert_eq!(
        names,
        vec!["a.j2".to_owned(), "b.j2".to_owned(), "c.j2".to_owned()],
        "listings must be ordered by name, whichever store answers"
    );

    // Leave nothing behind for the next check.
    store.retain_templates("alpha", &[]).await.expect("clear");
}
