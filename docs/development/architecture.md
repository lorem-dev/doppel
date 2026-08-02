# Architecture

Doppel is a Cargo workspace. Dependencies point one way and converge on
`doppel-core`, which is what lets the configuration model be exercised without
a network, rendering without HTTP, and the proxy logic without an admin API.

## The crates

| Crate | Owns |
|---|---|
| `doppel-core` | The configuration model and its types, YAML loading, validation, the `ConfigStore` trait and its file-backed implementation, the compiled runtime, the error model |
| `doppel-render` | Variable extraction and Jinja2 rendering for mock responses; knows nothing about HTTP |
| `doppel-proxy` | The proxy listener, resolution, fault injection, mock matching, upstream forwarding, the request pipeline |
| `doppel-admin` | The admin HTTP API: token access control, proxy CRUD, template files, reload, status, metrics, the OpenAPI document |
| `doppel-store-postgres` | `PostgresStore`, and the sqlx migrations that own its schema |
| `doppel-telemetry` | Logging initialization and optional Sentry |
| `doppel-cli` | The `doppel` binary: argument parsing, the control channel, and wiring the rest together |

```
doppel-cli -> { doppel-proxy, doppel-admin, doppel-store-postgres,
                doppel-telemetry, doppel-core }
doppel-proxy -> { doppel-render, doppel-core }
doppel-admin -> doppel-core
doppel-store-postgres -> doppel-core
doppel-render -> doppel-core
doppel-telemetry -> doppel-core
doppel-core -> (nothing in this workspace)
```

## Configuration: parse, don't validate

A constraint on a single configuration value belongs in a **type**, not in a
validation rule. `crates/doppel-core/src/config/` holds one module per kind of
value -- `Name`, `Token`, `Port`, `HttpStatus`, `HttpMethod`, `Ratio`,
`Seconds`, `TimeoutSeconds`, `ByteSize`, `UpstreamUrl`, `HeaderName`,
`HeaderValue`, `Pattern`, `Selector`, `TemplateName` -- each refusing bad input
while the document is being parsed.

The rule set in `crates/doppel-core/src/validate/` is for what a type cannot
decide: anything that needs two fields at once, or the whole document.
Duplicate names, `min <= max`, "this field is required when that one says so",
"the two listeners must not share a port".

Nineteen of the original thirty-five rules are gone, each into a type. The
[retired-rules table](../usage/configuration.md#retired-rules) records where
each went, and three tests keep that table honest.

A type that carries something expensive to derive should carry it derived:
`Pattern` holds the compiled regex, `UpstreamUrl` the parsed URL, `Selector`
its segments. That is what removes the second parse rather than merely moving
the first.

## The compiled runtime

A `Config` is a document. A `Runtime` is that document with everything the
request path needs already computed -- patterns compiled, header names folded,
response shapes resolved to one variant.

`RuntimeHolder` swaps one atomically. A reload builds a whole new `Runtime`,
validates and compiles it, and only then swaps; a failure anywhere leaves the
running one untouched. Requests in flight finish against the runtime they
started with.

`Runtime::compile` reads `config.proxies`. Other sections reached through the
same `Arc<Config>` -- notably `admin` -- are therefore live after a reload,
while the *listeners* those sections describe are not, because they are already
bound. `unapplied_sections` reports the difference.

## Authorization

Every admin handler judges a request against the configuration the last reload
put into effect, never against whatever the store holds at that moment. The
store is what it reads and writes *data* through.

That distinction is a security property, not a style: someone who can write the
configuration out of band, but holds no token, must not be able to grant
themselves one and use it on the next request. `access::policy` is the one way
a handler gets the configuration it judges by.

## Two stores, one trait

`ConfigStore` is the boundary. `FileStore` serialises writers with an advisory
lock on a sibling `<config>.lock` file; `PostgresStore` uses a conditional
`UPDATE` inside a transaction. Reads on the PostgreSQL side take one
`REPEATABLE READ` snapshot, because a configuration is five tables and a
per-statement snapshot would assemble one from two of them.

One conformance suite runs against both, which is what keeps the trait's
promises true rather than merely intended. It lives in
`doppel-core/src/conformance.rs` behind the `test-support` feature and is the
exit criterion for any new store.

## The control channel

A Unix socket carrying newline-delimited JSON, one request and one response per
connection. It is how `config reload` and `token add` reach a running process
without going through the admin listener -- which may be turned off, and which
needs a token the operator may be trying to create.

The socket is bound inside a private directory and published with a hard link,
which is atomic *and* exclusive: a second bind fails loudly rather than
silently taking over a live channel.
