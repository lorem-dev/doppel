# Doppel

Doppel is a CLI-driven HTTP reverse proxy. The name is from "doppelganger": it
stands in front of, or in place of, a real backend, so clients of that backend
can be developed and tested against a realistic, deliberately degraded, or
entirely absent upstream. It forwards traffic to a configured upstream,
injects controlled latency and loss, and (in a later phase) replaces selected
endpoints with templated mock responses.

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

## What it does not do yet

- No mocked responses and no Jinja2 templating (phase 2).
- No admin HTTP API: proxies cannot be created, read, updated or deleted over
  HTTP, and there is no Swagger document for one yet (phase 3).
- No Prometheus metrics and no Sentry integration (phase 3).
- No PostgreSQL-backed configuration store; configuration lives in a single
  YAML file on disk (phase 4).
- No published documentation site; this README and `AGENTS.md` are what
  exists until the mkdocs build lands (phase 5).

See `.superpowers/specs/` for the full design if you have access to it; it is
git-ignored and not shipped with the repository.

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

`admin` is required to parse even though phase 1 does not yet serve it; see
`main.example.yaml` at the repository root for a fuller reference covering
tokens, access control, and fault injection.

## Where the crates live

A Cargo workspace under `crates/`; each crate owns one responsibility, and
dependencies point one way, into `doppel-core`:

| Crate | Owns |
|---|---|
| `doppel-core` | Configuration model, YAML loading, validation, the `ConfigStore` trait and its file-backed implementation, the compiled runtime, the error model. |
| `doppel-proxy` | The proxy listener, request resolution, fault injection, and upstream forwarding. |
| `doppel-telemetry` | Logging initialization. |
| `doppel-cli` | The `doppel` binary: argument parsing, the control channel, and wiring the other three crates together. |

Two more crates, `doppel-render` (mock templating) and `doppel-admin` (the
admin HTTP API), are planned for phase 2 and phase 3 respectively and do not
exist in the workspace yet.
