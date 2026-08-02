# Configuration reference

One YAML document. Unknown keys are an error, not a silently ignored setting --
a mistyped field name fails at load rather than doing nothing at runtime.

`main.example.yaml` in the repository is this reference made concrete, and is
asserted against by the test suite.

## Top level

| Key | Required | Purpose |
|---|---|---|
| `server` | yes | Where the proxy listens |
| `admin` | yes | Admin API settings |
| `proxies` | yes | At least one proxy |
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
  host: "0.0.0.0"
  port: 8081
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
anything, and `/status`, `/metrics`, `/openapi.json` and the whole API are
gone with it. The proxy listener and the control socket are untouched, which
makes `doppel config reload` the only remaining way in.

The validation rules do not consult it. A configuration that is only safe
because nothing serves it is a trap set for whoever turns the listener on
later, and they will not re-read the rules first -- so rule V34 still refuses
a public write action with the listener off.

Toggling it takes effect on restart, not on reload; a reload reports `admin`
among the sections it could not apply.

`auth.header` defaults to `X-Proxy-Authorization` and expects `Bearer {token}`.

Each entry in `tokens` has a unique `name` and a unique `token`. `group` is
free-form; `admin` and `user` are predefined, and any other name must be
carried by at least one token -- referencing a group nobody belongs to locks the
action out entirely, which is far more often a typo than an intent.

`access` maps each action to `public`, a single name, or a list of names. An
empty list means public. Names are token names or group names.

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

`GET /status` stays unauthenticated regardless: it reports names, upstreams
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

At least one. Names must be unique.

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
| `type` | `http` | required | `tcp` is rejected with a message saying it is not implemented |
| `url` | absolute URL | required | `http` or `https`, no query or fragment |
| `timeout` | seconds > 0 | 30 | Bounds the whole upstream exchange |
| `body_limit` | byte size > 0 | 1 MiB | Only used when a matched mock extracts from the body |
| `resolve` | see below | `{type: default}` | |
| `access` | overrides | none | Only `read`, `update`, `delete`, `upload` |
| `headers` | map | none | Injected upstream, overriding the client's |
| `loss` | see below | none | |
| `latency` | see below | none | |
| `replace` | 0.0..1.0 | 1.0 | Probability a matching mock actually answers |
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

## Names

A proxy name, a mock name, a token name and a group name follow one rule:
letters, digits, `.`, `-` and `_`, between 2 and 128 characters, not starting
with a dot and not containing `..`.

The rule is enforced by the type, while the document is being parsed, rather
than by a validation rule afterwards. A name becomes a directory component, a
metric label, a log field and part of a URL, so the moment it comes into
existence is the only place worth checking it -- and there is then no later
moment at which an unchecked name exists. There used to be a rule V35 doing
this; it is gone, because a type that admits a bad value and a rule that
catches it later are two things to keep in step.

For the full list of rules and their identifiers, see the design
specifications in the repository.
