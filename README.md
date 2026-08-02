<h1 align="center">
  <img src="/docs/assets/logo.svg" width="24" alt="Doppel Logo"> Doppel
</h1>

<p align="center">
    Doppel is a CLI-driven HTTP reverse proxy. The name is from "doppelganger": it
    stands in front of, or in place of, a real backend, so clients of that backend
    can be developed and tested against a realistic, deliberately degraded, or
    entirely absent upstream. It forwards traffic to a configured upstream,
    injects controlled latency and loss, and (in a later phase) replaces selected
    endpoints with templated mock responses.
</p>

---

## What phase 1 does

- Forwards HTTP requests to a configured upstream, streaming both request and
  response bodies rather than buffering them.
- Resolves which of several configured proxies handles a request, either by a
  header value or by falling back to a configured default.
- Injects configured latency and loss (dropped requests answered with a
  configured status), independently per proxy.
- Loads, validates and reloads its YAML configuration from disk, and reloads
  a running server's configuration over a local control socket
  (`doppel config reload`) without a restart.
- Logs one line per request to stdout, in JSON or human-readable text.
- Serves an admin HTTP API on a second port: proxy CRUD with optimistic
  concurrency, template upload, reload, a status endpoint, Prometheus metrics
  and a generated Swagger UI, behind token access control. See
  [the documentation](docs/admin-api.md).
- Reports errors to Sentry, optionally, behind the `sentry` cargo feature.

## What it does not do yet

- No PostgreSQL-backed configuration store; configuration lives in a single
  YAML file on disk (phase 4).
- No TCP proxying. A `type: tcp` proxy is rejected at load with a message
  saying so, rather than being quietly ignored.

Mocked responses and Jinja2 templating are implemented -- see
[the documentation](docs/mocks.md).

## Documentation

The full documentation lives in `docs/` and builds with mkdocs and the Material
theme. Python tooling here is driven by `uv`, never `pip`:

```bash
uv run --with-requirements docs/requirements.txt mkdocs serve
```

`.superpowers/specs/` holds the design documents; that directory is git-ignored
and not shipped.

## Building and running

Prerequisites: a stable Rust toolchain via rustup, edition 2024 (see
`Cargo.toml` for the pinned `rust-version`).

```bash
cargo build --release
cp main.example.yaml main.yaml   # edit to taste; main.yaml is git-ignored
./target/release/doppel serve --config main.yaml
```

`cargo run -- <args>` works the same way during development.

## Commands

- `doppel serve` -- run the proxy. Binds `server.host:server.port` for
  traffic and a Unix domain socket at `control.socket` for the reload
  channel. Runs until it receives a shutdown signal, draining in-flight
  requests first.
- `doppel config validate` -- parse and validate the configuration, printing
  every violation found (one per line, with its config path) and exiting
  non-zero if there is one.
- `doppel config reload` -- ask a running server (over `control.socket`) to
  reload its configuration from disk, and report the resulting revision and
  proxy count, or the violations that made it refuse the new configuration.

Configuration path, store selection and similar are set with flags or their
`DOPPEL_*` environment equivalents; run any command with `--help` for the
full list.

## A minimal configuration

```yaml
server:
  host: "0.0.0.0"
  port: 8080

admin:
  host: "0.0.0.0"
  port: 8081
  tokens: []
  access: {}
  upload:
    limit: 1M

proxies:
  - name: backend
    type: http
    url: "https://example.com/"
    resolve:
      type: default
```

See `main.example.yaml` at the repository root for a fuller reference covering
tokens, access control, and fault injection. Note that every admin action --
reads included -- defaults to the `admin` group: a proxy document carries the
headers that proxy injects upstream, so a public listing would publish them.

## Where the crates live

A Cargo workspace under `crates/`; each crate owns one responsibility, and
dependencies point one way, into `doppel-core`:

| Crate | Owns |
|---|---|
| `doppel-core` | Configuration model, YAML loading, validation, the `ConfigStore` trait and its file-backed implementation, the compiled runtime, the error model. |
| `doppel-proxy` | The proxy listener, request resolution, fault injection, and upstream forwarding. |
| `doppel-render` | Mock matching and Jinja2 rendering. |
| `doppel-admin` | The admin HTTP API: access control, proxy CRUD, templates, reload, status, metrics, OpenAPI. |
| `doppel-telemetry` | Logging initialization and optional Sentry. |
| `doppel-cli` | The `doppel` binary: argument parsing, the control channel, and wiring the other crates together. |
