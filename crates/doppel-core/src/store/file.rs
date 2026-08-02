//! The YAML file implementation of `ConfigStore`.

use std::io::Write;
use std::path::{Path, PathBuf};

use super::{ConfigStore, Revision, StoreError, TemplateFile, name::sanitize};
use crate::config::{self, Config, ConfigError};
use crate::validate::validate;

pub struct FileStore {
    config_path: PathBuf,
    templates_dir: PathBuf,
}

impl FileStore {
    /// `templates_dir` is passed in rather than read from the config, so that
    /// template operations work before the first successful load.
    pub fn new(config_path: impl Into<PathBuf>, templates_dir: impl Into<PathBuf>) -> Self {
        Self {
            config_path: config_path.into(),
            templates_dir: templates_dir.into(),
        }
    }

    #[must_use]
    pub fn config_path(&self) -> &Path {
        &self.config_path
    }

    #[must_use]
    pub fn templates_dir(&self) -> &Path {
        &self.templates_dir
    }

    /// The dedicated sibling file `save` takes its cross-process lock on.
    ///
    /// This is deliberately not `self.config_path` itself. `save` writes the
    /// new content to a temporary file and `rename`s it into place, and
    /// `rename` replaces the directory entry's inode wholesale: a lock held
    /// on the config path's pre-rename inode would go on guarding a file
    /// that is no longer reachable at that path the instant the rename
    /// lands, while a second process opened its own handle on the same path
    /// and is now happily writing straight through the "protected" file.
    /// Locking a name that `save` never renames and never removes is what
    /// keeps the lock meaningful across that rename. Do not "simplify" this
    /// to `self.config_path`.
    fn lock_path(&self) -> PathBuf {
        let mut name = self.config_path.as_os_str().to_owned();
        name.push(".lock");
        PathBuf::from(name)
    }

    /// The revision of whatever is currently on disk at `config_path`, used
    /// by `save`'s compare-and-swap check. Called only while the lock in
    /// `save` is held, so it always sees the latest committed write from any
    /// process sharing this file.
    ///
    /// `Revision(0)` is a reserved sentinel meaning "nothing is stored at
    /// this path yet": a caller that supplies `expected: Some(_)` believes it
    /// is updating a configuration that exists, so a missing file must be a
    /// mismatch, not free rein to create one. `fnv1a`'s output is not
    /// otherwise guaranteed to avoid zero, but a real configuration's
    /// canonical YAML happening to hash to exactly the same value as this
    /// sentinel is the same order of accident (about 1 in 2^64) that this
    /// whole scheme already accepts for any two distinct configurations
    /// colliding, so reusing it here adds no meaningfully new risk.
    fn current_revision(config_path: &Path) -> Result<Revision, StoreError> {
        match config::load_from_path(config_path) {
            Ok(config) => Ok(Revision::of_config(&config)),
            Err(ConfigError::NotFound(_)) => Ok(Revision(0)),
            Err(ConfigError::Io { path, source }) => Err(StoreError::Io { path, source }),
            Err(ConfigError::Parse(err)) => {
                Err(StoreError::Invalid(vec![crate::validate::Violation::new(
                    "",
                    err.to_string(),
                )]))
            }
        }
    }

    fn proxy_dir(&self, proxy: &str) -> Result<PathBuf, StoreError> {
        // A proxy name becomes a path component, so it gets the same treatment
        // as a file name.
        let proxy = sanitize(proxy)?;
        Ok(self.templates_dir.join(proxy))
    }

    fn io(path: &Path) -> impl Fn(std::io::Error) -> StoreError + '_ {
        move |source| StoreError::Io {
            path: path.to_path_buf(),
            source,
        }
    }

    /// Decode a directory entry's raw file name, treating anything that is
    /// not valid UTF-8 as foreign junk rather than something to report or
    /// compare under a mangled, `to_string_lossy`-produced stand-in.
    /// `sanitize` takes a `&str`, so no real template name ever stored
    /// through `save_template` can fail to decode here; a name that does is
    /// something else's file, the same treatment `load_templates` already
    /// gives a name that merely fails `sanitize`.
    fn decode_template_name(name: std::ffi::OsString) -> Option<String> {
        name.into_string().ok()
    }
}

