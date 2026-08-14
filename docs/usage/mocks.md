# Mocks and templating

A mock answers a matching request instead of forwarding it. Everything that
does not match still reaches the upstream.

## Matching

A mock matches when the request's method equals its `method` exactly, and the
request's path matches its `url` as a regular expression.

Methods are case-sensitive, and the configuration must spell them in upper
case -- `method: get` is rejected at load rather than silently never matching.

!!! warning "Patterns are unanchored, and order decides"
    `url: /api/v1/resource/` matches any path *containing* that string, so it
    also matches `/api/v1/resource/42/`. Mocks are tried in the order they are
    written and the first match wins, so a general pattern placed above a
    specific one makes the specific one unreachable.

    Doppel does not detect this. A shadowed mock is a valid configuration --
    just a useless one. Put specific patterns first.

    This is not hypothetical: the reference configuration shipped with two
    unreachable mocks until an end-to-end test caught it.

The pattern stays unanchored deliberately. Anchoring it would silently change
what already-written configurations mean, and a change that alters behaviour
without the configuration changing is the worst kind -- there is no error and
nothing to review.

### Repeated leading slashes

A run of slashes at the start of the path is collapsed to one before matching,
so `GET //api/v1/index/` is matched as `/api/v1/index/`.

Clients produce the doubled form constantly -- a base URL ending in `/` joined
to a path beginning with `/` is the usual way -- and it is legal HTTP, so
nothing rejects it. An unanchored pattern happened to match it anyway, because
the extra slash falls outside the substring being looked for. An anchored one
(`^/api/v1/index/$`) did not, and the only symptom was the mock silently not
firing.

Only the leading run. `/a//b/` keeps its empty middle segment and does not
match a pattern written `/a/b/`: whether those name the same resource is the
upstream's business, and answering it here would also disagree with the path
that gets forwarded.

## Variables

Five sources. Four are yours and optional; the fifth is Doppel's and always
there.

**Path captures.** Named groups in the pattern become variables:

```yaml
          url: /api/v1/resource/(?P<resource_id>\d+)/
```

binds `resource_id`.

**Headers.** A map of variable name to header name:

```yaml
          headers:
            trace_id: X-Trace-Id
```

**Query and body.** A map of variable name to a selector -- a leading dot and
dot-separated keys:

```yaml
          query:
            filter: .filter
          body:
            item_count: .content.items
```

Selectors address object keys. A selector that lands on an array yields the
array, which templates can iterate or measure with `length`. Array indexing is
not supported.

A variable name may not collide with a capture group name; that is rejected at
load.

Names are `snake_case` throughout this documentation and in
`main.example.yaml`. Jinja accepts any identifier, so `itemCount` works -- but
Doppel's own variables are `snake_case`, and one convention per context reads as
one source.

## System variables

Doppel binds nine variables into every template it renders, whether or not the
mock asked for anything:

| Variable | Is |
|---|---|
| `proxy_name` | The proxy that resolved. Empty for a request that resolved to none |
| `mock_name` | The mock answering. Empty when the request is being forwarded |
| `doppel_version` | The version of the binary serving the request |
| `request_id` | The id echoed in `X-Request-ID`, minted when the client sent none |
| `method` | The request method |
| `path` | The request path, without the query string |
| `host` | The `Host` the client asked for. Empty when it sent none |
| `peer_ip` | The address the connection came from |
| `real_ip` | Who the request is *said* to be from: `X-Real-IP`, else the leftmost `X-Forwarded-For` entry, else `peer_ip` |

```yaml
        response:
          status: 200
          json: '{"served_by": "{{ proxy_name }} {{ doppel_version }}", "caller": "{{ real_ip }}"}'
          headers:
            X-Request-ID: "{{ request_id }}"
```

