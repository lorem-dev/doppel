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

## Variables

Four sources, all optional.

**Path captures.** Named groups in the pattern become variables:

```yaml
          url: /api/v1/resource/(?P<resourceId>\d+)/
```

binds `resourceId`.

**Headers.** A map of variable name to header name:

```yaml
          headers:
            requestId: X-Request-ID
```

**Query and body.** A map of variable name to a selector -- a leading dot and
dot-separated keys:

```yaml
          query:
            filter: .filter
          body:
            itemCount: .content.items
```

Selectors address object keys. A selector that lands on an array yields the
array, which templates can iterate or measure with `length`. Array indexing is
not supported.

A variable name may not collide with a capture group name; that is rejected at
load.

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
            requestId: X-Request-ID
        response:
          json: '{"seen": "{{ requestId }}"}'
```

A request without `X-Request-ID` fails this mock with
`TEMPLATE_RENDER_ERROR`, because `requestId` is undefined. That is deliberate:
a mock that silently renders `"seen": ""` because a variable was mistyped is
worse than one that refuses. The error message names the expression that
failed.

If a variable should be optional, say so:

```yaml
          json: '{"seen": "{{ requestId | default('''') }}"}'
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
a mock may name a file that does not exist yet -- a later phase uploads them at
runtime. A missing file at request time is `TEMPLATE_NOT_FOUND`.

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
