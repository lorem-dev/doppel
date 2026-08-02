# Observability

## Logs

Structured, to stdout, one line per request, configured by the `logging`
section:

```yaml
logging:
  level: info    # trace | debug | info | warn | error
  format: json   # json | text
```

`RUST_LOG` overrides `level` when set. That is deliberate: it is what an
operator reaches for when they need more detail from a process they cannot
reconfigure and restart. An empty or whitespace-only `RUST_LOG` is ignored
rather than treated as an empty filter, because an exported-but-empty variable
is far more often an accident than an instruction.

### The request line

Every request logs once, on completion, with the same key set on every branch:

| Field | Meaning |
|---|---|
| `request_id` | From the client's `X-Request-ID`, or generated |
| `proxy` | Which proxy handled it, empty if none resolved |
| `mock` | Which mock served it; absent when none did |
| `method`, `path`, `status` | The request and its outcome |
| `duration_ms` | Time to the response headers |
| `upstream_contacted` | Whether an upstream was reached at all |
| `upstream_status`, `upstream_duration_ms` | Present only when it was |
| `loss_injected`, `latency_injected_ms` | Which faults fired |

`upstream_contacted` is a boolean rather than a null status because a null
still invites a consumer to plot it, where a boolean says what happened. A
dropped request, an unresolved one and a mocked one all report `false`.

`duration_ms` and `upstream_duration_ms` both stop at the response headers, not
at the end of the body. A large download is not reflected in either.

Paths are logged in full. The prohibition on path labels applies to metrics,
where cardinality is expensive; logs are not aggregated by label.

### Secrets

Admin token values never reach the logs, at any level including `trace`, and
there are tests that assert it. A database URL and a Sentry DSN are masked
wherever they can surface -- a Sentry DSN carries the key that authorises
sending events, in the same position a URL carries a password.

## Metrics

`GET /metrics` on the admin listener, in the Prometheus text format,
unauthenticated because a scraper has nowhere to put a token.

| Metric | Type | Labels |
|---|---|---|
| `doppel_proxy_request_duration_seconds` | histogram | `proxy`, `method`, `status` |
| `doppel_upstream_request_duration_seconds` | histogram | `proxy`, `method`, `status` |
| `doppel_loss_total` | counter | `proxy` |
| `doppel_latency_injected_total` | counter | `proxy` |
| `doppel_mock_hits_total` | counter | `proxy`, `mock` |

Buckets are explicit -- 5ms to 10s -- rather than the exporter's default
summary, because a quantile cannot be aggregated across replicas.

Every request through the proxy listener appears in the proxy histogram,
including one that was dropped by loss injection or that resolved to no proxy
at all; otherwise a rise in failures would read as a fall in traffic. A
request answered by a mock produces no upstream observation, because no
upstream was contacted.

A request that resolved to no proxy is recorded with an empty `proxy` label.
Rule V35 refuses an empty proxy name, so it cannot collide with a real one.

### No path labels

Not by convention but by construction: no recording function in
`doppel_core::metrics` accepts a path, so there is nothing to forget at a call
site.

`method` is bounded the same way. It arrives from the wire, so an unbounded
label there is a cardinality explosion any client can trigger by inventing
methods. Anything outside the recognised set is recorded as `OTHER`.

## Sentry

Optional, behind the `sentry` cargo feature and off in a default build:

```bash
cargo build --release --features sentry
```

```yaml
sentry:
  dsn: "https://key@sentry.example.com/1"
```

An absent section or an empty DSN disables it and is not an error. A
malformed DSN fails startup rather than producing a client that silently
drops everything -- with the key masked in the message.

A build without the feature that is given a DSN warns at startup and carries
on. It does not pretend to report, and it does not refuse to run: reporting is
optional by design, so turning a missing integration into an outage would be
worse than the gap.