#[async_trait::async_trait]
impl ConfigStore for FileStore {
    async fn load(&self) -> Result<(Config, Revision), StoreError> {
        // Reads take no lock. `save`'s final `rename` is atomic, so a reader
        // racing a writer always observes either the complete old
        // configuration or the complete new one at `config_path`, never a
        // partial write; there is nothing here for a lock to protect.
        let config = match config::load_from_path(&self.config_path) {
            Ok(config) => config,
            Err(ConfigError::NotFound(path)) => return Err(StoreError::NotFound(path)),
            Err(ConfigError::Io { path, source }) => return Err(StoreError::Io { path, source }),
            Err(ConfigError::Parse(err)) => {
                return Err(StoreError::Invalid(vec![crate::validate::Violation::new(
                    "",
                    err.to_string(),
                )]));
            }
        };
        validate(&config).map_err(StoreError::Invalid)?;
        let revision = Revision::of_config(&config);
        Ok((config, revision))
    }

    async fn save(
        &self,
        config: &Config,
        expected: Option<Revision>,
    ) -> Result<Revision, StoreError> {
        validate(config).map_err(StoreError::Invalid)?;

        let lock_path = self.lock_path();
        // The lock file is created if absent, but never truncated, renamed,
        // or removed: it exists only to be locked. See `lock_path` for why
        // it, and not `self.config_path`, is what gets locked here.
        let lock_file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            // Explicit, not the default: this file's whole point is to keep
            // existing so it can be locked, and clippy correctly wants
            // `create(true)` paired with an explicit truncation choice.
            .truncate(false)
            .open(&lock_path)
            .map_err(Self::io(&lock_path))?;
        // A blocking, exclusive, advisory lock, held from here to the end of
        // this function. It serialises this whole check-then-write sequence
        // (the revision comparison below, the serialization, the temporary
        // file write, the sync, and the rename) across every process
        // sharing this configuration file -- which an in-process mutex
        // cannot do, since it is invisible to the other processes in the
        // deployment this store targets. It also serialises threads within
        // one process: each thread opens its own file description on
        // `lock_path`, and the OS advisory lock queues those exactly as it
        // would two separate processes, so no second, in-process lock is
        // kept alongside this one.
        //
        // This is locked on `lock_path` (`<config>.lock`), never on
        // `self.config_path` itself: see `lock_path` for why locking the
        // config file directly would be a bug that looks correct (the
        // rename below replaces its inode, so a lock on the pre-rename
        // inode would guard a path a second process has already moved on
        // from). Reads (`load`) take no lock at all: `save`'s final rename
        // is atomic, so a concurrent reader always observes either the
        // complete old configuration or the complete new one, never a
        // partial write, and there is nothing for a lock to protect there.
        //
        // No explicit `unlock` call: dropping `lock_file` closes its
        // descriptor, and the OS releases an advisory lock when the last
        // descriptor referring to it closes. Every early return below (via
        // `?`) still drops `lock_file` on its way out on a normal return
        // path (this is not a panic unwind), so a rejected save never
        // leaves the lock held for the next caller.
        lock_file.lock().map_err(Self::io(&lock_path))?;

        // WARNING: no `.await` from here to the end of this function. Every
        // one of the standing comments above this line about `save` being
        // unable to deadlock (an in-process mutex being unnecessary, an OS
        // lock queuing threads exactly like processes, and so on) is true
        // only because this region never suspends while holding the lock.
        // Adding an `.await` here would let the executor put this task
        // aside mid-lock, and is the one edit that would turn that argument
        // false.
        if let Some(expected_revision) = expected {
            let actual = Self::current_revision(&self.config_path)?;
            if actual != expected_revision {
                // Nothing has been written yet: the mismatch is caught
                // before serialization even starts, so the file on disk is
                // untouched.
                return Err(StoreError::RevisionMismatch {
                    expected: expected_revision,
                    actual,
                });
            }
        }

        let yaml = config::to_yaml(config).map_err(|e| StoreError::Serialize(e.to_string()))?;

        let dir = self.config_path.parent().unwrap_or(Path::new("."));
        // The temporary file must share the destination's directory, otherwise
        // the rename below could cross a filesystem boundary and stop being
        // atomic. A crash mid-write must never leave a truncated config, so the
        // final rename has to be a same-filesystem, all-or-nothing operation.
        let mut temp = tempfile::NamedTempFile::new_in(dir).map_err(Self::io(dir))?;
        temp.write_all(yaml.as_bytes())
            .map_err(Self::io(temp.path()))?;
        temp.as_file().sync_all().map_err(Self::io(temp.path()))?;
        temp.persist(&self.config_path)
            .map_err(|e| StoreError::Io {
                path: self.config_path.clone(),
                source: e.error,
            })?;

        // The rename itself is atomic and content is never at risk: a
        // concurrent reader always sees the complete old configuration or
        // the complete new one (see the comments on locking above), never a
        // partial write. But the directory entry that now points at the new
        // inode is not guaranteed durable just because `persist` returned --
        // on most POSIX filesystems a rename's effect on the directory can
        // still be sitting in volatile cache, and a crash before it reaches
        // disk can leave the *old* entry (or, on some filesystems, neither)
        // after a reboot, even though nothing was ever torn. Opening the
        // directory and syncing it is what makes the rename survive that
        // crash, so the durability of the last successful `save` matches
        // what its caller was told.
        //
        // If this sync fails, the error below is returned *after* the
        // rename above has already put the new config in place: the new
        // content is live and readable at `config_path` right now, in this
        // process, regardless of what a future crash might do to it. A
        // caller must not read an `Err` here as "nothing was written" --
        // that would only be true if the process crashes before the next
        // successful sync of this directory (by this call or another),
        // taking the not-yet-durable rename down with it. Short of that,
        // the write already landed.
        std::fs::File::open(dir)
            .and_then(|dir_file| dir_file.sync_all())
            .map_err(Self::io(dir))?;

        Ok(Revision::of_config(config))
    }

