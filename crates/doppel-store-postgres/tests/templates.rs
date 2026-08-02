//! Template files: rows, and the disk mirror they are kept in step with.

mod common;

use common::{TestSchema, require_database};
use doppel_core::store::ConfigStore;

async fn migrated(url: &str) -> TestSchema {
    let schema = TestSchema::create(url).await;
    schema.migrate().await;
    schema
}

fn mirrored(schema: &TestSchema, proxy: &str, file: &str) -> std::path::PathBuf {
    schema.templates_dir().join(proxy).join(file)
}

#[tokio::test]
async fn upload_list_and_delete_round_trip_through_the_database() {
    let Some(url) = require_database() else {
        return;
    };
    let schema = migrated(&url).await;
    let store = schema.store().await;

    store
        .save_template("alpha", "one.j2", b"hello")
        .await
        .expect("save");

    let files = store.load_templates("alpha").await.expect("list");
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].name, "one.j2");
    assert_eq!(files[0].content, b"hello");
    assert_eq!(schema.count("templates").await, 1);

    // And the mirror, because the render path reads a file, not a row.
    assert_eq!(
        std::fs::read(mirrored(&schema, "alpha", "one.j2")).expect("mirrored"),
        b"hello"
    );

    assert!(
        store
            .delete_template("alpha", "one.j2")
            .await
            .expect("delete")
    );
    assert_eq!(schema.count("templates").await, 0);
    assert!(!mirrored(&schema, "alpha", "one.j2").exists());

    schema.drop().await;
}

#[tokio::test]
async fn deleting_reports_whether_the_row_existed() {
    let Some(url) = require_database() else {
        return;
    };
    let schema = migrated(&url).await;
    let store = schema.store().await;
    store.save_template("alpha", "one.j2", b"x").await.unwrap();

    assert!(store.delete_template("alpha", "one.j2").await.unwrap());
    assert!(!store.delete_template("alpha", "one.j2").await.unwrap());

    schema.drop().await;
}

#[tokio::test]
async fn the_answer_comes_from_the_row_not_the_mirror() {
    // The row is the truth. A mirror that has drifted -- because a previous
    // run was killed between the two writes -- must not change what `delete`
    // reports, or a caller would be told a template is gone while the database
    // still serves it.
    let Some(url) = require_database() else {
        return;
    };
    let schema = migrated(&url).await;
    let store = schema.store().await;
    store.save_template("alpha", "one.j2", b"x").await.unwrap();
    std::fs::remove_file(mirrored(&schema, "alpha", "one.j2")).unwrap();

    assert!(
        store.delete_template("alpha", "one.j2").await.unwrap(),
        "the row existed, so the answer is true regardless of the mirror"
    );

    schema.drop().await;
}

#[tokio::test]
async fn listing_an_unknown_proxy_is_empty_rather_than_an_error() {
    // Same contract the file store has: having no templates is normal.
    let Some(url) = require_database() else {
        return;
    };
    let schema = migrated(&url).await;
    let store = schema.store().await;

    assert!(store.load_templates("ghost").await.unwrap().is_empty());

    schema.drop().await;
}

#[tokio::test]
async fn a_template_name_that_fails_the_name_check_is_refused() {
    let Some(url) = require_database() else {
        return;
    };
    let schema = migrated(&url).await;
    let store = schema.store().await;

    for name in ["..", "a/b", "", ".hidden"] {
        assert!(
            store.save_template("alpha", name, b"x").await.is_err(),
            "`{name}` must be refused"
        );
    }
    assert_eq!(schema.count("templates").await, 0);

    schema.drop().await;
}

#[tokio::test]
async fn retain_keeps_only_what_is_named() {
    let Some(url) = require_database() else {
        return;
    };
    let schema = migrated(&url).await;
    let store = schema.store().await;
    store.save_template("alpha", "keep.j2", b"x").await.unwrap();
    store.save_template("alpha", "drop.j2", b"y").await.unwrap();

    store
        .retain_templates("alpha", &["keep.j2".to_owned()])
        .await
        .expect("retain");

    let names: Vec<_> = store
        .load_templates("alpha")
        .await
        .unwrap()
        .into_iter()
        .map(|f| f.name)
        .collect();
    assert_eq!(names, vec!["keep.j2".to_owned()]);
    assert!(mirrored(&schema, "alpha", "keep.j2").exists());
    assert!(!mirrored(&schema, "alpha", "drop.j2").exists());

    schema.drop().await;
}

#[tokio::test]
async fn retain_with_an_empty_keep_removes_everything() {
    let Some(url) = require_database() else {
        return;
    };
    let schema = migrated(&url).await;
    let store = schema.store().await;
    store.save_template("alpha", "one.j2", b"x").await.unwrap();
    store.save_template("alpha", "two.j2", b"y").await.unwrap();
    store.save_template("beta", "other.j2", b"z").await.unwrap();

    store.retain_templates("alpha", &[]).await.expect("retain");

    assert!(store.load_templates("alpha").await.unwrap().is_empty());
    assert!(!schema.templates_dir().join("alpha").exists());
    // Another proxy's files are untouched: the contract is per proxy.
    assert_eq!(store.load_templates("beta").await.unwrap().len(), 1);

    schema.drop().await;
}

#[tokio::test]
async fn retain_also_removes_a_foreign_file_from_the_mirror() {
    // The mirror is a cache of the rows, so anything in it that no row
    // accounts for is stale by definition -- including a file some other
    // program left there.
    let Some(url) = require_database() else {
        return;
    };
    let schema = migrated(&url).await;
    let store = schema.store().await;
    store.save_template("alpha", "keep.j2", b"x").await.unwrap();
    std::fs::write(
        schema.templates_dir().join("alpha").join(".DS_Store"),
        b"junk",
    )
    .unwrap();

    store
        .retain_templates("alpha", &["keep.j2".to_owned()])
        .await
        .expect("retain");

    assert!(
        !schema
            .templates_dir()
            .join("alpha")
            .join(".DS_Store")
            .exists()
    );
    assert!(mirrored(&schema, "alpha", "keep.j2").exists());

    schema.drop().await;
}

#[tokio::test]
async fn materialize_writes_what_the_database_holds_and_removes_what_it_does_not() {
    // What a second instance needs after its peer uploaded something: the rows
    // are already shared, the files are not.
    let Some(url) = require_database() else {
        return;
    };
    let schema = migrated(&url).await;
    let store = schema.store().await;
    store
        .save_template("alpha", "one.j2", b"fresh")
        .await
        .unwrap();

    // Simulate the peer's disk: the row is there, the file is not, and a stale
    // file is.
    std::fs::remove_file(mirrored(&schema, "alpha", "one.j2")).unwrap();
    std::fs::write(mirrored(&schema, "alpha", "stale.j2"), b"old").unwrap();

    store
        .materialize_templates(schema.templates_dir())
        .await
        .expect("materialize");

    assert_eq!(
        std::fs::read(mirrored(&schema, "alpha", "one.j2")).expect("written"),
        b"fresh"
    );
    assert!(
        !mirrored(&schema, "alpha", "stale.j2").exists(),
        "a file no row accounts for is stale"
    );

    schema.drop().await;
}

#[tokio::test]
async fn materializing_with_no_templates_at_all_is_not_an_error() {
    let Some(url) = require_database() else {
        return;
    };
    let schema = migrated(&url).await;
    let store = schema.store().await;

    store
        .materialize_templates(schema.templates_dir())
        .await
        .expect("an empty store materializes to nothing, successfully");

    schema.drop().await;
}
