<p align="center">
    <img src="/assets/icon.svg" width="64" alt="Doppel Logo">
</p>

<h1 align="center">$$Doppel{\color{lightblue}ganger}$$</h1>

<p align="center">
    <a href="https://github.com/lorem-dev/doppel/releases/latest"><img src="https://img.shields.io/github/v/release/lorem-dev/doppel?label=download" alt="Download"></a>
    <a href="https://lorem-dev.github.io/doppel/"><img src="https://img.shields.io/badge/docs-online-blue" alt="Documentation"></a>
    <a href="./LICENSE"><img src="https://img.shields.io/github/license/lorem-dev/doppel" alt="License"></a>
    <a href="https://github.com/lorem-dev/doppel/actions/workflows/ci.yml"><img src="https://img.shields.io/badge/coverage-92%25-brightgreen" alt="Coverage"></a>
    <a href="https://github.com/lorem-dev/doppel/actions/workflows/ci.yml"><img src="https://github.com/lorem-dev/doppel/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
</p>

<p align="center">
    Doppel is a CLI-driven HTTP reverse proxy. The name is from "doppelganger": it
    stands in front of, or in place of, a real backend, so clients of that backend
    can be developed and tested against a realistic, deliberately degraded, or
    entirely absent upstream. It forwards traffic to a configured upstream,
    injects controlled latency and loss, and replaces selected endpoints with
    templated mock responses.
</p>

---

## Installing

```bash
curl -fsSL https://raw.githubusercontent.com/lorem-dev/doppel/main/scripts/install.sh | sh
```

Prebuilt binaries are published for macOS on Apple Silicon and Linux on x86-64
and arm64. Anywhere else, `cargo install --path crates/doppel-cli`. See
[the documentation](https://lorem-dev.github.io/doppel/usage/installation/).

Or run the image:

```bash
docker run --rm -p 8080:8080 -p 8081:8081 \
  -v "$PWD/config:/etc/doppel" \
  loremdev/doppel:1.2.3-alpine
```

Published for `linux/amd64` and `linux/arm64`, with no `latest` tag -- pin the
version. See
[Running in Docker](https://lorem-dev.github.io/doppel/usage/docker/).

## What it does

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
  [the documentation](https://lorem-dev.github.io/doppel/usage/admin-api/).
- Serves a browser dashboard from that same port's root, compiled into the
  binary: the proxy list, a form over the whole proxy document, mock templates,
  status and reload. It is a client of the admin API and bound by the same token
  rules, so it offers only what the caller may actually do -- and works without a
  token wherever the API does. `admin.dashboard: false` turns it off. See
  [the documentation](https://lorem-dev.github.io/doppel/usage/dashboard/).
- Reports errors to Sentry, optionally, behind the `sentry` cargo feature.
- Stores its configuration in a YAML file or in PostgreSQL, selected by
  `--store`, with `config push` and `config pull` to move a configuration
  between the two. See
  [the documentation](https://lorem-dev.github.io/doppel/usage/storage/).

## What it does not do yet

- No TCP proxying. A `type: tcp` proxy is rejected at load with a message
  saying so, rather than being quietly ignored.

Mocked responses and Jinja2 templating are implemented -- see
[the documentation](https://lorem-dev.github.io/doppel/usage/mocks/).

## Documentation

The full documentation is published at
<https://lorem-dev.github.io/doppel/>. Its source is `docs/`, and it builds
with mkdocs and the Material theme; Python tooling here is driven by `uv`,
never `pip`:

```bash
uv run --with-requirements docs/requirements.txt mkdocs serve
```

The dashboard's source is `frontend/`; building it needs Node, and the binary
embeds whatever `frontend/dist` holds at compile time:

```bash
npm --prefix frontend ci && npm --prefix frontend run build
cargo build --release -p doppel-cli
```

Skipping that leaves a working binary whose root answers 503 and says so.

The workspace layout -- the crates, what each owns, and the dependency
direction -- is in
[the architecture page](https://lorem-dev.github.io/doppel/development/architecture/).

`.superpowers/specs/` holds the design documents; that directory is git-ignored
and not shipped.

## Building and running

Prerequisites: a stable Rust toolchain via rustup, edition 2024 (see
`Cargo.toml` for the pinned `rust-version`).

```bash
make release                     # the dashboard, then the binary
mkdir -p config && cp main.example.yaml config/main.yaml   # config/ is git-ignored
./target/release/doppel serve --config config/main.yaml
```

`make help` lists every target: `make gate` runs the whole check suite,
`make image-rebuild` builds the container image from scratch, `make docs-serve`
serves the documentation. `cargo build --release` works too, and skips the
dashboard -- which then answers 503 at the admin root and says so.

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

`DOPPEL_EXTERNAL_URL` names the address clients reach Doppel at, for rewriting an
upstream's redirects. Needed behind a port mapping or an ingress; otherwise
`server.host:server.port` is used.

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
    limit: 1Mi

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
