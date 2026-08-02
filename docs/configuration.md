# Configuration reference

One YAML document. Unknown keys are an error, not a silently ignored setting --
a mistyped field name fails at load rather than doing nothing at runtime.

`main.example.yaml` in the repository is this reference made concrete, and is
asserted against by the test suite.

## Top level

| Key | Required | Purpose |
|---|---|---|
| `server` | yes | Where the proxy listens |
| `admin` | yes | Admin API settings, parsed and validated now, served in a later phase |
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
  workers: 4
```

| Key | Type | Default | Notes |
|---|---|---|---|
| `host` | IP address | required | Must parse as an IP, not a hostname |
| `port` | 1..65535 | required | Must differ from `admin.port` |
| `workers` | integer >= 1 | available parallelism | tokio runtime worker threads |

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

Parsed and validated now; the API itself arrives in a later phase. A
configuration written today stays valid when it does.

```yaml
admin:
  host: "0.0.0.0"
  port: 8081
  workers: 1
  auth:
    header: X-Proxy-Authorization
  tokens:
    - name: user1
      group: admin
      token: c0a721e2-90ff-40f0-a230-c1ab83d751d8
  access:
    list: public
    read: public
    create: ["admin"]
    update: user1
    delete: admin
    upload: admin
  upload:
    limit: 1M
```

`auth.header` defaults to `X-Proxy-Authorization` and expects `Bearer {token}`.

Each entry in `tokens` has a unique `name` and a unique `token`. `group` is
free-form; `admin` and `user` are predefined, and any other name must be
carried by at least one token -- referencing a group nobody belongs to locks the
action out entirely, which is far more often a typo than an intent.

`access` maps each action to `public`, a single name, or a list of names. An
empty list means public. Names are token names or group names.

`upload.limit` accepts `4096`, `512K`, `1M`, `2G`. It must be greater than
zero.

## `proxies`

At least one. Names must be unique.

```yaml
proxies:
  - name: proxy1
    type: http
    url: "https://external-service.com/api/v1/"
    timeout: 60
    body_limit: 1M
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

Thirty-three rules run identically at startup, on reload, and under
`doppel config validate`. Every violation is reported together with the others,
each carrying the configuration path that produced it:

```
proxies[0].latency.min: min must be <= max
proxies[1].resolve.header: `header` is required when `type: header`
```

Four rules are enforced by the types rather than by a rule: a host that is not
an IP, an unknown log level or format, and a proxy `access` block naming an
action it may not override all fail while the document is being parsed. Those
stop at the first error, because parsing does; everything the rule set checks
is collected and reported together.

For the full list of rules and their identifiers, see the phase 1 design
specification in the repository.
