//! The YAML file implementation of `ConfigStore`.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::{ConfigStore, Revision, StoreError, TemplateFile, name::sanitize};
use crate::config::{self, Config, ConfigError};
use crate::validate::validate;

pub struct FileStore {
    config_path: PathBuf,
    templates_dir: PathBuf,
    revision: AtomicU64,
}

impl FileStore {
    /// `templates_dir` is passed in rather than read from the config, so that
    /// template operations work before the first successful load.
    pub fn new(config_path: impl Into<PathBuf>, templates_dir: impl Into<PathBuf>) -> Self {
        Self {
            config_path: config_path.into(),
            templates_dir: templates_dir.into(),
            revision: AtomicU64::new(0),
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
}

#[async_trait::async_trait]
impl ConfigStore for FileStore {
    async fn load(&self) -> Result<Config, StoreError> {
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
        Ok(config)
    }

    async fn save(&self, config: &Config, _actor: Option<&str>) -> Result<Revision, StoreError> {
        validate(config).map_err(StoreError::Invalid)?;
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

        Ok(Revision(self.revision.fetch_add(1, Ordering::SeqCst) + 1))
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
            let name = entry.file_name().to_string_lossy().into_owned();
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
        for file in self.load_templates(proxy).await? {
            if !keep.contains(&file.name) {
                self.delete_template(proxy, &file.name).await?;
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
    limit: 1M
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
        let config = store.load().await.unwrap();
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
    async fn save_round_trips_and_bumps_the_revision() {
        let dir = tempfile::tempdir().unwrap();
        let (store, config_path) = store(dir.path());
        let mut config = store.load().await.unwrap();
        config.proxies[0].name = "renamed".to_owned();

        let first = store.save(&config, Some("tester")).await.unwrap();
        let second = store.save(&config, Some("tester")).await.unwrap();
        assert_eq!(first, Revision(1));
        assert_eq!(second, Revision(2));

        let reloaded = load_from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
        assert_eq!(reloaded.proxies[0].name, "renamed");
    }

    #[tokio::test]
    async fn save_refuses_an_invalid_config_and_leaves_the_file_alone() {
        let dir = tempfile::tempdir().unwrap();
        let (store, config_path) = store(dir.path());
        let mut config = store.load().await.unwrap();
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

        let before = store.load().await.unwrap();
        store.save(&before, None).await.unwrap();
        let after = store.load().await.unwrap();
        assert_eq!(before, after);
    }

    #[tokio::test]
    async fn save_leaves_no_temporary_files_behind() {
        let dir = tempfile::tempdir().unwrap();
        let (store, _) = store(dir.path());
        let config = store.load().await.unwrap();
        store.save(&config, None).await.unwrap();

        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|name| name != "main.yaml")
            .collect();
        assert!(leftovers.is_empty(), "unexpected leftovers: {leftovers:?}");
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
}
