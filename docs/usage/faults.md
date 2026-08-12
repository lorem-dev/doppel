# Injecting faults

A backend that always works is the one case your client is already tested
against. Doppel degrades a real upstream on purpose so the other cases can be
reached on demand.

Two faults, plus a third setting that decides how much traffic a mock takes.

## Delaying a share of requests

```yaml
proxies:
  - name: backend
    type: http
    url: "https://api.example.com/v1/"
    resolve:
      type: default
    latency:
      percentage: 0.45   # 45% of requests
      min: 0.05          # delayed by 50ms
      max: 0.2           # to 200ms, drawn uniformly
```

`percentage` is a fraction, not a percentage: `0.45`, never `45`. Writing `45`
is refused at load with a message saying so.

`min` and `max` are seconds and may be fractional. `min: 0` is legal and means
"between nothing and `max`".

Watch it work:

```bash
for i in $(seq 1 20); do
  curl -s -o /dev/null -w '%{time_total}\n' http://localhost:8080/health
done | sort -n | tail -5
```

Roughly nine of twenty should sit near the base latency and the rest between 50
and 200 milliseconds.

### The delay is a target, not an addition

The drawn delay is what the whole response should take, and the time the real
upstream already spent comes out of it. A `min: 0.5, max: 0.5` in front of a
backend answering in 120ms waits 380ms, so the client sees about 500ms -- not
620ms.

This is what makes the configured number mean something: adding to an upstream
whose own latency varies gives a figure nobody chose, and the setting you wrote
would be unreachable by construction.

!!! note "A floor, not a budget"
    An upstream slower than the delay leaves no remainder, and the request is
    passed straight through. Doppel never makes a slow backend look fast, so a
    500ms setting in front of a backend taking 900ms produces 900ms and waits
    for nothing.

    `latency_injected_ms` in the log line is the wait actually taken, so it
    reads `0` in that case even though the roll fired. `duration_ms` is the
    total. The `doppel_latency_injected_total` counter, by contrast, counts
    every request whose roll fired -- whether or not there was anything left to
    wait for.

A request answered by a mock is delayed on the same rule; see
[Faults on one endpoint only](#faults-on-one-endpoint-only).

## Dropping a share of requests

```yaml
    loss:
      percentage: 0.1    # one request in ten
      status: 503        # answered with this, not left hanging
```

A dropped request is answered immediately with `status` and the standard error
envelope. It is not delayed first: loss is decided before latency and
short-circuits, because a request that is being refused should not also occupy
a connection for 200ms.

```bash
for i in $(seq 1 50); do
  curl -s -o /dev/null -w '%{http_code}\n' http://localhost:8080/health
done | sort | uniq -c
```

```
  45 200
   5 503
```

## Combining them

```yaml
    loss:
      percentage: 0.02
      status: 502
    latency:
      percentage: 1.0
      min: 0.1
      max: 0.1
```

A fixed 100ms on everything, and one request in fifty failing. This is the
shape most useful for testing a retry policy: the delay is predictable, so a
timeout can be set just above or just below it, and the failures are frequent
enough to hit within a short run.

## Faults on one endpoint only

Faults belong to the proxy, so they apply everywhere. To degrade a single
endpoint, put the fault on a **mock's** `proxy` block instead:

```yaml
    mocks:
      - name: slow-checkout
        request:
          method: POST
          url: /checkout/
        response:
          status: 200
          json: '{"ok": true}'
        proxy:
          latency:
            percentage: 1.0
            min: 2.0
            max: 3.0
```

The mock's `proxy` block accepts the same three settings and is held to the
same bounds. They apply to requests the mock actually answers -- after it has
matched and won its `replace` roll.

What each one does when the mock leaves it out:

| Setting | A mock that does not declare it | A mock that does |
|---|---|---|
| `replace` | uses the proxy's | uses its own |
| `latency` | uses the proxy's | uses its own **instead** of the proxy's, never on top |
| `loss` | has none at all | uses its own |

`latency` is inherited because it describes how slow this proxy is to answer,
and that is true whatever answers -- so a mocked response is delayed like any
other, and the example above makes `/checkout/` slower than the rest rather than
being the only thing that is slow. Overriding replaces the proxy's figure; the
two are not added, or a mock could only ever be slower than its proxy.

`loss` is the one exception. A mock inheriting it would be dropped by the
proxy's loss, which is exactly the coupling between `loss` and `replace` that
[the ordering](#loss-does-not-eat-into-replace) exists to remove. So a mock that
should be flaky has to say so itself.

!!! note "A dropped request is not delayed first"
    Within either set, `loss` is decided before `latency` and short-circuits it.
    A request the mock's own loss drops does not wait for the mock's latency
    first, and a request the proxy's loss drops does not wait for the proxy's.

## Replacing a backend gradually

`replace` decides what share of *matching* requests a mock actually answers.
The rest are forwarded upstream as though the mock were not there.

```yaml
    mocks:
      - name: new-pricing
        request:
          method: GET
          url: /pricing/
        response:
          status: 200
          json: '{"price": 100, "currency": "EUR"}'
    replace: 0.1
```

Ten percent of `GET /pricing/` requests get the mock; ninety percent reach the
real service. Raise it as confidence grows.

`replace` defaults to `1.0` -- a mock that matches answers -- and can be set on
the proxy (as above) or per mock inside its `proxy` block.

!!! warning "`replace` is not a fault"
    It sits in the same family of fractions but does the opposite thing: loss
    and latency make the real backend worse, `replace` decides how much of it
    is still involved at all. A `replace: 0` mock is dead configuration, not a
    disabled fault.

### `loss` does not eat into `replace`

A mock is decided before either fault, so `replace` is the share of *matching*
requests the mock answers, whatever `loss` is set to:

```yaml
    loss:
      percentage: 0.5
      status: 503
    replace: 0.5
    mocks:
      - name: new-pricing
        request:
          method: GET
          url: /pricing/
        response:
          status: 200
          json: '{"price": 100}'
```

Half of `GET /pricing/` requests get the mock -- not a quarter. The other half
go on to the loss roll, so about a quarter are dropped with `503` and about a
quarter reach the real service. Requests to any other path are unaffected by
`replace` and take the loss roll as usual.

The mock's half is not touched by the proxy's `loss` -- that is the whole point
of deciding the mock first. It *is* delayed by the proxy's `latency`, which
applies to every answer this proxy gives. See
[Faults on one endpoint only](#faults-on-one-endpoint-only) for the table of
what a mock inherits and what it does not.

## The bounds, and why they exist

| Field | Range | Refused beyond it because |
|---|---|---|
| `percentage`, `replace` | 0.0 to 1.0 | A fraction. `50` is a unit mistake, not fifty percent |
| `latency.min`, `latency.max` | 0 to 300 seconds | Past five minutes every client has given up, so the request is not being delayed, it is being made to fail with nothing reporting it |
| `loss.status` | 100 to 599 | An HTTP status |

`min` must not exceed `max`. That one is a validation rule rather than a type,
because it needs both fields.

## What faults do not do

- They do not apply to the admin listener. `/api/v1/status` and `/metrics` answer
  normally while the proxy is dropping traffic, which is what makes them
  usable for watching it happen.
- They are not recorded separately in metrics. A lost request appears as its
  status in `doppel_requests_total`; there is no counter for "requests Doppel
  chose to fail". See [Observability](observability.md).
- They are not seeded. The draw is random per request, so two runs of the same
  configuration do not fail the same requests.
