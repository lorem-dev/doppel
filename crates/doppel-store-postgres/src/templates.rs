//! Template files: rows in the database, mirrored onto disk.
//!
//! The database is the source of truth and the directory is a cache. The
//! render path reads a template from the filesystem at request time, and
//! turning that into a database round trip would put a query on the hot path
//! of every mocked response.
//!
//! So every write here touches both, in that order, and `materialize` brings a
//! directory into line with the rows -- which is what a second instance needs
//! after its peer uploaded something.

use std::path::{Path, PathBuf};

use doppel_core::store::name::sanitize;
use doppel_core::store::{StoreError, TemplateFile};
use sqlx::Row;

use crate::PostgresStore;

impl PostgresStore {
    /// Where the mirror lives. A proxy name becomes a path component, so it
    /// gets the same check a file name does.
    fn proxy_dir(&self, proxy: &str) -> Result<PathBuf, StoreError> {
        Ok(self.templates_dir().join(sanitize(proxy)?))
    }

    pub(crate) async fn load_template_rows(
        &self,
        proxy: &str,
    ) -> Result<Vec<TemplateFile>, StoreError> {
        // Ordered by name, matching `FileStore`, which sorts its listing. A
        // caller comparing the two stores must not see a difference that is
        // only about iteration order.
        let rows = sqlx::query(
            "SELECT file, content FROM templates WHERE config = $1 AND proxy = $2 ORDER BY file",
        )
        .bind(self.config_name())
        .bind(proxy)
        .fetch_all(self.pool())
        .await
        .map_err(failed)?;

        rows.iter()
            .map(|row| {
                Ok(TemplateFile {
                    name: row.try_get("file").map_err(failed)?,
                    content: row.try_get("content").map_err(failed)?,
                })
            })
            .collect()
    }

    pub(crate) async fn save_template_row(
        &self,
        proxy: &str,
        file: &str,
        bytes: &[u8],
    ) -> Result<(), StoreError> {
        let file = sanitize(file)?;
        sqlx::query(
            "INSERT INTO templates (config, proxy, file, content) VALUES ($1, $2, $3, $4) \
             ON CONFLICT (config, proxy, file) DO UPDATE SET content = $4",
        )
        .bind(self.config_name())
        .bind(proxy)
        .bind(file)
        .bind(bytes)
        .execute(self.pool())
        .await
        .map_err(failed)?;

        // The mirror second, so a failure to write the file leaves the row
        // that a later `materialize` will replay. The other order would report
        // success for a template the database never received.
        let dir = self.proxy_dir(proxy)?;
        std::fs::create_dir_all(&dir).map_err(io(&dir))?;
        let path = dir.join(file);
        std::fs::write(&path, bytes).map_err(io(&path))
    }

    pub(crate) async fn delete_template_row(
        &self,
        proxy: &str,
        file: &str,
    ) -> Result<bool, StoreError> {
        let file = sanitize(file)?;
        let existed =
            sqlx::query("DELETE FROM templates WHERE config = $1 AND proxy = $2 AND file = $3")
                .bind(self.config_name())
                .bind(proxy)
                .bind(file)
                .execute(self.pool())
                .await
                .map_err(failed)?
                .rows_affected()
                > 0;

        let path = self.proxy_dir(proxy)?.join(file);
        match std::fs::remove_file(&path) {
            Ok(()) | Err(_) if !path.exists() => {}
            Err(source) => return Err(StoreError::Io { path, source }),
            Ok(()) => {}
        }

        // Whether the *row* existed, not whether the file did. The row is the
        // truth, and a mirror that had drifted must not change the answer.
        Ok(existed)
    }

    pub(crate) async fn retain_template_rows(
        &self,
        proxy: &str,
        keep: &[String],
    ) -> Result<(), StoreError> {
        // `<> ALL(...)` rather than `NOT IN`, and one statement rather than a
        // read followed by deletes: an empty `keep` then needs no special case
        // in SQL, because nothing is equal to all members of an empty set.
        sqlx::query("DELETE FROM templates WHERE config = $1 AND proxy = $2 AND file <> ALL($3)")
            .bind(self.config_name())
            .bind(proxy)
            .bind(keep)
            .execute(self.pool())
            .await
            .map_err(failed)?;

        if keep.is_empty() {
            let dir = self.proxy_dir(proxy)?;
            return match std::fs::remove_dir_all(&dir) {
                Ok(()) => Ok(()),
                Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(source) => Err(StoreError::Io { path: dir, source }),
            };
        }

        self.mirror_proxy(proxy, keep).await
    }

    /// Bring `dir` into line with the rows, for every proxy the store holds.
    ///
    /// Called on reload, so a second instance picks up a template its peer
    /// uploaded on the same reload that brings it the configuration.
    pub(crate) async fn materialize(&self) -> Result<(), StoreError> {
        let proxies: Vec<String> =
            sqlx::query_scalar("SELECT DISTINCT proxy FROM templates WHERE config = $1")
                .bind(self.config_name())
                .fetch_all(self.pool())
                .await
                .map_err(failed)?;

        for proxy in &proxies {
            let files = self.load_template_rows(proxy).await?;
            let names: Vec<String> = files.iter().map(|file| file.name.clone()).collect();
            let dir = self.proxy_dir(proxy)?;
            std::fs::create_dir_all(&dir).map_err(io(&dir))?;
            for file in &files {
                let path = dir.join(&file.name);
                std::fs::write(&path, &file.content).map_err(io(&path))?;
            }
            self.mirror_proxy(proxy, &names).await?;
        }
        Ok(())
    }

    /// Remove mirrored files the database does not have.
    ///
    /// Walks the directory rather than going through `delete_template_row`,
    /// for the same reason `FileStore::retain_templates` does: every name here
    /// came from a listing, so it is already a single component inside `dir`,
    /// and re-checking it would leave foreign files such as `.DS_Store`
    /// undeletable and break the "nothing but `keep` survives" contract.
    async fn mirror_proxy(&self, proxy: &str, keep: &[String]) -> Result<(), StoreError> {
        let dir = self.proxy_dir(proxy)?;
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(source) => return Err(StoreError::Io { path: dir, source }),
        };
        for entry in entries {
            let entry = entry.map_err(io(&dir))?;
            if !entry.file_type().map_err(io(&dir))?.is_file() {
                continue;
            }
            let keep_this = entry
                .file_name()
                .into_string()
                .ok()
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

fn failed(err: sqlx::Error) -> StoreError {
    StoreError::Unavailable(format!("template query failed: {err}"))
}

fn io(path: &Path) -> impl Fn(std::io::Error) -> StoreError + '_ {
    move |source| StoreError::Io {
        path: path.to_path_buf(),
        source,
    }
}
