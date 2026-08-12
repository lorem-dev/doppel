# Configuration reference

One YAML document. Unknown keys are an error, not a silently ignored setting --
a mistyped field name fails at load rather than doing nothing at runtime.

`main.example.yaml` in the repository is this reference made concrete, and is
asserted against by the test suite.

## Editor support

The configuration has a JSON Schema, so an editor can complete field names, show
what each field is for and mark a bad value as you type -- before Doppel is run
at all. Put this line at the top of your `main.yaml`:

```yaml
# yaml-language-server: $schema=https://raw.githubusercontent.com/lorem-dev/doppel/main/doppel-config.schema.json
```

VS Code's YAML extension reads it, as does any other `yaml-language-server`
client. `main.example.yaml` already carries it.

That URL follows `main`. Every release also attaches the schema as an asset, so
a deployment that pins a version can validate against the schema for exactly
that version rather than for whatever is current.

The schema is generated from the same Rust types this page documents -- see
[`doppel config schema`](cli.md#config-schema) -- so it cannot describe a field
that does not exist, and CI fails if the checked-in copy falls behind.

What it catches: an unknown key, `percentage: 45` where a fraction was meant,
`method: get` in lower case, a port of `0`, a missing `url`. What it cannot
catch: anything needing more than one field, such as `min <= max`. Those are the
[validation rules](#validation) below, and they run when the configuration is
loaded.

## Top level

| Key | Required | Purpose |
|---|---|---|
| `server` | yes | Where the proxy listens |
| `admin` | yes | Admin API settings |
| `proxies` | no | Empty or absent is legal; requests then get `503` |
| `logging` | no | Level and format; defaults to `info` and `json` |
| `control` | no | Control socket path; defaults to `/tmp/doppel.sock` |
| `templates` | no | Template directory; defaults to `./templates` |
| `sentry` | no | Absent or an empty DSN disables it |

## `server`

```yaml
server:
  host: "0.0.0.0"
  port: 8080
```

| Key | Type | Default | Notes |
|---|---|---|---|
| `host` | IP address | required | Must parse as an IP, not a hostname |
| `port` | 1..65535 | required | Must differ from `admin.port` |

Worker threads are **not** configured here. They size the tokio runtime, and a
database-backed store cannot be opened before that runtime exists -- so the
value has to be known before the configuration is read, which puts it on the
same side of the boundary as the connection settings. Use `--workers` or
`DOPPEL_WORKERS`; see the [CLI reference](cli.md#serve). A document still
carrying `server.workers` is rejected as an unknown field rather than quietly
ignored.

## `logging`

```yaml
logging:
  level: info
  format: json
```

`level` is one of `trace`, `debug`, `info`, `warn`, `error`; `format` is `json`
or `text`. `RUST_LOG` overrides `level` when set and non-empty.

## `control`

```yaml
control:
  socket: /tmp/doppel.sock
```

The Unix socket `doppel config reload` talks to. Created with mode `0600` and
removed on shutdown. Its parent directory must exist -- that is checked when
`serve` starts, not by `config validate`, which stays independent of the
machine it runs on.

## `templates`

```yaml
templates:
  dir: ./templates
```

Template files live at `<dir>/<proxy-name>/<file>`. Created at startup if
absent.

## `sentry`

```yaml
sentry:
  dsn: "https://key@sentry.example.com/1"
```

Optional. An absent section or an empty DSN disables it.

## `admin`

The admin listener's address, its tokens, and who may do what.

```yaml
admin:
  enable: true
  dashboard: true
  title: "Doppel"
  host: "0.0.0.0"
  port: 8081
  public: false
  groups: ["*"]
  auth:
    header: X-Proxy-Authorization
  tokens:
    - name: user1
      group: admin
      token: c0a721e2-90ff-40f0-a230-c1ab83d751d8
  access:
    list: ["admin", "user"]
    read: ["admin", "user"]
    create: ["admin"]
    update: user1
    delete: admin
    upload: admin
  upload:
    limit: 1Mi
```

`enable` defaults to `true`. Set it to `false` to run the proxy with no admin
application at all: the port is never bound, so it cannot collide with
anything, and `/api/v1/status`, `/api/v1/metrics`, `/api/openapi.json` and the whole API are
gone with it. The proxy listener and the control socket are untouched, which
makes `doppel config reload` the only remaining way in.

The validation rules do not consult it. A configuration that is only safe
because nothing serves it is a trap set for whoever turns the listener on
later, and they will not re-read the rules first -- so rule V34 still refuses
a public write action with the listener off.

`dashboard` defaults to `true` and serves the browser dashboard from this
listener's root: `/`, `/static/{path}` and `/robots.txt`. `false` leaves those
three unrouted -- they answer 404 like any other unknown path -- and changes
nothing about the JSON API. `title` is the heading that dashboard shows and the
browser tab's name, at most 64 characters and free of control characters,
defaulting to `Doppel`. Both take effect on restart, since the routes are built
once. See [The dashboard](dashboard.md).

Toggling it takes effect on restart, not on reload; a reload reports `admin`
among the sections it could not apply.

`auth.header` defaults to `X-Proxy-Authorization` and expects `Bearer {token}`.

Each entry in `tokens` has a unique `name` and a unique `token`. `group` is
free-form; `admin` and `user` are predefined, and any other name must be
carried by at least one token -- referencing a group nobody belongs to locks the
action out entirely, which is far more often a typo than an intent.

`access` maps each action to `public`, a single name, or a list of names. An
empty list means public. Names are token names or group names.

### `public`: serve the admin API unauthenticated

```yaml
admin:
  public: true
```

Every action answers as `public`, for anyone, with no token. `false` by default.

This overrides rule V34, which otherwise refuses a public write action. V34 is
there so an unauthenticated writable proxy set cannot happen by *omission*; a
field called `public` set to `true` is not an omission.

Anything `access`, `groups` or a proxy's overrides still say is ignored, and
startup says so rather than refusing the document -- so a configuration can be
made public temporarily without being gutted first:

```
admin.public is true: the whole admin API is served unauthenticated, including
the actions that rewrite the proxy set
admin.access, admin.groups are ignored while the admin API is public; every
action answers as `public` regardless
```

### `groups`: which names `access` may reference

```yaml
admin:
  groups: ["*"]     # the default
```

Bounds the vocabulary `access` may draw on, here and in a proxy's overrides.
Rule **V36**.

| `groups` | `access` may name |
|---|---|
| absent, or `["*"]` | anything. The default |
| `["admin", "ci"]` | `admin`, `ci`, and nothing else -- `user` is refused |
| `[]` | nobody, which is the same as `public: true`. See below |

`public` and `admin` are always available whatever the list says. `public` is the
absence of a subject rather than a name; `admin` is the fallback every action
already has, and a list that revoked it would leave the four write actions with
no legal value at all.

`groups: []` names nobody, so nothing can be granted to anyone -- which leaves
all-public as the only reading of it that describes a configuration that runs. It
is treated as `public: true`, and startup says which of the two you wrote.

The violation names everything permitted, because the reader has to choose
between changing the reference and widening the list:

```
admin.access.read: `user` is not an allowed group; `admin.access` may name only `admin`, `ci`, `public`
```

Two things `groups` does not do. It does not create groups -- a name still has to
be predefined or carried by a token, which is rule V27, reported separately
because widening the list and adding a token are different fixes. And it is not
authorisation: it bounds what a *configuration* may say, and a caller's rights
still come from `access`.

`POST` and `PUT /api/v1/proxies` refuse a document whose `access` names something
outside the list, with `400` and `CONFIG_INVALID`.

Every action defaults to the `admin` group, reads included. The most common
configuration is the one nobody wrote, so the default has to be the safe one.

Reads are not exempt: a proxy document carries the `headers` that proxy
injects upstream, and `url` may itself contain `user:password@`. Listing
proxies therefore publishes upstream credentials, which is no lesser harm
than rewriting the proxy set. Setting `list` or `read` to `public` is
allowed -- a configuration with no secrets in it may reasonably do so -- but
it has to be a choice, not what happens when the section is left out.

Setting a *write* action to `public` is refused outright (rule V34); no
configuration wants an unauthenticated caller rewriting the proxy set.

`GET /api/v1/status` stays unauthenticated regardless: it reports names, upstreams
and counts, and strips any credentials from the upstream before printing
it.

`upload.limit` is a byte count. It must be greater than zero.

Binary and decimal units are both accepted and mean different things:
`Ki`/`Mi`/`Gi` (equivalently `KiB`/`MiB`/`GiB`) are powers of 1024, and
`kB`/`MB`/`GB` are powers of 1000. A plain number is bytes. Case is not
significant on input.

A bare `K`, `M` or `G` is **refused**. It meant the binary unit in earlier
versions, which contradicts SI, and quietly reinterpreting it as decimal would
have shrunk every configured buffer by a few percent with nothing to show for
it. The error names both replacements, so the fix is a one-character edit and
the choice is the operator's.

## `proxies`

Names must be unique. The list may be empty or left out -- see
[No proxies configured](proxying.md#no-proxies-configured).

```yaml
proxies:
  - name: proxy1
    type: http
    url: "https://external-service.com/api/v1/"
    timeout: 60
    body_limit: 1Mi
    resolve:
      type: default
    access:
      read: ["user1", "user2"]
    headers:
      Authorization: "Bearer 1234567890"
    loss:
      percentage: 0.1
      status: 503
    latency:
      percentage: 0.45
      min: 0.05
      max: 0.2
    replace: 1.0
    mocks: []
```

| Key | Type | Default | Notes |
|---|---|---|---|
| `name` | string | required | Unique; also used as the template subdirectory |
| `type` | `http` | required | The only value. `tcp` is refused while parsing, with a message saying it is not implemented |
| `url` | absolute URL | required | `http` or `https`, no query or fragment |
| `timeout` | seconds > 0 | 30 | Bounds the whole upstream exchange |
| `body_limit` | byte size > 0 | 1 MiB | Only used when a matched mock extracts from the body |
| `resolve` | see below | `{type: default}` | |
| `access` | overrides | none | Only `read`, `update`, `delete`, `upload` |
| `headers` | map | none | Injected upstream, overriding the client's |
| `loss` | see below | none | |
| `latency` | see below | none | |
| `replace` | 0.0..1.0 | 1.0 | Probability a matching mock actually answers |
| `rewrite_redirects` | boolean | `true` | Point a redirect back at Doppel when its target is under this proxy's base. See [Redirects](proxying.md#redirects) |
| `mocks` | list | none | See [Mocks and templating](mocks.md) |

### `resolve`

```yaml
    resolve:
      type: header
      header: X-Proxy-Name
```

`type` is `default` or `header`. At most one proxy may be the default; zero is
legal and means every request must resolve by header. A `header` resolver must
name a valid header.

### `loss` and `latency`

```yaml
    loss:
      percentage: 0.1
      status: 503
    latency:
      percentage: 0.45
      min: 0.05
      max: 0.2
```

Percentages are fractions in `0.0..1.0`; `0.0` never fires and `1.0` always
does. `status` is a real HTTP status. `min` and `max` are seconds, both
non-negative, with `min <= max`.

### `body_limit`

A mock that extracts variables from the request body has to buffer it, which
the proxy otherwise avoids -- bodies stream through. This bounds that buffer.
Exceeding it is `413`. See [Mocks and templating](mocks.md#bodies-and-the-size-limit).

### `mocks[]`

```yaml
    mocks:
      - name: pricing
        request:
          method: GET
          url: "^/pricing/(?P<id>[0-9]+)/$"
          headers:
            who: X-User
          query:
            page: .page
          body:
            items: .content.items
        response:
          status: 200
          json: '{"id": "{{ id }}", "page": "{{ page }}"}'
          headers:
            X-Served-By: "mock {{ id }}"
        proxy:
          replace: 0.5
```

| Key | Type | Default | Notes |
|---|---|---|---|
| `name` | string | required | Unique within the proxy |
| `request` | see below | required | What the mock matches |
| `response` | see below | required | What it answers |
| `proxy` | see below | none | Per-mock overrides |

`request`:

| Key | Type | Default | Notes |
|---|---|---|---|
| `method` | upper-case method | required | Matched exactly; `get` is rejected at load |
| `url` | regex | required | Matched against the path, unanchored. Named groups become variables |
| `headers` | variable → header name | none | |
| `query` | variable → selector | none | |
| `body` | variable → selector | none | Buying the buffer bounded by `body_limit` |

`response` -- exactly one of `body`, `json` or `template`:

| Key | Type | Default | Notes |
|---|---|---|---|
| `status` | 100..599 | required | |
| `body` | template | none | Sent as `text/plain` |
| `json` | template | none | Sent as `application/json`; must render to valid JSON |
| `template` | file name | none | A file under this proxy's template directory |
| `headers` | header name → template | none | The value is a template, rendered per request |

`proxy` accepts `replace`, `loss` and `latency`, with the same types and bounds
as on the proxy. What is inherited and what is not is in
[Injecting faults](faults.md#faults-on-one-endpoint-only).

Every variable a template names has to be bound, or the render fails with
`500` -- see [Mocks and templating](mocks.md#rendering-is-strict).

## Validation

The rule set runs identically at startup, on reload, and under
`doppel config validate`. Every violation is reported together with the others,
each carrying the configuration path that produced it:

```
proxies[0].latency.min: min must be <= max
proxies[1].resolve.header: `header` is required when `type: header`
```

Rules carry stable `V<n>` numbers so a message can be looked up, and a number
is never reused once retired. Several things a rule would otherwise check are
instead enforced by the types: a host that is not an IP, an unknown log level
or format, a name (below), and a proxy `access` block naming an action it may
not override all fail while the document is being parsed. Those stop at the
first error, because parsing does; everything the rule set checks is collected
and reported together.

**V34** is worth naming because it refuses a configuration earlier versions
accepted: `admin.access` may not grant `create`, `update`, `delete` or
`upload` to `public`. No configuration wants an unauthenticated caller
rewriting the proxy set.

### Retired rules

Most of the original rule set is gone, not because the checks were dropped but
because they moved into the types and now run while the document is being
parsed. A message quoted in an old issue can be looked up here.

| Rule | Was | Now |
|---|---|---|
| V2 | `server.host` is an IP | `IpAddr` |
| V3 | `server.workers` is positive | the field is `--workers` |
| V4 | log level and format are known | `LogLevel`, `LogFormat` |
| V5 | at least one proxy is configured | nothing -- an empty list is legal, see [No proxies configured](proxying.md#no-proxies-configured) |
| V7 | `type: tcp` is refused | `ProxyKind`, while the document is parsed |
| V8, V32 | upstream url is absolute http(s), no query | [`UpstreamUrl`](#upstream-urls) |
| V9 | timeout is positive | [`TimeoutSeconds`](#numbers-with-units) |
| V12, V13 | probability in 0..=1, status in 100..=599 | [`Ratio`](#numbers-with-units), [`HttpStatus`](#methods-and-statuses) |
| V14 (part) | latency bounds are non-negative | [`Seconds`](#numbers-with-units) |
| V15, V24 | header names and values are well formed | [`HeaderName`, `HeaderValue`](#headers) |
| V17 | method is known and upper case | [`HttpMethod`](#methods-and-statuses) |
| V18 | mock url pattern compiles | [`Pattern`](#mock-patterns-and-selectors) |
| V22 | response status in 100..=599 | [`HttpStatus`](#methods-and-statuses) |
| V23 | selector is well formed | [`Selector`](#mock-patterns-and-selectors) |
| V28 | proxy `access` overrides a permitted action | `ProxyAccessConfig` |
| V29, V33 | size limits are positive | [`ByteSize`](#sizes) |
| V31 | template file name is safe | [`TemplateName`](#template-file-names) |
| V35 | proxy name is a usable directory name | [`Name`](#names) |

A retired number is never reused.

Fifteen rules remain: V1, V6, V10, V11, V14, V16, V19, V20, V21, V25, V26, V27,
V30, V34 and V36. Each needs more than one field to decide, which is exactly why
none of them could become a type -- V36, the newest, compares `access` against
`admin.groups`.

## Names

A proxy name, a mock name, a token name and a group name follow one rule:
letters, digits, `-` and `_`, between 2 and 64 characters. A **proxy** name is
capped at 32 instead.

`.` is not allowed. It was until 0.3.0, and the reference configuration taught
names like `Billing.API.v2`; write `Billing-API-v2`. Dropping it removed two
further rules with it -- a name becomes a directory component, so `.hidden` and
`..` each had to be refused separately, and neither can now be written at all.

A proxy name is capped shorter because it travels further than any other: a
directory under `templates.dir`, a `proxy` label on every metric, a field in
every log line, and the value a client puts in a resolution header on every
request.

The rule is enforced by the type, while the document is being parsed, rather
than by a validation rule afterwards. A name becomes a directory component, a
metric label, a log field and part of a URL, so the moment it comes into
existence is the only place worth checking it -- and there is then no later
moment at which an unchecked name exists. There used to be a rule V35 doing
this; it is gone, because a type that admits a bad value and a rule that
catches it later are two things to keep in step.

## Template file names

A mock's `response.template` names one file under
`<templates.dir>/<proxy>/`: one path component, no separators, no leading dot,
no `..`, no control characters, at most 200 bytes.

That is the same rule the admin API applies to an uploaded file name, and it
is now literally the same code. Rule **V31** used to restate it for the
configuration side; a configuration and an upload could have drifted apart
about what a file name is, and only one of them would have been checked.

Names are refused rather than normalised. Rewriting a path an operator asked
for is worse than refusing it: they then believe a file landed somewhere it
did not.

## Mock patterns and selectors

A mock's `request.url` is a regular expression, matched unanchored against the
request path. Its named capture groups become template variables. It is
compiled when the document is read and kept compiled, so the request path
never compiles it again.

A query or body selector is a leading dot followed by dot-separated field
names: `.filter`, `.content.items`. It addresses object keys only; a segment
reaching an array yields the array, and a missing key binds nothing rather
than failing the request.

Rules **V18** and **V23** did these checks. Both are types now, and both
removed a second copy of the same work: `Runtime::compile` used to recompile
every pattern, and the render path used to re-parse every selector on every
request -- each with its own failure branch for input a loaded configuration
could not contain.

## Headers

Every field naming an HTTP header -- `admin.auth.header`, a proxy's
`resolve.header`, the keys of `proxies[*].headers`, the values of a mock's
`request.headers`, and the keys of a mock's `response.headers` -- is a header
name: a non-empty RFC 9110 token. Case is kept as written; matching against an
incoming request folds case at that point instead.

The values of `proxies[*].headers` are header values: visible ASCII, plus
space and tab. A line break is refused with that named, because one would end
the header early and let the rest be read as headers of its own.

Rules **V15** and **V24** did part of this. Between them they left a gap: a
mock's *response* header names were checked by neither, so `X Id:` was a
configuration that loaded, validated, and then produced a header no client
could parse. The types close it, because they are the same types everywhere.

What remains of **V11** is the part that needs two fields: `resolve.header` is
required when `resolve.type` is `header`.

## Upstream URLs

A proxy's `url` is an absolute `http` or `https` URL with no query string and
no fragment. Rules **V8** and **V32** did those checks; the type does them
now, and keeps the URL parsed rather than as text, so the forwarding path
never parses it again.

The query and fragment are refused rather than ignored because the forwarding
path replaces the query wholesale with the incoming request's. One configured
here would be discarded on every request, silently, which is worse than being
told at startup.

The stored form is normalised: `https://example.com` is written back as
`https://example.com/`. That is what makes two spellings of the same upstream
produce the same revision.

A URL may carry `user:password@`. It is not refused -- some upstreams want it
-- but it is logged once at startup, because that credential is part of the
proxy document `GET /api/v1/proxies` returns.

## Sizes

`admin.upload.limit` and a proxy's `body_limit` are byte counts, from 1 byte
to 1 GiB. Both bound something Doppel holds in memory, so a number past that
is not a larger limit but the absence of one.

Write a plain integer, or a suffix: `Ki`, `Mi`, `Gi` are binary (1024-based)
and `kB`, `MB`, `GB` are decimal (1000-based). A bare `K`, `M` or `G` is
refused rather than guessed at -- it meant the binary unit in this project's
own past, which contradicts SI, and reinterpreting it silently would resize
every configured buffer without a word. The message names both replacements.

A limit of 0 is refused: it rejects everything, which shows up as a confusing
413 on every request rather than as the configuration mistake it is. Rules
**V29** and **V33** said that once per field and are gone.

Whatever spelling is used, the value is written back as a plain integer, so
`1Mi` and `1048576` produce the same revision.

## Numbers with units

| Field | Type | Range |
|---|---|---|
| `loss.percentage`, `latency.percentage`, `replace` | probability | 0.0 to 1.0 |
| `latency.min`, `latency.max` | seconds | 0 to 300 |
| `timeout` | whole seconds | 1 to 3600 |

The probabilities are fractions despite the field name: 50% is `0.5`. Writing
`50` is refused, and the message says so, because the old rule reported it as
"out of range" and left the reader to work out which range.

The upper bounds are sanity bounds, not protocol ones. A latency past five
minutes outlives every client that would wait for it, and a timeout past an
hour is a value written in milliseconds far more often than it is an intent --
`timeout: 30000` is refused with that named.

A timeout of 0 is refused rather than read as "no timeout": leave `timeout`
out to get the default.

Rules **V9**, **V12**, **V13** and the sign half of **V14** did these checks
and are gone. What is left of V14 is the ordering: `min` must not exceed
`max`, which needs both fields and so cannot be a type.

## Methods and statuses

A mock's `request.method` is one of the methods Doppel knows, spelled in upper
case. The list is a typo guard, not a protocol restriction: `FETCH` is refused
because it is far more often a mistake than an intent, and a genuinely
non-standard method needs adding to the list. The value is not upper-cased for
you -- HTTP methods are case-sensitive, so a stored `get` would never match an
incoming `GET`, and a document writing it is told to write `GET` instead.

A status -- `response.status`, and `loss.status` -- is a number from 100 to
599. Rules **V17** and **V22** did these checks and are gone; the types do
them now, at parse time.

Rule **V30** still applies: 204 and 304 forbid a body, so a mock declaring one
alongside either status is refused. That compares the status against what the
response declares, which is two fields.

## Ports

`server.port` and `admin.port` are numbers from 1 to 65535. Port 0 is refused
by the type, with a message saying why: to the operating system it means "any
free port", so a configuration naming it describes a server whose address
nobody can predict -- including whoever wrote the line.

Rule **V1** is what remains: the two listeners must not share a port. That
compares two fields, so no single value can be checked for it.

A port below 1024 is accepted and logged once at startup, because binding one
usually needs elevated privilege. It is not an error -- running on port 80
behind a capability or a redirect is a real deployment -- but the far more
common cause is a typo, and the failure that produces otherwise is a bare
`Permission denied` from `bind`.

## Tokens from the environment

`DOPPEL_ADMIN_TOKENS` supplies admin tokens without writing them into the
configuration document. It is a JSON object keyed by token name:

```json
{
  "ci":       {"token": "3f2504e0-4f89-41d3-9a0c-0305e82c3301", "group": "admin"},
  "readonly": {"token": "0a1b2c3d-4e5f-6071-8293-a4b5c6d7e8f9"}
}
```

`group` is optional and defaults to `user`, matching `doppel token add`, and
for the same reason: a variable set by provisioning tooling rather than read
back by a person should not grant administration because a field was left out.

Every field is held to exactly the rules the document is held to -- the name
and group are [names](#names), the value is a [token](#tokens) -- and a
malformed variable **fails startup**. It is not logged and skipped: an
operator who provisioned a token and saw no error believes they have access,
and finding out otherwise happens at the worst possible moment. An unset or
empty variable is simply no tokens, since deployment tooling routinely renders
an empty string for an absent secret.

These tokens are checked before the configured ones. A name given in both
resolves to the environment's group, and the configured token of that name
stops authenticating entirely -- two live secrets for one identity, one of
which nobody remembers issuing, is worse than a replacement. A warning at
startup names each configured token that is shadowed.

`doppel token add` refuses a name the environment claims, because the token it
would generate and store could never authenticate.

They are deliberately **not** merged into the loaded configuration. The
revision is derived from the configuration's content, so folding the
environment in would make two instances reading one stored document compute
two different revisions, and every compare-and-swap between them would fail
over a difference neither had written. One consequence to know: an access list
naming a token that exists only in the environment fails rule V27, because
validation is pure and cannot see it. Name a group instead -- `admin` and
`user` always exist.

## Access lists

`admin.access` and a proxy's `access` name subjects: `public`, one token or
group name, or a list of them. An empty list means public.

The names are names, checked by the same type that checks a token's own name.
An access list can no longer reference something no token could ever be called
-- rule V27 would have reported that as an unknown subject, which is true and
unhelpful.

`public` is a keyword in this position and is never parsed as a name, even
though it happens to be spelled like a legal one.

## Tokens

An admin token is printable ASCII with no spaces, 32 to 255 characters. The
character set is the one an HTTP header value admits, because that is where
the token is read from -- a token containing a space could be written into a
configuration and would then never match anything a client could send.

There is no required form. A version 4 UUID is the recommended shape and what
Doppel generates, but a token pasted out of a secret manager does not have to
be reformatted to be accepted.

Rule **V26** still applies on top of the type: token names must be unique, and
so must token values. Uniqueness is a property of the set rather than of one
token, which is why it stays a rule.

A token is redacted in log lines and in debug output. It is written out in
full by `config pull` and stored in full, since those are the places whose
whole purpose is to reproduce the configuration.

For the full list of rules and their identifiers, see the design
specifications in the repository.
