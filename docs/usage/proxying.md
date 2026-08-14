# Proxy behaviour

## No proxies configured

An empty `proxies` list, or no `proxies` key at all, is a valid configuration.
Doppel starts, binds both listeners, serves the admin API, and waits.

A request arriving meanwhile is answered:

```json
{
  "status": 503,
  "message": "no proxies are configured; add one and reload",
  "code": "NO_PROXIES_CONFIGURED"
}
```

`503`, not `404`. Nothing is wrong with the request -- the service is not in a
position to answer one yet. A `404` would tell the caller their path was wrong
and send whoever is debugging the client into the client. `503` also carries the
right invitation: add a proxy, reload, and the next attempt works, with no
restart.

This is deliberately not a startup failure. Rule V5 used to refuse it, which
meant a fresh deployment could not come up until its proxies were written --
so the two ways of adding one, `doppel config reload` and the admin API, were
both unreachable exactly when they were most useful. Provisioning an empty
Doppel and filling it over the API is now a supported order of operations.

`NO_PROXIES_CONFIGURED` is distinct from `PROXY_NOT_RESOLVED` (`404`), which
means proxies exist and none of them wanted this particular request.

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

Because `Host` is replaced, what the client asked for is sent on instead:

| Header | Value |
|---|---|
| `X-Forwarded-Host` | the authority the client used |
| `X-Forwarded-Proto` | `http` -- Doppel terminates no TLS, so `https` would name a hop that does not exist |
| `X-Forwarded-For` | the chain that arrived, with the peer appended |

The first two are only set when the request did not already carry them, and
`X-Forwarded-For` is appended to rather than replaced. All three preserve what
arrived, so Doppel behind another proxy keeps the authority the client really
used rather than substituting an internal one. The flip side is that a client
talking to Doppel directly can put whatever it likes in them -- true of any proxy
that preserves a chain, and the reason these headers are only ever as
trustworthy as the hop that set them.

`X-Forwarded-Port` and RFC 7239 `Forwarded` are not generated. One arriving from
a client is relayed untouched.

`X-Request-ID` is reused if the client sent one and generated otherwise, sent
upstream, and returned on the response, so one request can be followed across
services.

## Redirects

A `3xx` from the upstream is relayed to the caller, not followed. The redirect
target is the client's decision, and a streamed request body could not be
replayed to it anyway.

Its `Location` is rewritten to keep the client behind the proxy:

```yaml
proxies:
  - name: backend
    url: "https://api.example.com/v2/"
    rewrite_redirects: true    # the default
```

With a base of `https://api.example.com/v2/`, an upstream answering
`Location: https://api.example.com/v2/orders/7` produces
`Location: http://127.0.0.1:8080/orders/7` to the client. Query and fragment
survive.

