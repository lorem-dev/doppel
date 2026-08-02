//! Configuration storage. `FileStore` here, `PostgresStore` in phase 4.

pub mod file;
pub mod name;

use std::path::PathBuf;

use crate::config::ProxyConfig;
use crate::validate::Violation;

/// An identity marker derived from a configuration's own content: two
/// configurations with the same canonical serialization have the same
/// revision, regardless of which store loaded or saved them, or how many
/// times. It is not a counter, and it is not a cryptographic checksum: it
/// answers "is this the same configuration?", never "has anyone tampered
/// with this file?".
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Revision(pub u64);

impl Revision {
    /// The whole-configuration revision, returned by `ConfigStore::load` and
    /// `ConfigStore::save`. Two configurations that serialize to the same
    /// canonical YAML always produce the same revision, regardless of which
    /// process computed it or how the file was formatted, so several Doppel
    /// instances sharing one configuration agree on what they are running,
    /// and reformatting the file or editing a comment does not look like a
    /// change.
    #[must_use]
    pub fn of_config(config: &crate::Config) -> Self {
        // Nothing in this codebase builds a `Config` except deserialisation
        // (see `load_from_str`/`load_from_path`), and everything serde
        // deserializes into these field types re-serializes cleanly -- with
        // one caveat: `control.socket` and `templates.dir` are `PathBuf`s,
        // and serde's YAML serializer fails on a `PathBuf` that is not valid
        // UTF-8. That failure is unreachable today only because nothing
        // constructs a `Config` by hand with such a path; it is not
        // impossible in principle the way "a Config value always
        // serializes" claimed. If a caller ever does build one directly,
        // this `expect` is the assertion that catches it, rather than
        // silently handing out a wrong revision.
        let yaml = crate::config::canonical_yaml(config)
            .expect("a Config produced by deserialization always serializes back to YAML");
        Self(fnv1a(yaml.as_bytes()))
    }

    /// The per-proxy revision. Exposed by the admin API (phase 3) when
    /// listing proxies and when reading one, and required from the client
    /// when updating that proxy: it is the finer-grained counterpart to
    /// `of_config`, used so a concurrent edit to a different proxy does not
    /// look like a conflict on this one.
    #[must_use]
    pub fn of_proxy(proxy: &ProxyConfig) -> Self {
        // Same reasoning as `of_config`, and the same helper, so the two can
        // never spell "canonical serialization" differently.
        let yaml = crate::config::canonical_yaml(proxy)
            .expect("a ProxyConfig produced by deserialization always serializes back to YAML");
        Self(fnv1a(yaml.as_bytes()))
    }
}

/// A stable, non-cryptographic 64-bit hash (FNV-1a), used by both
/// `Revision::of_config` and `Revision::of_proxy` to turn a canonical YAML
/// serialization into a revision.
///
/// `std::collections::hash_map::DefaultHasher` is deliberately not used
/// here: its output is explicitly not guaranteed stable across Rust
/// versions, which would silently break the "same config, same revision"
/// property on a toolchain upgrade. FNV-1a has no such caveat.
fn fnv1a(bytes: &[u8]) -> u64 {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET_BASIS;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("config not found: {0}")]
    NotFound(PathBuf),
    #[error("config is invalid")]
    Invalid(Vec<Violation>),
    #[error("io error on {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("serialization failed: {0}")]
    Serialize(String),
    #[error("template name `{name}` rejected: {reason}")]
    BadTemplateName { name: String, reason: String },
    #[error("revision mismatch: expected {expected:?}, actual {actual:?}")]
    RevisionMismatch {
        expected: Revision,
        actual: Revision,
    },
}

pub use file::FileStore;

/// One template file, as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateFile {
    pub name: String,
    pub content: Vec<u8>,
}

/// Where configuration lives. `FileStore` implements it now; `PostgresStore`
/// implements it in phase 4 without any change to callers.
#[async_trait::async_trait]
pub trait ConfigStore: Send + Sync {
    /// Load and validate the configuration, returning it alongside the
    /// revision that identifies its content.
    async fn load(&self) -> Result<(crate::Config, Revision), StoreError>;

