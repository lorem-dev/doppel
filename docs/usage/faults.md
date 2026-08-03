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

Roughly nine of twenty should sit near the base latency and the rest between
50 and 200 milliseconds above it.

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
same bounds.

!!! warning "Only `replace` is applied per mock today"
    A mock's `proxy.replace` overrides the proxy's, and does so on every
    request that matched the mock. Its `proxy.loss` and `proxy.latency` are
    accepted, validated and compiled, and then nothing reads them: no request
    is dropped or delayed on their account. They are declared behaviour that
    does not exist yet, not a setting with a subtle scope.

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

The mock's half is never dropped and never delayed. That is the same rule
stated from the other side: the faults are on the path to the upstream, and a
mocked request does not take it.

## The bounds, and why they exist

| Field | Range | Refused beyond it because |
|---|---|---|
| `percentage`, `replace` | 0.0 to 1.0 | A fraction. `50` is a unit mistake, not fifty percent |
| `latency.min`, `latency.max` | 0 to 300 seconds | Past five minutes every client has given up, so the request is not being delayed, it is being made to fail with nothing reporting it |
| `loss.status` | 100 to 599 | An HTTP status |

`min` must not exceed `max`. That one is a validation rule rather than a type,
because it needs both fields.

## What faults do not do

- They do not apply to the admin listener. `/status` and `/metrics` answer
  normally while the proxy is dropping traffic, which is what makes them
  usable for watching it happen.
- They are not recorded separately in metrics. A lost request appears as its
  status in `doppel_requests_total`; there is no counter for "requests Doppel
  chose to fail". See [Observability](observability.md).
- They are not seeded. The draw is random per request, so two runs of the same
  configuration do not fail the same requests.
