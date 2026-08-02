# CLI reference

```
doppel serve            [--config <path>] [--store file|postgres]
                        [--database-url <dsn>] [--config-name <name>]
doppel config validate  [same store flags]
doppel config reload    [--socket <path>] [same store flags]
doppel version
```

## Exit codes

Scripts depend on these, so they are a contract:

| Code | Meaning |
|---|---|
| `0` | Success |
| `1` | The configuration was rejected, or a reload failed |
| `2` | An unsupported option, such as `--store postgres` in this build |

## Store flags

Where the configuration lives. These come from the command line and the
environment only, never from the configuration document -- reaching the store
cannot depend on reading the store.

| Flag | Environment | Default |
|---|---|---|
| `--store` | `DOPPEL_CONFIG_STORE` | `file` |
| `--config` | `DOPPEL_CONFIG_PATH` | `./main.yaml` |
| `--config-name` | `DOPPEL_CONFIG_NAME` | `default` |
| `--database-url` | `DOPPEL_DATABASE_URL` | none |

Precedence is command line, then environment, then default.

`--store postgres` exits `2`. The flag exists now so that adding the store
later adds behaviour rather than syntax, and a build that cannot do it says so
instead of pretending.

A database URL is masked wherever it can surface -- in an error message, in a
log line, in a debug formatting of the arguments. An unparseable value is
replaced wholesale rather than echoed, since it may still hold a secret.

## `serve`

Loads and validates the configuration, then binds the proxy port and the
control socket. Validation runs before anything acts on a value, so a bad
`workers` count is a validation error rather than a panic.

`server.workers` sets the tokio runtime's worker threads; absent, the runtime
sizes itself to the machine.

Shutdown on `SIGINT` or `SIGTERM` stops accepting, drains in-flight requests
for up to 30 seconds, removes the control socket and exits `0`. A second signal
exits at once.

## `config validate`

Reports every violation, not just the first, each with its path in the
configuration. Prints them on stdout and exits `1` if there were any.

It performs no filesystem work beyond reading the configuration -- deliberately,
so it answers the same on a developer's machine as in production. Checks that
depend on the machine, such as whether the templates directory can be created,
belong to `serve`.

## `config reload`

Connects to the control socket and asks a running server to reload.

```bash
doppel config reload --socket /tmp/doppel.sock
```

Without `--socket` it reads the socket path from the configuration, which means
it consults the store; with `--socket` it does not, so the store flags are
irrelevant in that case.

A reload is all or nothing: the new configuration is loaded, validated and
compiled before anything is swapped, so a failure leaves the running server
untouched. The response reports the revision now in effect, the number of
proxies, and any configuration section that changed but needs a restart to take
effect.

## `version`

Prints the version and exits `0`.