    /// Validate and persist the configuration, returning the revision that
    /// identifies its content.
    ///
    /// `expected` is a compare-and-swap token: `None` means an unconditional
    /// write (first-time provisioning, `config push`); `Some(rev)` means the
    /// caller built its change from revision `rev`, and the store must fail
    /// with `StoreError::RevisionMismatch` rather than write anything if the
    /// stored configuration's current revision differs. The check and the
    /// write happen under the same lock, so this is an actual compare-and-
    /// swap and not a comparison racing an unrelated write.
    ///
    /// There is no `actor` parameter: an audit trail needs somewhere to put
    /// an actor, and phase 1 has none, so accepting and discarding one here
    /// would be a hook with nothing attached. Phase 3 can add it back
    /// alongside the audit log that gives it meaning.
    async fn save(
        &self,
        config: &crate::Config,
        expected: Option<Revision>,
    ) -> Result<Revision, StoreError>;

    /// Every template file belonging to a proxy. An unknown proxy yields an
    /// empty list rather than an error: having no templates is normal.
    async fn load_templates(&self, proxy: &str) -> Result<Vec<TemplateFile>, StoreError>;

    async fn save_template(&self, proxy: &str, file: &str, bytes: &[u8]) -> Result<(), StoreError>;

    /// Returns whether the file existed.
    async fn delete_template(&self, proxy: &str, file: &str) -> Result<bool, StoreError>;

    /// Drop every template for `proxy` except those named in `keep`. An empty
    /// `keep` removes the proxy's storage entirely.
    async fn retain_templates(&self, proxy: &str, keep: &[String]) -> Result<(), StoreError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ByteSize, ProxyKind, ResolveConfig};

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

    #[test]
    fn revision_of_config_agrees_across_a_save_and_load_round_trip_for_both_public_shapes() {
        // `Subjects::Names(vec![])` and `Subjects::Names(vec!["public"])`
        // both mean the same thing as `Subjects::Public`, and both parse
        // back as `Public`. Before the `Serialize` fix this test guards,
        // `to_yaml` on either shape produced something other than
        // `"public"`, so a `Revision` computed directly from an in-memory
        // `Config` (as `save` does) disagreed with the `Revision` computed
        // after parsing that same config back out of YAML (as `load`
        // does) -- two representations of one configuration that could
        // never compare-and-swap against each other.
        for names in [Vec::new(), vec!["public".to_owned()]] {
            let mut config = crate::config::load_from_str(GOOD).unwrap();
            config.admin.access.read = crate::config::Subjects::Names(names.clone());

            let direct = Revision::of_config(&config);
            let yaml = crate::config::to_yaml(&config).unwrap();
            let reparsed = crate::config::load_from_str(&yaml).unwrap();
            let round_tripped = Revision::of_config(&reparsed);

            assert_eq!(
                direct, round_tripped,
                "Names({names:?}) disagreed with its own round trip"
            );
        }
    }

    fn proxy(url: &str) -> ProxyConfig {
        ProxyConfig {
            name: "p1".to_owned(),
            kind: ProxyKind::Http,
            url: url.to_owned(),
            timeout: None,
            resolve: ResolveConfig::default(),
            access: None,
            headers: Default::default(),
            loss: None,
            latency: None,
            replace: None,
            body_limit: ByteSize(1024 * 1024),
            mocks: Vec::new(),
        }
    }

    #[test]
    fn of_proxy_agrees_for_structurally_identical_proxies() {
        let a = proxy("https://example.com/");
        let b = proxy("https://example.com/");
        assert_eq!(Revision::of_proxy(&a), Revision::of_proxy(&b));
    }

    #[test]
    fn of_proxy_changes_when_a_field_changes() {
        let a = proxy("https://example.com/");
        let b = proxy("https://example.org/");
        assert_ne!(Revision::of_proxy(&a), Revision::of_proxy(&b));
    }
}
