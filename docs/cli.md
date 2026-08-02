# CLI reference

```
doppel serve            [--workers <n>] [--config <path>] [--store file|postgres]
                        [--database-url <dsn>] [--config-name <name>]
doppel config validate  [same store flags]
doppel config reload    [--socket <path>] [same store flags]
doppel config push      [--config <path>] --database-url <dsn>
                        [--config-name <name>] [--if-revision <hex>]
doppel config pull      --database-url <dsn> [--config-name <name>]
                        [--output <path>]
doppel config migrate   --database-url <dsn>
doppel version
```

## Exit codes

Scripts depend on these, so they are a contract:

| Code | Meaning |
|---|---|
| `0` | Success |
| `1` | The configuration was rejected, or the command could not do its work |
| `2` | A usage error, from argument parsing |

There is no longer a code for "this build cannot do that". It went with the
refusal it described when the PostgreSQL store landed: an exit code nothing
can produce is a promise a script waits on forever.

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

`--database-url` is required with `--store postgres`, and there is no local
default: a mistyped environment would otherwise connect to whatever database
happened to be at hand. See [Configuration storage](storage.md) for what each
store does.

A database URL is masked wherever it can surface -- in an error message, in a
log line, in a debug formatting of the arguments. An unparseable value is
replaced wholesale rather than echoed, since it may still hold a secret.

## `serve`

Builds the runtime, then loads and validates the configuration, then binds the
proxy port, the admin port and the control socket.

`--workers` / `DOPPEL_WORKERS` sets the tokio runtime's worker threads;
absent, the runtime sizes itself to the machine. It is an argument rather than
a configuration field because the runtime has to exist before a
database-backed store can be opened, and the store is where the configuration
is. `--workers 0` is a usage error: the value is parsed as a non-zero integer,
so zero cannot reach the runtime builder, which would panic on it.

Shutdown on `SIGINT` or `SIGTERM` stops accepting, drains in-flight requests
for up to 30 seconds, removes the control socket and exits `0`. A second signal
exits at once.

## `config push`

Reads a YAML document and writes it into the database. Both ends are named
separately rather than through `--store`, because this command always reads a
file and always writes a database: a store selector would be a flag with one
legal value and a misleading name.

Unconditional by default, which is what provisioning wants. `--if-revision
<hex>` makes it the same compare-and-swap the admin API uses, so a scripted
push cannot overwrite a change made in between; a revision that has moved is
refused and nothing is written. A malformed `--if-revision` is refused before
the database is touched, and is not reported as a mismatch -- a typo is not a
stale copy, and the two need different fixes.

An invalid document is refused with every violation listed, and writes
nothing.

## `config pull`

Reads the database and writes canonical YAML to `--output`, or to stdout when
there is none, so `doppel config pull > main.yaml` produces a file `push`
accepts unchanged. Comments and layout are not preserved; they are no part of
what the database stores.

Pulling a name that is not there fails without creating the output file.

## `config migrate`

Applies any migrations the database has not seen, and reports how many ran.
Safe to run twice. Never run at startup -- see
[Configuration storage](storage.md#migrations).

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