    async fn load_templates(&self, proxy: &str) -> Result<Vec<TemplateFile>, StoreError> {
        let dir = self.proxy_dir(proxy)?;
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(source) => return Err(StoreError::Io { path: dir, source }),
        };

        let mut files = Vec::new();
        for entry in entries {
            let entry = entry.map_err(Self::io(&dir))?;
            if !entry.file_type().map_err(Self::io(&dir))?.is_file() {
                continue;
            }
            let Some(name) = Self::decode_template_name(entry.file_name()) else {
                continue;
            };
            // A name that fails `sanitize` here is not an operator's template
            // request, it is something else's file sitting in our directory
            // (a `.DS_Store`, an editor swapfile, ...). This is a listing, so
            // silently leave it off the list rather than erroring out.
            if sanitize(&name).is_err() {
                continue;
            }
            let content = std::fs::read(entry.path()).map_err(Self::io(&entry.path()))?;
            files.push(TemplateFile { name, content });
        }
        files.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(files)
    }

    async fn save_template(&self, proxy: &str, file: &str, bytes: &[u8]) -> Result<(), StoreError> {
        let dir = self.proxy_dir(proxy)?;
        let file = sanitize(file)?;
        std::fs::create_dir_all(&dir).map_err(Self::io(&dir))?;
        let path = dir.join(file);
        std::fs::write(&path, bytes).map_err(Self::io(&path))
    }

    async fn delete_template(&self, proxy: &str, file: &str) -> Result<bool, StoreError> {
        let dir = self.proxy_dir(proxy)?;
        let file = sanitize(file)?;
        let path = dir.join(file);
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(true),
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(source) => Err(StoreError::Io { path, source }),
        }
    }

    async fn retain_templates(&self, proxy: &str, keep: &[String]) -> Result<(), StoreError> {
        let dir = self.proxy_dir(proxy)?;
        if keep.is_empty() {
            return match std::fs::remove_dir_all(&dir) {
                Ok(()) => Ok(()),
                Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(source) => Err(StoreError::Io { path: dir, source }),
            };
        }

        // This is a directory cleanup, not an operator request, so it walks
        // `read_dir` directly instead of going through `load_templates` and
        // `delete_template`. Every name here came from the directory listing:
        // by construction it is already a single path component inside `dir`,
        // never an operator-supplied string that could escape it. Re-running
        // `sanitize` on it would buy no safety and would leave foreign files
        // such as `.DS_Store` undeletable, breaking the "drop everything not
        // kept" contract.
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(source) => return Err(StoreError::Io { path: dir, source }),
        };
        for entry in entries {
            let entry = entry.map_err(Self::io(&dir))?;
            if !entry.file_type().map_err(Self::io(&dir))?.is_file() {
                continue;
            }
            // A non-UTF-8 name cannot equal anything in `keep` -- `keep` is
            // only ever built from sanitize-passing, valid-UTF-8 template
            // names -- so there is no need to mangle it through
            // `to_string_lossy` just to compare it: treat it the same as any
            // other name not in `keep`, i.e. delete it below.
            let keep_this = Self::decode_template_name(entry.file_name())
                .is_some_and(|name| keep.contains(&name));
            if keep_this {
                continue;
            }
            let path = entry.path();
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
                Err(source) => return Err(StoreError::Io { path, source }),
            }
        }
        Ok(())
    }
}

