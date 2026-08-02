# Getting started

## Build

Doppel is a Rust workspace. The toolchain is pinned in `rust-toolchain.toml`,
so rustup will fetch the right one:

```bash
cargo build --release
```

The binary is `target/release/doppel`.

## A minimal configuration

Save this as `main.yaml`:

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
  - name: backend
    type: http
    url: "https://api.example.com/v1/"
    resolve:
      type: default
```

The `admin` block is required. Empty `tokens` and `access` mean the API is
reachable only under the default access rules, which grant every action to the
`admin` group -- so with no tokens configured, nothing can call it. See
[the admin API](admin-api.md).

## Check it before running it

```bash
doppel config validate --config main.yaml
```

This reports every problem it finds, not just the first, each with the path in
the configuration that produced it:

```
proxies[0].latency.min: min must be <= max
admin.upload.limit: upload limit must be greater than 0
```

It exits `0` when the configuration is valid, `1` when it is not. It touches
nothing on disk, so it gives the same answer on a laptop as in production.

## Run it

```bash
doppel serve --config main.yaml
```

Requests to `http://127.0.0.1:8080/users/1` are forwarded to
`https://api.example.com/v1/users/1`. Logs go to stdout as one JSON object per
line.

## Make it misbehave

Add faults to the proxy:

```yaml
    loss:
      percentage: 0.1    # drop one request in ten
      status: 503
    latency:
      percentage: 0.5    # delay half of them
      min: 0.2           # seconds
      max: 1.0
```

Reload without restarting:

```bash
doppel config reload
```

A reload is all or nothing. If the new configuration is invalid it is rejected
whole, the running one keeps serving, and the command prints what was wrong:

```
reload rejected: CONFIG_INVALID
proxies[0].latency.min: min must be <= max
```

!!! note "What a reload can and cannot change"
    Reloading applies changes to `proxies`. Changes to `server`, `logging`,
    `control`, `templates` or `admin` are accepted and reported as needing a
    restart -- the reload response names them, so you are not left believing a
    change took effect when it did not.

## Replace an endpoint

Add a mock to the proxy and Doppel answers that request itself instead of
forwarding it:

```yaml
    mocks:
      - name: one-user
        request:
          method: GET
          url: /users/1
        response:
          status: 200
          json: '{"id": 1, "name": "Ada"}'
          headers:
            Content-Type: application/json
```

Everything else still goes upstream. See [Mocks and templating](mocks.md) for
variables, templates, and the ordering rule that matters once you have more
than one mock.
