# Working on Doppel

How the workspace is laid out and why is in [Architecture](architecture.md).
This page is the mechanics.

## The gate

Three commands, all of which must pass before a change is done:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

While iterating, run the focused test for what you changed. Run the whole gate
once at the end, not after every edit.

Capture its output rather than recalling it:

```bash
{ cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test; } 2>&1 | tee /tmp/gate.txt
```

## Tests

Unit tests live beside the code. Integration tests in
`crates/doppel-cli/tests/` start the built binary, bind ports and send signals;
shared harness code is in `tests/common/`, split by topic into `proxying.rs`,
`reload.rs`, `cli.rs`, `shutdown.rs`, `logging.rs` and `mocks.rs`.

Two habits this codebase holds to:

- **A test must fail if the behaviour it names is removed.** When adding one,
  break the behaviour, watch it fail, then restore. Several tests here were
  found passing for the wrong reason, including one asserting a mock's response
  that a *different* mock also produced.
- **Fixtures should discriminate.** Ordering tests use adversarial names --
  `zeta` declared before `alpha` -- so a regression that sorts instead of
  preserving order actually fails.

## Tests that need a database

The PostgreSQL suites skip themselves when there is nothing to connect to, so
`cargo test` works on a machine with no database.

```bash
docker compose up -d --wait
export DOPPEL_TEST_DATABASE_URL=postgres://doppel:doppel@127.0.0.1:55432/doppel
cargo test
```

That starts the database and nothing else. `docker-compose.yml` also describes
Doppel itself, behind a `doppel` profile, so a bare `up` does not start it --
running the proxy during a test run would put a second process on the ports and
the templates directory the integration tests bind and write themselves.

To bring both up, for trying the two together rather than for testing:

```bash
cp main.example.yaml main.yaml   # once; main.yaml is git-ignored
make frontend                    # the dashboard the image embeds
docker compose --profile doppel up -d --wait
```

The copy matters twice over. It is the file the container mounts, and it is yours
to edit -- a port, an upstream, a token -- without any of that showing up as a
change to `main.example.yaml`, which the documentation quotes. It is also not
optional in a way that fails clearly: a bind mount whose source does not exist
makes Docker create a directory at that path, and Doppel then reports
`cannot read /etc/doppel/main.yaml: Is a directory`.

Doppel's own ports are offset -- `58080` and `58081` -- so a container and a
`cargo run` can be up at once.

Each test creates its own schema and drops it afterwards, so the suites can run
in parallel against one database.

| Variable | Effect |
|---|---|
| `DOPPEL_TEST_DATABASE_URL` | Where the database suites connect. Unset means they skip. |
| `DOPPEL_REQUIRE_DATABASE` | Turns a skip into a failure. Set it in CI, or a broken database looks like a green run. |

The second exists because a skip is invisible: `cargo test` captures the output
of passing tests, so a notice printed on the skip path never reaches anyone. A
run that was supposed to exercise the store and silently did not is worse than
one that failed.

## Building the documentation

Python tooling is driven by `uv`, never `pip` or `python -m venv`:

```bash
uv run --with-requirements docs/requirements.txt mkdocs serve   # preview on :8000
uv run --with-requirements docs/requirements.txt mkdocs build --strict
```

The toolchain is pinned in `docs/requirements.txt`, with the reasoning in that
file: MkDocs 2.0 removes the plugin system and ships unlicensed, and this
repository requires every dependency to carry an Apache-2.0-compatible
licence.

`--strict` turns a broken internal link into a build failure.

### How it is published

The site is versioned with [`mike`](https://github.com/jimporter/mike), which
keeps one built copy per release on the `gh-pages` branch alongside the
`versions.json` that fills the switcher in the header. GitHub Pages serves that
branch; nothing here uses the Pages deployment API.

`.github/workflows/docs.yml` decides what is published from the ref it ran on:

| Ref | Published as |
|---|---|
| a push to `main` | the `dev` alias |
| a final tag `v1.2.3` | `1.2.3`, and the `latest` alias moves to it |
| a pre-release tag `v1.2.3-rc.1` | nothing |

The site root redirects to `latest`, so the bare URL always lands on the newest
release rather than on unreleased documentation.

A pre-release publishes nothing on purpose. Release candidates exist to exercise
the release pipeline, their documentation is in-progress documentation that
`dev` already carries, and publishing it would only add entries to the switcher
that someone has to delete by hand afterwards.

Nothing needs doing by hand for a normal release. To rebuild one version -- after
fixing a typo in already-released documentation, say -- check that tag out and
run mike against it:

```bash
uv run --with-requirements docs/requirements.txt mike deploy --push 1.2.3
uv run --with-requirements docs/requirements.txt mike list
```

## Dependencies

Prefer the standard library and the existing set. Every direct dependency must
be justified, and the `check-licenses` skill runs after any `Cargo.toml` edit.
The permitted licences are MIT, BSD-2-Clause, BSD-3-Clause, ISC, Apache-2.0,
0BSD and CC0-1.0.

## Commits

Conventional Commits -- `feat`, `fix`, `chore`, `docs`, `test`, `refactor`,
`perf`, `ci`, `build` -- in English, imperative, subject under 72 characters,
scopes only where one already exists in the log. No mention of AI tools,
agents or assistants anywhere in a message, body or trailer.