// Blocking `std::fs` calls are used throughout this file even though every
// method here is `async fn`. That is deliberate: configuration is loaded and
// saved rarely, and the files involved are small, so the cost of blocking the
// executor thread briefly is lower than the complexity of async filesystem
// plumbing would buy back. The atomic rename in `save` also wants a
// synchronous file handle: `NamedTempFile::persist` is a blocking rename
// under the hood, and there is no async equivalent worth adding a dependency
// for.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::load_from_str;

    const GOOD: &str = r#"
server:
  host: "127.0.0.1"
  port: 8080
admin:
  host: "127.0.0.1"
  port: 8081
  tokens: []
  access: {}
  upload:
    limit: 1Mi
proxies:
  - name: p1
    type: http
    url: "https://example.com/"
"#;

    fn store(dir: &std::path::Path) -> (FileStore, std::path::PathBuf) {
        let config_path = dir.join("main.yaml");
        std::fs::write(&config_path, GOOD).unwrap();
        let store = FileStore::new(config_path.clone(), dir.join("templates"));
        (store, config_path)
    }

    #[tokio::test]
    async fn load_returns_a_validated_config() {
        let dir = tempfile::tempdir().unwrap();
        let (store, _) = store(dir.path());
        let (config, _revision) = store.load().await.unwrap();
        assert_eq!(config.proxies[0].name, "p1");
    }

    #[tokio::test]
    async fn load_reports_a_missing_file_distinctly() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileStore::new(dir.path().join("absent.yaml"), dir.path().join("templates"));
        assert!(matches!(store.load().await, Err(StoreError::NotFound(_))));
    }

    #[tokio::test]
    async fn load_rejects_an_invalid_config_with_violations() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("main.yaml");
        std::fs::write(&config_path, GOOD.replace("port: 8081", "port: 8080")).unwrap();
        let store = FileStore::new(config_path, dir.path().join("templates"));
        let Err(StoreError::Invalid(violations)) = store.load().await else {
            panic!("expected Invalid");
        };
        assert!(violations.iter().any(|v| v.path == "admin.port"));
    }

    #[tokio::test]
    async fn save_refuses_an_invalid_config_and_leaves_the_file_alone() {
        let dir = tempfile::tempdir().unwrap();
        let (store, config_path) = store(dir.path());
        let (mut config, _) = store.load().await.unwrap();
        config.admin.port = config.server.port;

        assert!(matches!(
            store.save(&config, None).await,
            Err(StoreError::Invalid(_))
        ));
        assert_eq!(std::fs::read_to_string(&config_path).unwrap(), GOOD);
    }

    #[tokio::test]
    async fn the_reference_config_survives_a_save_and_load_round_trip() {
        // Guards against a field that deserializes but does not serialize back,
        // which would silently truncate the config the first time the admin API
        // writes it in phase 3.
        let dir = tempfile::tempdir().unwrap();
        let reference = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../main.example.yaml"
        ))
        .unwrap();
        let config_path = dir.path().join("main.yaml");
        std::fs::write(&config_path, &reference).unwrap();
        let store = FileStore::new(config_path, dir.path().join("templates"));

        let (before, _) = store.load().await.unwrap();
        store.save(&before, None).await.unwrap();
        let (after, _) = store.load().await.unwrap();
        assert_eq!(before, after);
    }

    #[tokio::test]
    async fn save_leaves_no_temporary_files_behind() {
        let dir = tempfile::tempdir().unwrap();
        let (store, _) = store(dir.path());
        let (config, _) = store.load().await.unwrap();
        store.save(&config, None).await.unwrap();

        // The lock file is expected to be there; it is not a leftover.
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|name| name != "main.yaml" && name != "main.yaml.lock")
            .collect();
        assert!(leftovers.is_empty(), "unexpected leftovers: {leftovers:?}");
    }

    // -- Revision identity --------------------------------------------------

    #[tokio::test]
    async fn saving_the_same_configuration_twice_returns_the_same_revision() {
        let dir = tempfile::tempdir().unwrap();
        let (store, _) = store(dir.path());
        let (config, _) = store.load().await.unwrap();

        let first = store.save(&config, None).await.unwrap();
        let second = store.save(&config, Some(first)).await.unwrap();
        assert_eq!(first, second);
    }

    #[tokio::test]
    async fn saving_a_changed_configuration_returns_a_different_revision() {
        let dir = tempfile::tempdir().unwrap();
        let (store, _) = store(dir.path());
        let (mut config, _) = store.load().await.unwrap();

        let before = store.save(&config, None).await.unwrap();
        config.proxies[0].name = crate::config::Name::parse("renamed").unwrap();
        let after = store.save(&config, Some(before)).await.unwrap();
        assert_ne!(before, after);
    }

    #[tokio::test]
    async fn load_returns_the_same_revision_the_preceding_save_returned() {
        let dir = tempfile::tempdir().unwrap();
        let (store, _) = store(dir.path());
        let (config, _) = store.load().await.unwrap();

        let saved = store.save(&config, None).await.unwrap();
        let (_, loaded) = store.load().await.unwrap();
        assert_eq!(saved, loaded);
    }

    #[tokio::test]
    async fn two_stores_over_separate_files_agree_on_the_revision_of_the_same_configuration() {
        let dir = tempfile::tempdir().unwrap();
        let (store_a, _) = store(dir.path());
        let config_b_path = dir.path().join("other.yaml");
        std::fs::write(&config_b_path, GOOD).unwrap();
        let store_b = FileStore::new(config_b_path, dir.path().join("templates-b"));

        let (_, revision_a) = store_a.load().await.unwrap();
        let (_, revision_b) = store_b.load().await.unwrap();
        assert_eq!(revision_a, revision_b);
    }

    // Same configuration as `GOOD`, but reindented and with a comment added:
    // different bytes, same meaning. Kept as a full YAML literal (rather than
    // derived from `GOOD` by string surgery) so its indentation is honestly
    // different rather than mechanically doubled.
    const GOOD_REFORMATTED: &str = r#"
