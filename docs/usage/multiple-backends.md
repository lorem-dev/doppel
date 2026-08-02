# Several backends behind one port

One Doppel process, one listener, several upstreams. A request picks its proxy
by carrying a header that names it; anything that does not is handled by the
default.

This is what lets a test suite point at a single address and still reach a
staging backend, a production-shaped one and a fully mocked one, without
restarting anything or juggling ports.

## A worked example

```yaml
server:
  host: "127.0.0.1"
  port: 8080

admin:
  host: "127.0.0.1"
  port: 8081
  tokens: []
  access: {}
  upload:
    limit: 1Mi

proxies:
  # Everything that does not ask for something else.
  - name: production
    type: http
    url: "https://api.example.com/v1/"
    resolve:
      type: default

  # Reached by `X-Backend: staging`.
  - name: staging
    type: http
    url: "https://staging.example.com/v1/"
    resolve:
      type: header
      header: X-Backend
    latency:
      percentage: 1.0
      min: 0.3
      max: 0.3

  # Reached by `X-Backend: offline`. No upstream is ever contacted for the
  # paths its mocks cover; everything else still goes to the url below.
  - name: offline
    type: http
    url: "https://api.example.com/v1/"
    resolve:
      type: header
      header: X-Backend
    mocks:
      - name: catalogue
        request:
          method: GET
          url: /catalogue/
        response:
          status: 200
          json: '{"items": []}'
```

```bash
curl -s localhost:8080/catalogue/                         # production
curl -s -H 'X-Backend: staging' localhost:8080/catalogue/ # staging, +300ms
curl -s -H 'X-Backend: offline' localhost:8080/catalogue/ # {"items": []}
```

## The rules resolution follows

**One default at most.** Two proxies declaring `type: default` is refused at
load. Zero is legal: a request matching no header then gets `404` with
`PROXY_NOT_RESOLVED`, which is a reasonable configuration for a process that
should only ever serve traffic that asks for something specific.

**Headers are tried in configuration order.** A request carrying two
resolution headers resolves to whichever proxy is written first, on every
process and every run. That is deterministic rather than arbitrary, which
matters when the same configuration runs in three places.

**A name only works through its own header.** With `staging` resolving on
`X-Backend`, a request carrying `X-Proxy-Name: staging` does *not* reach it.
Otherwise every proxy would be reachable through every resolution header and
the per-proxy setting would be decorative.

**Resolution headers are stripped before forwarding.** The upstream does not
learn Doppel's routing vocabulary.

## Different headers for different groups

Nothing requires one shared header name:

```yaml
  - name: eu
    resolve:
      type: header
      header: X-Region
  - name: us
    resolve:
      type: header
      header: X-Region
  - name: canary
    resolve:
      type: header
      header: X-Canary
```

`X-Region: eu` reaches the first, `X-Canary: canary` the third. A request
carrying both `X-Region: us` and `X-Canary: canary` reaches `us`, because it is
written first.

## Per-proxy access control

Each proxy can narrow who may read or change it through the admin API, for the
four actions a proxy may override:

```yaml
  - name: production
    access:
      read: ["sre"]
      update: ["sre"]
      delete: ["sre"]
      upload: ["sre"]
```

`list` and `create` cannot be overridden per proxy -- they are not about one
proxy -- and a configuration trying to is refused at parse time.

See [The admin API](admin-api.md#access-control).

## Watching which proxy served what

The `proxy` label is on every request metric:

```
doppel_requests_total{proxy="staging",method="GET",status="200"} 41
doppel_requests_total{proxy="production",method="GET",status="200"} 900
```

There is deliberately no `path` label. See
[Observability](observability.md).
