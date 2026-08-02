# Proxy behaviour

## Choosing a proxy

Several proxies can sit behind one port. A proxy either declares itself the
default or is selected by a named header carrying its name:

```yaml
proxies:
  - name: primary
    resolve:
      type: default
  - name: staging
    resolve:
      type: header
      header: X-Proxy-Name
```

`X-Proxy-Name: staging` selects the second. Anything else falls back to the
default. Resolution headers are tried in configuration order, so a request
carrying two of them resolves the same way on every process and every run.

A header naming a proxy that resolves on a *different* header does not match --
otherwise any proxy could be reached through any resolution header and the
per-proxy setting would be decorative.

Zero defaults is legal. A request that matches no header then gets `404` with
`PROXY_NOT_RESOLVED`.

## Building the upstream URL

The proxy's `url` is treated as a directory even without a trailing slash, and
the incoming path is appended. The query string is forwarded unchanged.

A configured `https://host/api/v1` plus an incoming `/resource/42` gives
`https://host/api/v1/resource/42` -- not `https://host/api/resource/42`, which
is what naive URL joining produces.

!!! note "The upstream is confined"
    A proxy configured for one upstream and one base path can only ever reach
    paths under that base on that upstream. This is enforced by checking the
    built URL -- its scheme, host, port and path prefix -- not by sanitising the
    input.

    That distinction matters. Input sanitising was tried twice and failed twice:
    first a request target like `/https://evil.example.com/x` replaced the
    authority outright, then a filter rejecting literal `.` and `..` segments
    was bypassed by `%2e%2e` and by backslashes. A rule that enumerates
    dangerous inputs tracks another library's normalisation table and loses when
    that table changes; asserting the property you need does not.

    A request path containing `..` or a backslash is rejected with `400` and
    `INVALID_REQUEST_PATH`.

## Headers

Hop-by-hop headers are stripped in both directions, as are any headers named in
a `Connection` field. `Host` is not relayed -- it is derived from the upstream
URL, so the upstream sees its own authority.

Headers configured on the proxy are injected into the outbound request and
override anything the client sent by the same name. The resolution headers are
stripped, so the upstream does not learn Doppel's routing vocabulary.

`X-Forwarded-For` is appended to rather than replaced, preserving any chain
that arrived.

`X-Request-ID` is reused if the client sent one and generated otherwise, sent
upstream, and returned on the response, so one request can be followed across
services.

## Redirects

A `3xx` from the upstream is relayed to the caller with its `Location` intact,
not followed. The redirect target is the client's decision, and a streamed
request body could not be replayed to it anyway.

## Faults

```yaml
    loss:
      percentage: 0.1
      status: 503
    latency:
      percentage: 0.45
      min: 0.05
      max: 0.2
```

Percentages are fractions: `0.0` never fires, `1.0` always does. Loss is
decided first and short-circuits -- a request being dropped is not delayed
first. Latency is drawn uniformly between `min` and `max` seconds.

Both apply before an endpoint is chosen, because they are properties of the
proxy rather than of a route.

## Timeouts and upstream failures

`timeout` bounds the whole upstream exchange, not just connecting, and defaults
to 30 seconds.

| Situation | Code | Status |
|---|---|---|
| The upstream did not answer in time | `UPSTREAM_TIMEOUT` | 504 |
| The connection failed, or the response could not be relayed | `UPSTREAM_ERROR` | 502 |
| No proxy matched and there is no default | `PROXY_NOT_RESOLVED` | 404 |
| The request path contained `..` or a backslash | `INVALID_REQUEST_PATH` | 400 |