# same configuration as GOOD, just formatted differently
server:
    host: "127.0.0.1"
    port: 8080
admin:
    host: "127.0.0.1"
    port: 8081
    tokens: []
    access: {}
    upload:
        limit: 1Mi
proxies:
    - name: p1
      type: http
      url: "https://example.com/"
"#;

    #[tokio::test]
    async fn reformatting_without_changing_meaning_does_not_change_the_revision() {
        let dir = tempfile::tempdir().unwrap();
        let path_a = dir.path().join("a.yaml");
        let path_b = dir.path().join("b.yaml");
        std::fs::write(&path_a, GOOD).unwrap();
        std::fs::write(&path_b, GOOD_REFORMATTED).unwrap();
        let store_a = FileStore::new(path_a, dir.path().join("templates-a"));
        let store_b = FileStore::new(path_b, dir.path().join("templates-b"));

        let (config_a, revision_a) = store_a.load().await.unwrap();
        let (config_b, revision_b) = store_b.load().await.unwrap();
        assert_eq!(config_a, config_b);
        assert_eq!(revision_a, revision_b);
    }

    // -- Compare-and-swap -----------------------------------------------------

    #[tokio::test]
    async fn save_with_no_expected_writes_unconditionally() {
        let dir = tempfile::tempdir().unwrap();
        let (store, config_path) = store(dir.path());
        let (mut config, _) = store.load().await.unwrap();
        config.proxies[0].name = crate::config::Name::parse("renamed").unwrap();

        store.save(&config, None).await.unwrap();
        let reloaded = load_from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
        assert_eq!(reloaded.proxies[0].name, "renamed");
    }

    #[tokio::test]
    async fn save_with_the_correct_expected_revision_succeeds() {
        let dir = tempfile::tempdir().unwrap();
        let (store, _) = store(dir.path());
        let (mut config, revision) = store.load().await.unwrap();
        config.proxies[0].name = crate::config::Name::parse("renamed").unwrap();

        let new_revision = store.save(&config, Some(revision)).await.unwrap();
        assert_ne!(new_revision, revision);
    }

    #[tokio::test]
    async fn save_with_a_stale_expected_revision_fails_and_leaves_the_file_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let (store, config_path) = store(dir.path());
        let (config, revision) = store.load().await.unwrap();
        let before = std::fs::read(&config_path).unwrap();

        // Not the current revision, by construction: `save` has not run yet,
        // so `revision` is the only value currently valid, and this is not it.
        let stale = Revision(revision.0.wrapping_add(1));
        let err = store.save(&config, Some(stale)).await.unwrap_err();
        let StoreError::RevisionMismatch { expected, actual } = err else {
            panic!("expected RevisionMismatch, got {err:?}");
        };
        assert_eq!(expected, stale);
        assert_eq!(actual, revision);

        // Not just "it errored": the bytes on disk are exactly what they
        // were, not merely a config with the same meaning.
        assert_eq!(std::fs::read(&config_path).unwrap(), before);
    }

    #[tokio::test]
    async fn save_with_expected_against_a_missing_config_file_fails_rather_than_creating_it() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("absent.yaml");
        let store = FileStore::new(config_path.clone(), dir.path().join("templates"));
        let config = load_from_str(GOOD).unwrap();

        let err = store.save(&config, Some(Revision(123))).await.unwrap_err();
        assert!(matches!(err, StoreError::RevisionMismatch { .. }));
        assert!(!config_path.exists());
    }

    // Two concurrent `save` calls that both start from the same revision:
    // exactly one must win and every other one must see `RevisionMismatch`.
    //
    // This does not prove that the two `save` calls truly overlapped in
    // time -- tokio's scheduler and the OS decide the actual interleaving,
    // and `save`'s file lock serialises the calls regardless of how they
    // land. What it does prove is the property the lock exists for: the
    // compare-and-swap check happens under that lock, so no matter how these
    // tasks are interleaved, at most one caller that read the old revision
    // can succeed, and it is never possible for two of them to both
    // "succeed" and silently clobber each other.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_saves_from_the_same_revision_let_exactly_one_through() {
        let dir = tempfile::tempdir().unwrap();
        let (store, _) = store(dir.path());
        let store = std::sync::Arc::new(store);
        let (config, revision) = store.load().await.unwrap();

        let mut handles = Vec::new();
        for i in 0..8 {
            let store = std::sync::Arc::clone(&store);
            let mut config = config.clone();
            config.proxies[0].name = crate::config::Name::parse(format!("renamed-{i}")).unwrap();
            handles.push(tokio::spawn(async move {
                store.save(&config, Some(revision)).await
            }));
        }

        let mut successes = 0;
        let mut mismatches = 0;
        for handle in handles {
            match handle.await.unwrap() {
                Ok(_) => successes += 1,
                Err(StoreError::RevisionMismatch { .. }) => mismatches += 1,
                Err(other) => panic!("unexpected error: {other:?}"),
            }
        }

        assert_eq!(successes, 1);
        assert_eq!(mismatches, 7);
    }

    // -- Locking --------------------------------------------------------------

    #[tokio::test]
    async fn the_lock_file_exists_next_to_the_config_after_a_successful_save() {
        let dir = tempfile::tempdir().unwrap();
        let (store, _) = store(dir.path());
        let (config, _) = store.load().await.unwrap();
        store.save(&config, None).await.unwrap();

        assert!(store.lock_path().exists());
    }

    // The multi-threaded race test above (`concurrent_saves_from_the_same_revision_let_exactly_one_through`)
    // proves the compare-and-swap property but only by timing margin: it
    // does not deterministically prove that `save` actually blocks on the
    // lock file. This test does, by locking `lock_path()` from outside
    // `save` entirely and observing `save` fail to complete while that lock
    // is held, then succeed the instant it is released. That pins both
    // "`save` takes the lock before doing its work" and the underlying
    // flock-per-open-file-description semantics the whole design rests on:
    // a lock taken by one file description (this test's `external_lock`)
    // blocks a second, independently opened description on the same path
    // (the one `save` opens internally), exactly as it would across two
    // separate processes.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn save_does_not_complete_while_the_lock_file_is_held_externally() {
        let dir = tempfile::tempdir().unwrap();
        let (store, _) = store(dir.path());
        let (config, _) = store.load().await.unwrap();

        let lock_path = store.lock_path();
        let external_lock = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(&lock_path)
            .unwrap();
        external_lock.lock().unwrap();

        let store = std::sync::Arc::new(store);
        let task_store = std::sync::Arc::clone(&store);
        let mut handle = tokio::spawn(async move { task_store.save(&config, None).await });

        // Bounded wait: if `save` did not block on the lock, this would
        // resolve almost immediately; if it deadlocked instead of blocking
        // on the OS lock as expected, the final await below would hang
        // instead of this one timing out -- either way the test fails
        // rather than hanging forever.
        let still_running =
            tokio::time::timeout(std::time::Duration::from_millis(200), &mut handle).await;
        assert!(
            still_running.is_err(),
            "save completed while the lock file was held externally"
        );

        drop(external_lock);

        let result = tokio::time::timeout(std::time::Duration::from_secs(5), handle)
            .await
            .expect("save did not complete after the external lock was released")
            .expect("save task panicked");
        result.unwrap();
    }

    #[tokio::test]
    async fn a_second_save_in_the_same_process_succeeds_afterward() {
        let dir = tempfile::tempdir().unwrap();
        let (store, _) = store(dir.path());
        let (config, _) = store.load().await.unwrap();

        store.save(&config, None).await.unwrap();
        // If the first `save` had left the lock held, this would block
        // forever: it does not, because the lock is released when `save`
        // returns.
        store.save(&config, None).await.unwrap();
    }

    #[tokio::test]
    async fn the_lock_file_is_not_the_config_file_and_saving_does_not_clobber_it() {
        let dir = tempfile::tempdir().unwrap();
        let (store, config_path) = store(dir.path());
        let (config, _) = store.load().await.unwrap();
        store.save(&config, None).await.unwrap();

        let lock_path = store.lock_path();
        assert_ne!(lock_path, config_path);
        // `save` only ever opens the lock file to lock it, never writes to
        // it, so it stays empty.
        assert!(std::fs::read(&lock_path).unwrap().is_empty());
        // And the config file was written normally, not replaced by the lock
        // file's (empty) content.
        assert!(
            std::fs::read_to_string(&config_path)
                .unwrap()
                .contains("proxies")
        );
    }

    #[tokio::test]
    async fn templates_round_trip_through_disk() {
        let dir = tempfile::tempdir().unwrap();
        let (store, _) = store(dir.path());

        store
            .save_template("p1", "put.json.j2", b"{{ id }}")
            .await
            .unwrap();
        let files = store.load_templates("p1").await.unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].name, "put.json.j2");
        assert_eq!(files[0].content, b"{{ id }}");

        assert!(dir.path().join("templates/p1/put.json.j2").exists());
    }

    #[tokio::test]
    async fn load_templates_for_an_unknown_proxy_is_empty_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let (store, _) = store(dir.path());
        assert!(store.load_templates("ghost").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn delete_template_reports_whether_it_existed() {
        let dir = tempfile::tempdir().unwrap();
        let (store, _) = store(dir.path());
        store.save_template("p1", "a.j2", b"x").await.unwrap();

        assert!(store.delete_template("p1", "a.j2").await.unwrap());
        assert!(!store.delete_template("p1", "a.j2").await.unwrap());
    }

    #[tokio::test]
    async fn retain_templates_removes_everything_not_kept() {
        let dir = tempfile::tempdir().unwrap();
        let (store, _) = store(dir.path());
        store.save_template("p1", "keep.j2", b"x").await.unwrap();
        store.save_template("p1", "drop.j2", b"y").await.unwrap();

        store
            .retain_templates("p1", &["keep.j2".to_owned()])
            .await
            .unwrap();

        let names: Vec<_> = store
            .load_templates("p1")
            .await
            .unwrap()
            .into_iter()
            .map(|f| f.name)
            .collect();
        assert_eq!(names, vec!["keep.j2".to_owned()]);
    }

    #[tokio::test]
    async fn retain_nothing_removes_the_proxy_directory() {
        let dir = tempfile::tempdir().unwrap();
        let (store, _) = store(dir.path());
        store.save_template("p1", "a.j2", b"x").await.unwrap();

        store.retain_templates("p1", &[]).await.unwrap();
        assert!(!dir.path().join("templates/p1").exists());
    }

    #[tokio::test]
    async fn template_names_are_checked_before_touching_disk() {
        let dir = tempfile::tempdir().unwrap();
        let (store, _) = store(dir.path());
        assert!(matches!(
            store.save_template("p1", "../escape.j2", b"x").await,
            Err(StoreError::BadTemplateName { .. })
        ));
        assert!(!dir.path().join("templates").join("escape.j2").exists());
    }

    #[tokio::test]
    async fn proxy_names_are_checked_too() {
        let dir = tempfile::tempdir().unwrap();
        let (store, _) = store(dir.path());
        assert!(matches!(
            store.save_template("../p1", "a.j2", b"x").await,
            Err(StoreError::BadTemplateName { .. })
        ));
    }

    #[tokio::test]
    async fn retain_templates_deletes_a_foreign_file_sanitize_would_reject() {
        let dir = tempfile::tempdir().unwrap();
        let (store, _) = store(dir.path());
        store.save_template("p1", "keep.j2", b"x").await.unwrap();
        std::fs::write(dir.path().join("templates/p1/.DS_Store"), b"junk").unwrap();

        store
            .retain_templates("p1", &["keep.j2".to_owned()])
            .await
            .unwrap();

        assert!(!dir.path().join("templates/p1/.DS_Store").exists());
        assert!(dir.path().join("templates/p1/keep.j2").exists());
    }

    #[tokio::test]
    async fn load_templates_does_not_report_a_foreign_file() {
        let dir = tempfile::tempdir().unwrap();
        let (store, _) = store(dir.path());
        store.save_template("p1", "real.j2", b"x").await.unwrap();
        std::fs::write(dir.path().join("templates/p1/.DS_Store"), b"junk").unwrap();

        let names: Vec<_> = store
            .load_templates("p1")
            .await
            .unwrap()
            .into_iter()
            .map(|f| f.name)
            .collect();
        assert_eq!(names, vec!["real.j2".to_owned()]);
    }

    // A directory entry whose file name is not valid UTF-8 cannot be
    // fixtured end to end on every filesystem this suite might run on:
    // APFS (macOS, this workspace's own platform) and most Windows
    // filesystems reject a non-UTF-8 name at file-creation time, so there is
    // no on-disk state to point `load_templates`/`retain_templates` at in
    // the first place on those platforms -- only Linux filesystems such as
    // ext4, which store names as opaque bytes, would let such a fixture
    // exist. `decode_template_name` is the whole of the fix for both
    // methods (see its call sites above), so it is unit-tested directly
    // instead: the `OsString` here is built in memory with `OsStrExt`,
    // never written to disk, sidestepping the filesystem's own opinion of
    // what a valid name is.
    #[test]
    #[cfg(unix)]
    fn decode_template_name_rejects_non_utf8_rather_than_mangling_it() {
        use std::os::unix::ffi::OsStrExt;
        let name = std::ffi::OsStr::from_bytes(b"bad-\xff-name.j2").to_owned();
        assert_eq!(FileStore::decode_template_name(name), None);
    }

    #[test]
    fn decode_template_name_accepts_a_normal_name() {
        assert_eq!(
            FileStore::decode_template_name(std::ffi::OsString::from("real.j2")),
            Some("real.j2".to_owned())
        );
    }
}
