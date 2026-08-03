# Changes

Entries are written as part of the change that causes them, not reconstructed
afterwards. The `check-changes` skill verifies that.

The Development section collects everything since the last release. Cutting a
release promotes it to a version heading; the `bump-version` skill does that.

## Development

## 0.2.0 -- 2026-08-03

### Changed

- A matching mock is now decided before `loss` and `latency`, not after, so
  `replace` is the share of matching requests a mock answers rather than the
  share of those that survived a loss roll. Previously `loss: 0.5` halved every
  `replace` in the proxy, and no configuration could ask a mock to answer half
  of its matching requests while any loss was set. The proxy's `loss` and
  `latency` no longer apply to a request a mock answered: they describe the real
  backend, and a mock replaces it.
- A run of slashes at the start of a request path is collapsed to one before
  mocks are matched, so `//api/v1/index/` matches a mock declared
  `^/api/v1/index/$`. Clients produce the doubled form by joining a base URL
  ending in `/` to a path beginning with `/`; it is legal HTTP, nothing rejected
  it, and the only symptom was an anchored mock silently not firing. Empty
  segments elsewhere in the path are left alone.
- An injected `latency` is now a target for the whole response rather than an
  addition to it: the time the upstream really took is subtracted, and only the
  remainder is waited out. A 500ms latency in front of a backend answering in
  120ms delays by 380ms, where before it delayed by 500 and produced 620ms
  total -- so the number written in the configuration was unreachable by
  construction, and moved with whatever the upstream happened to be doing. An
  upstream slower than the target leaves no remainder and is passed straight
  through; the setting is a floor, never a ceiling. `latency_injected_ms` in the
  log line is the wait actually taken and reads `0` in that case, while
  `doppel_latency_injected_total` still counts the request.

### Fixed

- A mock's `proxy.loss` and `proxy.latency` are applied. They were parsed,
  validated and compiled into the runtime, and then never read, so a mock
  declaring either was silently answering every request it matched. They now
  apply to the requests the mock answers, after it has won its `replace` roll,
  and go through the same `decide` as the proxy's -- so loss short-circuits
  latency there too.
- What a mock inherits from its proxy is now settled per setting rather than by
  accident: `replace` and `latency` fall back to the proxy's, `loss` does not.
  `latency` describes how slow the proxy is to answer, which holds whatever
  answers, so a mocked response is delayed like any other and a mock's own value
  overrides rather than adds to it. `loss` is excluded because a mock inheriting
  it would be dropped by the proxy's loss, which is the coupling between `loss`
  and `replace` the ordering above exists to remove.

## 0.1.0 -- 2026-08-02

The first release. Everything below is new, so the sections are what a reader
arriving at 0.1.0 needs rather than a diff against something earlier.

### Proxying

- HTTP reverse proxying with per-proxy upstreams, header injection, hop-by-hop
  hygiene in both directions, and streaming request and response bodies, so a
  transfer larger than memory passes through.
- The upstream is confined: a proxy configured for one upstream and one base
  path can only reach paths under it. Enforced by checking the built URL, not
  by sanitising the input -- two attempts at the latter were bypassed, once by
  an absolute-URI request target and once by percent-encoded traversal.
- Proxy resolution by request header, with an optional default. Several proxies
  share one port; resolution headers are tried in configuration order, so the
  same request resolves the same way on every process and every run.
- Fault injection: latency and loss, per proxy and overridable per mock. Loss
  is decided first and short-circuits -- a request being dropped is not delayed
  first.
- Mocks: matching on method and an unanchored path pattern, variables taken
  from path captures, headers, the query string and a JSON body, and responses
  rendered with minijinja. `replace` decides what share of matching requests a
  mock answers, so a backend can be replaced gradually.
- One fixed error envelope on every failure path, including the ones no
  handler produces -- an unknown route, a wrong method, a body over the limit.

### Configuration

- A configuration model for the whole service, loaded from YAML with unknown
  keys rejected rather than ignored.
- Values are parsed into types rather than validated afterwards: names, tokens,
  ports, statuses, methods, probabilities, durations, byte sizes, upstream
  URLs, header names and values, mock patterns, selectors and template file
  names each refuse bad input while the document is being read. Sixteen
  validation rules remain, for what needs more than one field to decide.
- Reload without dropping in-flight traffic: a new configuration is loaded,
  validated and compiled before anything is swapped, and a failure leaves the
  running one untouched. Sections whose listener is already bound are reported
  as needing a restart rather than silently ignored.
- Two stores behind one trait, chosen by `--store`: a YAML file, or PostgreSQL.
  One conformance suite runs against both. `config push` and `config pull` move
  a configuration between them.
- Compare-and-swap on a content-derived revision, so two writers cannot
  overwrite each other unnoticed. The same document always produces the same
  revision, on any machine.

### Administration

- An admin HTTP API on a second port: proxy CRUD with per-proxy optimistic
  concurrency, template upload, reload, status, Prometheus metrics and a
  generated Swagger UI, behind token access control. It can be turned off
  entirely with `admin.enable`.
- Every request is judged against the configuration the last reload put into
  force, never against what the store holds at that moment -- so someone who
  can write the configuration out of band, but holds no token, cannot grant
  themselves one and use it.
- Admin tokens can come from `DOPPEL_ADMIN_TOKENS` instead of the document.
  They take precedence over a configured token of the same name, and a
  malformed variable fails startup rather than being skipped.
- `doppel token add` issues a token on a running server over the control
  socket: generated, stored, brought into force before the command answers,
  and printed once.
- Metrics carry `proxy`, `method` and `status` labels and deliberately no
  `path` -- a path label is attacker-controlled cardinality.

### Operating

- The `doppel` binary: `serve`, `config validate`, `config reload`,
  `config push`, `config pull`, `config migrate`, `token add`, `version`.
- Migrations are embedded in the binary and applied only by
  `config migrate`, never at startup. `config migrate --status` reports what
  is applied, complete and unchanged without altering anything, and exits
  non-zero when the schema is behind.
- Structured logging to stdout, JSON or text. Optional Sentry reporting behind
  a cargo feature.
- Graceful shutdown on SIGTERM, draining in-flight requests.

### Distribution

- Prebuilt binaries for macOS on Apple Silicon and Linux on x86-64 and arm64,
  with SHA-256 checksums signed by the Lorem Dev release key.
- A one-line installer that verifies the checksum and refuses to install on a
  mismatch.
- An Alpine image on Docker Hub as `loremdev/doppel`, for linux/amd64 and
  linux/arm64, tagged `<version>-alpine`.

### Notes

- TCP proxying is not implemented. A `type: tcp` proxy is rejected at load with
  a message saying so, rather than being quietly ignored.
- Before 1.0 a breaking change is a minor bump, which is what `0.x` semantics
  allow.