**`peer_ip` and `real_ip` are not the same claim.** `peer_ip` is the socket's own
address and nobody can fake it. `real_ip` is what a proxy in front says, out of
headers a client can also send -- useful for a mock that reports who called it,
and not something to make a decision on unless you know what sits in front.

**They are reserved.** They are bound after your extractions, so a mock that
extracts into `proxy_name` finds Doppel's value in its template rather than its
own. The extraction still happens and its result is thrown away, which is why
startup says so:

```
mock `m1` of proxy `alpha` extracts `proxy_name` into a name Doppel binds
itself; the system value wins, so the extraction is read and thrown away
```

Being always present, they also never need `| default('')` -- an absent one is an
empty string rather than an undefined variable, which is the one place Doppel's
own variables are gentler than yours.

## Rendering

Exactly one of three fields produces the body, or none at all for a status that
forbids one:

- **`body`** -- a template rendered and returned as-is.
- **`json`** -- a template whose output is additionally parsed as JSON, so a
  template that produces malformed JSON fails loudly instead of emitting a
  broken body. The parse is a *check*: what goes on the wire is exactly what
  the template rendered, with your key order and spacing intact.
- **`template`** -- the name of a file under `<templates.dir>/<proxy>/`.

Every value in `response.headers` is a template too.

### Rendering is strict

An undefined variable is an error, not an empty string.

```yaml
          headers:
            request_id: X-Request-ID
        response:
          json: '{"seen": "{{ request_id }}"}'
```

A request without `X-Request-ID` fails this mock with
`TEMPLATE_RENDER_ERROR`, because `request_id` is undefined. That is deliberate:
a mock that silently renders `"seen": ""` because a variable was mistyped is
worse than one that refuses. The error message names the expression that
failed.

If a variable should be optional, say so:

```yaml
          json: '{"seen": "{{ request_id | default('''') }}"}'
```

## Serving some of the time

`replace` is the probability that a matching mock actually answers. It defaults
to `1.0`.

```yaml
    replace: 0.5          # on the proxy: half of all matched requests
    mocks:
      - name: sometimes
        proxy:
          replace: 0.1    # on the mock: overrides the proxy for this one
```

A mock that matches but loses the roll falls through to the upstream. That is
the point of the setting: serve a mock part of the time and the real backend
the rest.

## Bodies and the size limit

Doppel streams request bodies rather than buffering them, so an upload larger
than memory passes straight through. A mock that extracts variables from the
body cannot do that -- it needs the whole body in hand.

So buffering happens only when a matched mock declares `body` selectors, and
only up to the proxy's `body_limit`, which defaults to 1 MiB. A body over the
limit is rejected with `413` and `UPLOAD_TOO_LARGE`.

It is rejected rather than quietly forwarded upstream on purpose. A proxy that
stopped mocking under load would show you real backend traffic you believed was
intercepted, and you would have no way to tell.

## Template files

`template: put.json.j2` names a file at `<templates.dir>/<proxy-name>/put.json.j2`.

The file is read when the request arrives, not when the configuration loads, so
a mock may name a file that does not exist yet and have it
[uploaded through the admin API](runtime-changes.md#uploading-a-template-at-runtime)
later, with no reload. A missing file at request time is `TEMPLATE_NOT_FOUND`.

## Errors

Every failure returns the standard envelope with the status below:

```json
{"status": "error", "message": "...", "code": "TEMPLATE_RENDER_ERROR"}
```

| Situation | Code | Status |
|---|---|---|
| Undefined variable, bad filter, template syntax error | `TEMPLATE_RENDER_ERROR` | 500 |
| A `json` response rendered to something that is not JSON | `TEMPLATE_RENDER_ERROR` | 500 |
| `template` names a file that is not on disk | `TEMPLATE_NOT_FOUND` | 500 |
| A body selector was declared and the body is not valid JSON | `BODY_EXTRACTION_ERROR` | 500 |
| The body exceeded `body_limit` | `UPLOAD_TOO_LARGE` | 413 |
