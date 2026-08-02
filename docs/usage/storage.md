# Configuration storage

Doppel keeps its configuration in one of two places, chosen by `--store`: a
YAML file, or a PostgreSQL database.

Everything above the store is written against one trait, so the choice changes
where a configuration lives and nothing about what the proxy or the admin API
does with it. One conformance suite runs against both, which is what keeps
that true rather than merely intended.

## Choosing a store

| Setting | Argument | Environment | Default |
|---|---|---|---|
| Store | `--store file\|postgres` | `DOPPEL_CONFIG_STORE` | `file` |
| YAML path | `--config` | `DOPPEL_CONFIG_PATH` | `./main.yaml` |
| Database | `--database-url` | `DOPPEL_DATABASE_URL` | none |
| Configuration name | `--config-name` | `DOPPEL_CONFIG_NAME` | `default` |
| Worker threads | `--workers` | `DOPPEL_WORKERS` | available parallelism |

Precedence is argument, then environment, then default.

**None of these can come from the configuration document.** Settings needed to
*reach* the store cannot live inside it, or reading the configuration would
require the configuration. That is also why `--workers` is here rather than
under `server`: the tokio runtime has to exist before a database store can be
opened, and the store is where the document is.

`--database-url` is required with `--store postgres`. There is no local
default, deliberately: a mistyped environment would otherwise connect to
whatever database happened to be at hand.

A database URL is masked wherever it can surface -- an error message, a log
line, a debug formatting of the arguments.

## The file store

One YAML document, and a `templates/<proxy>/` directory beside it.

Writes go through a temporary file and a rename, so a reader sees either the
old configuration or the new one and never half of either. Concurrent writers
are serialised by an advisory lock on a sibling `<config>.lock` file -- a
separate name because `rename` replaces the inode, and a lock held on the
config path itself would stop guarding it the moment a write landed.

## The PostgreSQL store

### Getting a database

```bash
docker compose up -d --wait
export DOPPEL_DATABASE_URL=postgres://doppel:doppel@127.0.0.1:55432/doppel
doppel config migrate
```

`docker-compose.yml` at the repository root runs one PostgreSQL on port
`55432` -- not `5432`, so it cannot collide with a database you already run,
or quietly become one these tools write into.

### Migrations

Built into the binary. A deployment ships one file and needs no `migrations/`
directory beside it.

They are applied only by `doppel config migrate`, never at startup. A process
that silently alters a shared schema when it boots turns a rollback into data
loss, and the operator who rolled back is the one least expecting it. Starting
against a schema that has not been migrated is refused with a message naming
the command.

Running `config migrate` twice is safe; the second run reports that there was
nothing to do.

### Checking what is applied

```bash
doppel config migrate --status
```

Changes nothing, and exits `0` only when every migration this binary carries
is applied, complete, and unchanged since it was applied -- so a deploy gate
can branch on the code without parsing the output. Otherwise it exits `1` and
says which of those three is false.

There is deliberately no separate table holding a single revision number.
It would be the same fact written twice, and two copies of one fact disagree
eventually. sqlx's table already carries what a number cannot: a row per
migration, with a checksum, so it can tell "version 1 is applied" apart from
"version 1 is applied and the file that produced it has since been edited".
`--status` reports the highest applied version, which is the revision number a
single-row table would have held.

sqlx records a checksum per applied migration and refuses to start against a
database whose applied file no longer matches. Until the first release that
protection is being spent deliberately: schema changes are merged into the
initial migration rather than appended, because nothing is deployed and a
history of one file is easier to read than a history of corrections. After the
release, migrations are append-only.

### The schema

Five tables. Deliberately between one opaque blob and full normalisation:
**anything with its own identity and lifecycle is a row, and a leaf map
hanging off one is a JSONB column on that row.**

| Table | Holds |
|---|---|
| `configurations` | One row per named configuration: the revision, and the settings that are single values |
| `admin_tokens` | One row per token |
| `proxies` | One row per proxy; `headers` and `access` are JSONB on it |
| `mocks` | One row per mock; the selector maps and response headers are JSONB on it |
| `templates` | One row per template file, content included |

A blob would make the row opaque, and rewriting a whole document to change one
field throws away the per-proxy concurrency the admin API is built on. A table
per leaf map would buy joins nobody will write.

`ordinal` columns preserve document order. That is not decoration: mock
patterns are unanchored, so a general one placed before a specific one shadows
it, and a store that reordered them would change which mock answers a request.

`templates` has no cascading foreign key on purpose. Deleting a proxy's files
is a decision the store makes explicitly and *after* the configuration write --
the write is what authorises dropping them -- and a cascade would move that
ordering into the schema where the reasoning behind it is invisible.

### Templates

The database holds them; the directory named by `templates.dir` is a cache of
it. The render path reads a file at request time, and turning that into a
database round trip would put a query on the hot path of every mocked
response.

Every write touches both, row first. A failure to write the file then leaves
the row for the next reload to replay; the other order would report success
for a template the database never received.

### Concurrency

A save is one transaction. When the caller supplies the revision it built from,
the write becomes `UPDATE ... WHERE revision = ...`, and a revision that has
moved matches nothing and rolls the transaction back. The database serialises
the transaction, so the file store's advisory lock has no analogue here.

No explicit `SELECT ... FOR UPDATE` is needed for that, and adding one would
not make it safer. The conditional `UPDATE` takes the row lock itself, and a
second writer that blocks on it re-evaluates its own `WHERE` against the row
as the winner left it -- finds the revision moved, matches nothing, and is
told it lost.

A load is also one transaction, at `REPEATABLE READ`. A configuration is five
tables, and each query used to take its own connection and its own snapshot,
so a save committing in between produced a configuration assembled from two of
them. The revision check at the end of a load caught it and reported "the rows
and the revision column have diverged" -- which accuses the database of
corruption for what is ordinary concurrency, and sends an operator looking for
a hand-edited row that does not exist. Measured at 37 failures in 600 loads
against one concurrent writer; zero with one snapshot.

Template writes are single statements, so they need nothing further.
`materialize` reads across statements deliberately: a template added between
two of them appears on the next reload, which is the propagation model
described below rather than a gap in it.

## Two instances, one database

Both instances see the same configuration and agree on its revision, because a
revision is derived from content rather than counted.

What they do **not** get is automatic propagation. An instance keeps serving
the configuration it compiled until it is told to reload, by
`doppel config reload` or `POST /api/v1/config/reload`. A change one instance
writes reaches the other on that other's next reload -- including any template
the first uploaded, which is materialised onto disk as part of the same
sequence.

So: a rolling change is a write followed by a reload on each instance. There
is no coordination between them, and nothing stops two instances running
different revisions in between; `GET /status` on each reports which one it is
holding.

## Moving between the stores

```bash
# file -> database
doppel config push --config main.yaml --database-url "$DOPPEL_DATABASE_URL"

# database -> file
doppel config pull --database-url "$DOPPEL_DATABASE_URL" --output main.yaml
```

`pull` renders the same canonical form the revision is computed over, so a
pulled document pushed straight back produces the revision it left with.
Comments and layout are not preserved: they are no part of what the database
stores.

`push` is unconditional by default, which is what provisioning wants. Add
`--if-revision <hex>` to make it the same compare-and-swap the admin API uses,
so a scripted push cannot overwrite a change made in between.

An invalid document is refused with every violation listed, and nothing is
written -- validation happens inside the same transaction as the write.
