# Changes

Entries are written as part of the change that causes them, not reconstructed
afterwards. The `check-changes` skill verifies that.

The Development section collects everything since the last release. Cutting a
release promotes it to a version heading; the `bump-version` skill does that.

## Development

### Added

- Configuration model for the whole service -- server, logging, control
  channel, templates, Sentry, admin, and proxies with their mocks -- loaded
  from YAML with unknown keys rejected rather than ignored.
- Semantic validation, thirty-three rules, reported together with the config
  path that produced each one rather than one failure at a time.
- `FileStore`: configuration on disk, written atomically, guarded by an
  advisory lock held across processes, with compare-and-swap on a
  content-derived revision.
- HTTP reverse proxying with per-proxy upstreams, header injection,
  hop-by-hop hygiene in both directions, and streaming request and response
  bodies.
- Fault injection: latency and loss, per proxy and overridable per mock.
- Proxy resolution by request header, with a default fallback.
- The mock engine: request matching, variables extracted from the path,
  headers, query and body, and responses rendered with minijinja.
- A Unix socket control channel and `doppel config reload`, applying a new
  configuration without dropping in-flight traffic.
- Structured logging to stdout, JSON or text.
- The `doppel` binary: `serve`, `config validate`, `config reload`, `version`.

### Notes

- Phase 1 and phase 2 of the plan are complete. The admin HTTP API, its
  Prometheus metrics and Swagger, template upload, the PostgreSQL store and
  TCP proxying are not implemented yet; the configuration accepts and
  validates the settings they will use.