The host in that answer is Doppel's own, and where it comes from is
[Doppel's own address](#doppels-own-address) below.

!!! warning "Why this is on by default"
    `Host` is not relayed, so the upstream answers with its *own* authority in
    `Location`. Relayed untouched, a client following it talks to the backend
    directly from then on -- past every injected fault and every mock, with
    nothing logged and nothing failing. The test still passes; it has just
    stopped testing anything.

    `nginx` has `proxy_redirect` for this and Apache `ProxyPassReverse`, both on
    by default, for the same reason.

A target on the upstream's own host but **outside** the proxy's base is kept on
Doppel too, with the path the upstream wrote: an upstream answering
`Location: /login` under a base of `/v2/` produces
`Location: http://127.0.0.1:8080/login`. This is what `nginx`'s `proxy_redirect`
does, and it is the case the relative form could not express -- relayed as-is,
the client would come back asking for `/v2/login`, a different resource nobody
named, and pointed at the upstream it leaves Doppel altogether.

Whether Doppel serves that path is a question about the configuration rather than
about the rewrite: `/login` reaches `<base>/login`, so a proxy whose base has a
prefix will forward it under that prefix. That is visible in the logs and fixable
in the configuration, which the silent escape was not.

A target on **another host** is left pointing there, absolutely. Doppel does not
proxy it, and naming itself in a redirect to somewhere it cannot serve would be a
lie.

## Urls in a body

The same problem one layer down, and on by default for the same reason:

```yaml
proxies:
  - name: backend
    url: "https://api.example.com/v2/"
    rewrite_urls: true    # the default
```

A page, a script or a JSON document that names `https://api.example.com/v2/orders`
sends the client straight to the upstream on its next request -- past every
injected fault and every mock, with nothing logged. Doppel replaces its own address
into the body instead: that URL becomes `http://127.0.0.1:8080/orders`, and one on
the same host outside the proxied path keeps its own path, exactly as a rewritten
redirect does. `nginx` calls this `sub_filter`.

**Only the exact host.** `https://cdn.api.example.com/` is a different host, is not
proxied by this proxy, and is left alone -- pointing it here would break the page
rather than keep it working. A host that merely starts the same, like
`https://api.example.com.evil.test/`, is left alone too.

Three more limits, each of which relays the body untouched rather than guessing:

- **Text only**, by `Content-Type`: `text/*`, JSON, JavaScript, XML and the `+json`
  and `+xml` suffixes. An image cannot carry a URL that matters, and buffering one
  to look is how a proxy runs out of memory.
- **Uncompressed only.** A body with a `Content-Encoding` is relayed as it came;
  the client asked the upstream for that encoding.
- **Bounded by `body_limit`.** Rewriting needs the whole body, so it is buffered up
  to that ceiling; a bigger body streams on from where the buffering stopped.

A rewritten body loses the `ETag` and digest headers that described the upstream's
version of it, and carries the length it now has. A conditional request with the
upstream's validator would otherwise be answered `304` for content the client has
never seen.

Set `rewrite_urls: false` for a client being tested against the bytes the upstream
actually sent.

## Doppel's own address

Rewriting a `Location` means naming the address the client used, and Doppel
cannot work that out for itself -- `Host` is a claim by the caller, and building
a redirect out of it hands the caller the redirect.

In order:

```yaml
server:
  host: 0.0.0.0
  port: 8080
  external_url: "https://doppel.example.com/"   # optional
```

1. `DOPPEL_EXTERNAL_URL`, which overrides everything below it.
2. `server.external_url`.
3. `server.host` and `server.port`, which is right for the common case and needs
   no configuration: a laptop on `127.0.0.1:8080`, a pod on its own address.
   A wildcard bind (`0.0.0.0`, `::`) becomes loopback, because `0.0.0.0` names
   every address this host has and therefore none of them.

The third is a guess, and the one place this is wrong: behind a container port
mapping (`-p 18080:8080`), a load balancer or an ingress, the client used neither
that address nor that port. Set `external_url`, or the variable -- and Doppel logs
which address it settled on at startup, so it is one line away from being checked
rather than assumed.

A path is kept as a prefix: `https://gw.example.com/doppel/` is a Doppel reached
under a prefix, and its rewritten locations carry it.

### When one address is not enough

`external_url` may be a template over the
[system variables](mocks.md#system-variables), rendered per request:

```yaml
server:
  # Whatever this client asked for.
  external_url: "http://{{ host }}/"
  # Or a name per proxy, behind a wildcard.
  external_url: "https://{{ proxy_name }}.gw.example.com/"
```

A value containing `{{` is a template; anything else is parsed as a url when the
configuration is read, as before. The scheme has to be literal -- `http://` or
`https://` -- because a value that does not start with one cannot become a usable
url however it renders, and that is worth failing at startup.

!!! warning "`{{ host }}` is the caller's claim"
    `Host` arrives from the client. A deployment that builds a redirect out of it
    is choosing to let a caller decide where its own redirects point, which is
    fine when something in front validates the host and is not when nothing does.
    That is why Doppel does not do this by default: it is one line to opt in, and
    the line is where the decision belongs.

A template that fails to render, or renders to something that is not a url, means
**no rewriting for that request** -- the upstream's own `Location` is relayed
instead. A cosmetic feature is not worth a `500`, and the reason is logged at
debug rather than per redirect at warn.

Set `rewrite_redirects: false` to relay the header byte for byte. That is what a
client being tested *for its redirect handling* needs; it is not what a client
being tested against a degraded backend needs.

Only `Location` is rewritten. `Content-Location` names where a payload lives
rather than where to go next, `Refresh` is not a standard header, and the
`Domain` attribute of a `Set-Cookie` needs its own rule -- none of the three is
touched.

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
