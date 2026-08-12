# Admin API

A second HTTP listener on `admin.host:admin.port`, separate from the proxy
because the proxy's fallback handler swallows every path -- there is nowhere
on it an admin route could live.

`admin.enable: false` turns all of it off, including `/api/v1/status` and `/metrics`,
and the port is then never bound. See
[the configuration reference](configuration.md#admin).

Everything the API writes goes through the configuration store. No handler
touches the filesystem, which is what makes the PostgreSQL store a matter of
constructing a different store rather than rewriting handlers.

## Authentication

A token arrives in the header named by `admin.auth.header`, default
`X-Proxy-Authorization`, as `Bearer {token}`, and must match one entry of
`admin.tokens`.

`doppel token add` issues one on a running server; see the
[CLI reference](cli.md#token-add).

```bash
curl -H 'X-Proxy-Authorization: Bearer c0a721e2-...' \
     http://localhost:8081/api/v1/proxies
```

An absent token and an unrecognised one are both anonymous. They are not
distinguished on purpose: telling them apart would confirm which tokens
exist, and both answer `401`.

Every request is judged against the configuration the last reload put into
effect, never against whatever the store holds at that moment. That is what
stops someone who can write the configuration out of band, but holds no token,
from granting themselves one and using it immediately. Handlers read and write
their *data* through the store; only the policy comes from the running
configuration.

The comparison against a configured token does not stop at the first differing
byte, so how long a rejection takes does not depend on how much of a guess was
right. Token length is not hidden -- which is why the accepted range is
published (see [Tokens](configuration.md#tokens)) rather than treated as a
secret.

`401` and `403` *are* distinguished, because a caller fixes them differently:
`401` means "send a token", `403` means "that token is not enough".

## Access control

Six actions: `list`, `read`, `create`, `update`, `delete`, `upload`. Each maps
to `public`, one token or group name, or a list of them.

**Every action defaults to the `admin` group, reads included.** A proxy
document carries the `headers` that proxy injects upstream and a `url` that
may contain `user:password@`, so listing proxies publishes credentials. See
[the configuration reference](configuration.md#admin) for the full reasoning.

`read`, `update`, `delete` and `upload` may be overridden per proxy. `list`
and `create` may not -- they are not about one proxy.

Authorization is decided **before** existence. A caller who may not read a
proxy gets the same answer whether or not it exists, so `404` versus `403`
cannot be used to enumerate proxy names.

## Revisions

Every proxy has a revision: sixteen hex digits derived from its content. Two
identical proxies have the same revision, and reformatting the configuration
file does not change it.

List and read return it in the body and in an `ETag`. An update must send it
back, in `If-Match` or as the body's `revision` field:

```bash
# read
curl -sD- http://localhost:8081/api/v1/proxies/proxy1
# ETag: "59df43ad3b02dcf3"

# update
curl -X PUT http://localhost:8081/api/v1/proxies/proxy1 \
     -H 'If-Match: "59df43ad3b02dcf3"' \
     -H 'Content-Type: application/json' \
     -d '{"proxy": {"name": "proxy1", "type": "http", "url": "https://new.example.com/"}}'
```

| Situation | Answer |
|---|---|
| The revision matches | `200`, with the new revision |
| It does not | `409` `REVISION_MISMATCH` -- re-read and retry |
| None was sent | `428` `REVISION_REQUIRED` |
| `If-Match` and the body disagree | `400` `CONFIG_INVALID` |
| `If-Match: *` | `428` -- see below |

`If-Match: *` is refused rather than honoured. RFC 9110 reads it as "if the
resource exists", which here means "overwrite whatever is there" -- the exact
lost update the precondition exists to prevent.

Delete accepts `If-Match` but does not require it. A delete names its target
completely and overwrites no unread fields, so there is no lost update to
prevent.

### Concurrent edits

The store's compare-and-swap token covers the whole configuration, so an edit
to an unrelated proxy invalidates it too. The handler absorbs that: it
re-reads, re-checks only the revision the client sent, and retries, up to four
attempts.

So two clients editing *different* proxies both succeed, and two editing the
*same* one produce a `409` for the loser. Sustained contention that exhausts
the four attempts answers `409` `CONFLICT` -- not `REVISION_MISMATCH`, because
the client's revision was current every time it was checked and re-reading
would not help.

## Endpoints

| Method | Path | Action | Success |
|---|---|---|---|
| GET | `/api/v1/proxies` | `list` | `200` |
| POST | `/api/v1/proxies` | `create` | `201` + `Location` |
| GET | `/api/v1/proxies/{name}` | `read` | `200` + `ETag` |
| PUT | `/api/v1/proxies/{name}` | `update` | `200` + `ETag` |
| DELETE | `/api/v1/proxies/{name}` | `delete` | `204` |
| GET | `/api/v1/proxies/{name}/templates` | `read` | `200` |
| POST | `/api/v1/proxies/{name}/templates/{file}` | `upload` | `204` |
| DELETE | `/api/v1/proxies/{name}/templates/{file}` | `upload` | `204` |
| POST | `/api/v1/config/reload` | `update` | `200` |
| GET | `/api/v1/access` | none | `200` |
| GET | `/api/v1/schema` | none | `200` |
| GET | `/api/v1/status` | none | `200` |
| GET | `/metrics` | none | `200` |
| GET | `/api/openapi.json` | none | `200` |
| GET | `/api/swagger-ui` | none | `200` |

A trailing slash is accepted everywhere: `/metrics/` answers as `/metrics` does.
Axum stopped redirecting between the two spellings, and a 404 for a slash is a
poor answer for a path an operator typed. The proxy listener is deliberately not
like this -- a proxied path is relayed byte for byte, because `/orders/` and
`/orders` are two resources upstream.

`/api/v1/status`, `/api/openapi.json` and `/api/swagger-ui` sit outside `/api/v1`
because they are not resources of the API; they describe or observe the process.
`/metrics` sits outside `/api/` altogether, because that path is what every
scraper looks for by default.

Everything the API serves is under `/api/`. That is a boundary rather than a
convention: with `admin.dashboard` on, a `GET` outside `/api/` and `/static/` is
answered with the dashboard's page, so a client-side route survives being reloaded
or bookmarked. Inside `/api/` an unknown path still answers `404` in the envelope,
so a mistyped endpoint cannot hand a client an HTML document to parse, and a `POST`
to a path that does not exist is refused rather than answered with a page.

The dashboard also serves `/robots.txt` and its hashed assets under `/static/`.
None of that is in the OpenAPI document, which describes the API; see
[The dashboard](dashboard.md).

### What the caller may do

`GET /api/v1/access` reports the calling token's own rights. It answers `200` for
everybody, anonymous included: an endpoint whose purpose is to say "you may do
nothing" cannot itself require a right, and it discloses nothing that attempting
the six actions would not.

```json
{
  "caller": { "kind": "token", "name": "ci", "group": "ops" },
  "global": {
    "list": true, "read": true, "create": false,
    "update": false, "delete": false, "upload": false
  },
  "proxies": {
    "alpha": { "read": true, "update": false, "delete": false, "upload": false }
  }
}
```

- `caller` is `{"kind": "anonymous"}` for a request with no token, or with one
  this configuration does not recognise -- the two are deliberately not
  distinguished.
- `global` is the six actions as `admin.access` decides them for this caller.
- `proxies` gives the four overridable actions per proxy, with that proxy's own
  `access` block applied. It is **absent** -- not empty -- when the caller may not
  `list`: the map is keyed by proxy name, so returning it would be a proxy listing
  by another route.

Every value is the same decision the endpoint itself would make, evaluated by the
same code. It exists so a client can disable an action instead of offering it and
being refused; it is not a substitute for handling `401` and `403`, since rights
can change under a running page.

### The configuration schema

`GET /api/v1/schema` returns the configuration document as a JSON Schema
(2020-12), from the process that is going to read it. No token, for the same
reason as above: it describes the shape of a configuration, never the contents of
one, and the identical bytes are published in the repository and attached to every
release.

```bash
curl -s http://127.0.0.1:8081/api/v1/schema | jq '.["$defs"].ProxyName'
```

```json
{
  "description": "Letters, digits, `-` and `_`, between 2 and 32 characters.",
  "maxLength": 32,
  "minLength": 2,
  "pattern": "^[A-Za-z0-9_-]{2,32}$",
  "type": "string"
}
```

Use it to check a document before pushing it, and to build an editor that knows
the rules: it is generated from the same Rust types that enforce them, so it
cannot describe a field that does not exist or a bound nobody applies. See
[Editor support](configuration.md#editor-support) for the other two copies and
when to prefer which.

### Bodies

Create and update take the same shape, so what a client reads is what it sends
back:

```json
{ "revision": "59df43ad3b02dcf3", "proxy": { "name": "proxy1", "...": "..." } }
```

`revision` is required on update and refused on create -- a revision names a
version of something that already exists, and accepting one on a create would
let a client that meant to send a `PUT` overwrite a proxy it never read.

A `PUT` whose body names a different proxy than the path **renames it**. The name
is also where the templates live, so the rename moves them: the proxy answers at the
new path, the old one is a `404`, and every template file the proxy had is under the
new name. A name another proxy already holds is refused with `400`
`CONFIG_INVALID`.

The configuration is written before the templates are moved, because the write is
what authorises moving them. If the move then fails -- a permission problem, a full
disk -- the reply says so in full: the proxy has the new name and its templates are
still under the old one, so a mock naming a template file fails to render until they
are moved by hand.

## Templates

Upload takes a raw body, not multipart: the resource is one file and its name
is already in the path.

Three checks, in this order:

1. The file name survives the same check a path component gets -- no
   separators, no `..`, no leading dot.
2. Some mock of *that* proxy names it in `response.template`, else `422`
   `TEMPLATE_NOT_DECLARED`. An upload nothing will ever read is a mistake
   worth reporting.
3. The body fits `admin.upload.limit`, else `413` `UPLOAD_TOO_LARGE`.

The order matters: a name nothing declares is refused before the body is read
at all.

Deleting a proxy removes its templates. Updating one removes the templates no
remaining mock names. Both happen *after* the configuration write, so a
rejected change leaves every file in place.

## Reload

`POST /api/v1/config/reload` promotes the stored configuration to the running
one, and answers with the revision now in effect, the proxy count, and any
section that changed but needs a restart:

```json
{ "revision": "a41c0b93e7d25f18", "proxies": 3, "unapplied": ["server"] }
```

`unapplied` is absent when empty. It lists sections `Runtime::compile` never
reads -- `server`, `logging`, `control`, `templates`, `sentry`, `admin` -- so
an operator is told when a change was accepted and stored but is not what the
process is doing.

Every step before the swap can fail; the swap itself cannot. A reload that
rejects the stored configuration leaves the process serving exactly what it
was serving.

This endpoint authorizes against the **running** configuration, unlike the
CRUD handlers, which authorize against the stored one. Each uses the policy
governing what it changes. Reading `admin.access` from the stored document
here would let that document authorise its own promotion: anyone able to write
the configuration file out of band could add a token for themselves and reload
it into effect.

Same effect as `doppel config reload`, and the two share one implementation
and one mutex, so they cannot swap runtimes in the wrong order.

## Status

`GET /api/v1/status` reports what the process is serving right now -- from the
running runtime, not from the store, because a configuration written but not
reloaded is not what this process is doing.

```json
{
  "uptime_seconds": 1043,
  "revision": "a41c0b93e7d25f18",
  "proxies": [
    { "name": "proxy1", "upstream": "https://external-service.com/api/v1/",
      "resolve": "default", "mocks": 6 }
  ]
}
```

Unauthenticated, because it is what a load balancer calls. Upstreams are
printed with any credentials stripped.

## Errors

Every error is the same envelope:

```json
{ "status": "error", "message": "...", "code": "ERROR_CODE" }
```

The code set is closed. A client may switch on it exhaustively, and that
holds for the routes the framework answers too: an unknown path, a wrong verb
and an oversized body all carry the envelope rather than an empty body.

| Code | Status | Meaning |
|---|---|---|
| `CONFIG_INVALID` | 400 | The document, the body or a parameter is not valid |
| `INVALID_REQUEST_PATH` | 400 | A request path that would resolve outside the upstream |
| `UNAUTHORIZED` | 401 | No token, or one this process does not know |
| `FORBIDDEN` | 403 | A known token without the right |
| `NOT_FOUND` | 404 | No such proxy, no such template file, or no such route |
| `METHOD_NOT_ALLOWED` | 405 | The path exists and does not accept that verb; the response also carries `Allow` |
| `PROXY_NOT_RESOLVED` | 404 | No proxy matched and there is no default |
| `NO_PROXIES_CONFIGURED` | 503 | The configuration names no proxies at all. See [No proxies configured](proxying.md#no-proxies-configured) |
| `DASHBOARD_NOT_BUILT` | 503 | `GET /` on a binary compiled without the dashboard's assets. See [When the root answers 503](dashboard.md#when-the-root-answers-503) |
| `CONFLICT` | 409 | The name exists, or the store is under sustained contention |
| `REVISION_MISMATCH` | 409 | The proxy changed since it was read |
| `UPLOAD_TOO_LARGE` | 413 | A template body over `admin.upload.limit`, or a configuration document over 1 MiB |
| `TEMPLATE_NOT_DECLARED` | 422 | No mock names that file |
| `REVISION_REQUIRED` | 428 | An update carried no revision |
| `TEMPLATE_RENDER_ERROR` | 500 | A mock template failed to render |
| `TEMPLATE_NOT_FOUND` | 500 | A mock names a file that is not on disk |
| `BODY_EXTRACTION_ERROR` | 500 | A mock could not read what it needed from the body |
| `STORE_ERROR` | 500 | The configuration store is unavailable |
| `UPSTREAM_ERROR` | 502 | The upstream failed |
| `UPSTREAM_TIMEOUT` | 504 | The upstream did not answer in time |

`CONFLICT` and `REVISION_MISMATCH` share a status but stay distinct codes: an
intermediary cares about the status, and a client acting on the body needs to
tell "that already exists" from "you are holding a stale copy".

## OpenAPI

`GET /openapi.json` serves a document generated from the handlers themselves,
so it cannot describe an endpoint this binary does not serve. `GET
/api/swagger-ui` serves a browser UI over it, with the assets built into the
binary rather than fetched at runtime.

Both are unauthenticated: they describe the API rather than expose any of it,
and a client cannot authenticate before it knows how.
