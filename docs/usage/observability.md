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
| `loss_injected` | Whether the loss roll dropped this request |
| `latency_injected_ms` | How long the request was actually made to wait |

`upstream_contacted` is a boolean rather than a null status because a null
still invites a consumer to plot it, where a boolean says what happened. A
dropped request, an unresolved one and a mocked one all report `false`.

`latency_injected_ms` is the wait taken, not the delay drawn. An injected
latency is a target for the whole response, so an upstream that already spent
longer than the target leaves nothing to wait for and this reads `0` even though
the roll fired. `doppel_latency_injected_total` counts the roll, so the two
disagree in exactly that case -- deliberately, since "how often latency was in
play" and "how much of it this request felt" are different questions. See
[Injecting faults](faults.md#the-delay-is-a-target-not-an-addition).

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

The one endpoint outside `/api/`. `metrics_path` defaults to `/metrics` in
Prometheus and in every agent and annotation that scrapes one, so the path is
settled by something larger than this project -- and a scrape of `/metrics` while
the exposition was under `/api/v1/` was answered by the dashboard with `200
text/html`, which reads as a parse failure or as no metrics at all.

### Traffic

| Metric | Type | Labels |
|---|---|---|
| `doppel_proxy_request_duration_seconds` | histogram | `proxy`, `method`, `status`, `replace`, `loss`, `upstream_error` |
| `doppel_upstream_request_duration_seconds` | histogram | `proxy`, `method`, `status` |
| `doppel_admin_request_duration_seconds` | histogram | `route`, `method`, `status` |
| `doppel_loss_total` | counter | `proxy` |
| `doppel_latency_injected_total` | counter | `proxy` |
| `doppel_mock_hits_total` | counter | `proxy`, `mock` |

`replace`, `loss` and `upstream_error` are `1` or `0`, and independent: a mock can
answer a request its own loss roll then drops, and `upstream_error` covers a
transport failure, a timeout, and a relayed status of 500 or above.

`route` on the admin histogram is the route template -- `/api/v1/proxies/{name}`,
never `/api/v1/proxies/alpha` -- so a hundred proxies are one series and a query
string is none. A request that matched no route is recorded with an empty `route`.

Buckets are explicit rather than the exporter's default summary, because a
quantile cannot be aggregated across replicas. Two ladders: 5ms to 60s for
proxied traffic, because `latency` injection is asked to be slow on purpose, and
5ms to 5s for the admin API, where nothing has any business taking longer.

### State

These exist from startup, before anything has happened, because a panel and an
alert both read a never-recorded metric as "no data" -- which is
indistinguishable from a process nobody is scraping.

| Metric | Type | Labels | Meaning |
|---|---|---|---|
| `doppel_build_info` | gauge | `version` | Always `1`. The version is a label because it is not a number |
| `doppel_dashboard_info` | gauge | `enabled` | Always `1`. Separate from the build: one describes the artifact, the other what this deployment turned on |
| `doppel_proxy_last_error_timestamp_seconds` | gauge | `code` | When the proxy listener last answered with that error. Starts as `code=""`, value `0` |
| `doppel_proxy_mocks` | gauge | `proxy` | Mocks per proxy in the configuration **now in effect**, republished on every reload and every write through the API. A proxy that leaves the configuration goes to `0` rather than keeping its last count |

"Is it still failing" is `time() - doppel_proxy_last_error_timestamp_seconds`,
which is why the metric is a timestamp and not a counter: a counter answers "how
many" and needs a rate window to say anything about now.

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

### The DSN from the environment

```bash
DOPPEL_SENTRY_DSN="https://key@sentry.example.com/1"
```

A DSN carries the key that authorises sending events, which makes it the one
Sentry setting that is a credential -- and a deployment that provisions
credentials through the environment should not have to write this one into a
document the admin API returns and the store keeps.

The variable wins over `sentry.dsn`, the same way `DOPPEL_ADMIN_TOKENS` wins over
`admin.tokens`: it is how a deployment overrides a document it may not be able to
edit. Startup says which source it used, so the answer is one line away rather
than a guess:

```json
{"level":"INFO","fields":{"message":"sentry reporting enabled",
 "dsn":"https://<redacted>@sentry.example.com/1","source":"DOPPEL_SENTRY_DSN"}}
```

**An empty variable is not a way to turn reporting off.** It counts as unset and
leaves `sentry.dsn` in force, deliberately: `DOPPEL_SENTRY_DSN=${SENTRY_DSN}` with
nothing behind `SENTRY_DSN` is a compose file that means nothing by it, and
silently disabling error reporting is the worse reading. To turn it off, write
`dsn: ""` or remove the section.

Doppel reads its own name, not the conventional `SENTRY_DSN`. A variable that is
in the environment for the service beside this one should not make this one start
reporting to it.

A build without the feature that is given a DSN warns at startup and carries
on. It does not pretend to report, and it does not refuse to run: reporting is
optional by design, so turning a missing integration into an outage would be
worse than the gap.
