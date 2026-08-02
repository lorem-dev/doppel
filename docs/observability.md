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
there are tests that assert it. A database URL is masked wherever it can
surface.

## Metrics

Not implemented yet. The admin API in a later phase exposes Prometheus metrics:
latency histograms for the upstream and for the proxy, labelled by status and
method but deliberately not by path.
