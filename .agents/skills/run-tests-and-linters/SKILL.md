---
name: run-tests-and-linters
description: Use before marking any task done, before opening a pull request, or whenever someone needs to know whether the workspace is currently green. Runs the full gate and captures its output.
---

# Run tests and linters

The whole gate, in one command:

```bash
make gate
```

That is `fmt-check`, `lint`, `test`, `test-frontend`, `size`, `docs`, `schema`
and `licences`. It deliberately leaves out the browser suite -- `make e2e`
downloads Chromium on first run -- so run that too whenever the change touches
`frontend/` or the dashboard routes.

The Rust half by hand, in this order, all of which must pass:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

## The frontend half

The dashboard has its own gate, and one ordering that matters: the assets are
embedded into the binary at compile time, so they are built before anything that
reads them.

```bash
npm --prefix frontend ci
npm --prefix frontend run lint          # eslint
npm --prefix frontend run typecheck     # tsc, the application
(cd frontend && npx tsc --noEmit -p tsconfig.e2e.json)   # tsc, the specs
npm --prefix frontend run build         # writes frontend/dist
npm --prefix frontend test              # jest, including the bundle-size budgets
cargo build -p doppel-cli               # re-embeds what the build just wrote
npm --prefix frontend run e2e           # Playwright, against that binary
```

Two traps this order exists to avoid:

- `npm run build` alone changes nothing about a running binary. Rebuilding the
  frontend without rebuilding the binary is how half an hour goes into chasing a
  fixed bug that is still in the embedded bundle.
- The bundle-size suite measures `frontend/dist`. Running jest before the build
  measures the previous one, or fails because there is nothing to measure.

`DOPPEL_REQUIRE_DASHBOARD_ASSETS=1` on `cargo test` turns a skipped
asset-delivery suite into a failure, exactly as `DOPPEL_REQUIRE_DATABASE` does
for the store. `make test` sets it.

## Capture the output, do not recall it

Run the gate so its output lands in a file, and quote the file:

```bash
docker compose up -d --wait   # the database only; Doppel is behind a profile
export DOPPEL_TEST_DATABASE_URL=postgres://doppel:doppel@127.0.0.1:55432/doppel
export DOPPEL_REQUIRE_DATABASE=1

{ cargo fmt --check \
  && cargo clippy --all-targets --all-features -- -D warnings \
  && cargo test --workspace \
  && cargo test --workspace --all-features; } 2>&1 | tee /tmp/doppel-gate.txt
```

The database comes up first, and `DOPPEL_REQUIRE_DATABASE` turns a missing
URL into a test failure. Without it the PostgreSQL tests skip and pass, and
the skip notice they print is swallowed by `cargo test`'s output capture --
measured, not assumed: a full gate run contained zero occurrences of it. So
the variable, not the message, is what stops "skipped" from becoming "never
verified".

Both feature configurations are run because `sentry` is off by default; an
integration that only ever compiles one way is half-tested.

This is not ceremony. Over this project's history several reports stated test
counts, error codes or compiler diagnostics that did not survive being checked
against a transcript -- a column that pointed at the wrong token, a total that
was right while its parts were not, an experiment quoted with no artifact
behind it. Every number you report must be one you read out of a file.

Sum the per-suite `test result:` lines to get the workspace total rather than
guessing it, and say which command produced the figure.

## When something fails

- **`cargo fmt --check` fails.** Run `cargo fmt` and re-run the gate. Note in
  your report that formatting changed, since it means the code as written did
  not match the project's style.
- **Clippy fails.** Fix the lint. Do not add an `#[allow]` to silence it unless
  you can say in one sentence why the lint is wrong here, and put that sentence
  in the code as a comment. A blanket allow at module or crate level is almost
  never the right answer -- the one legitimate case in this repository is
  `tests/common/mod.rs`, which is compiled into several test binaries that each
  use a subset of it.
- **A test fails.** Read the failure before changing anything. If the test is
  wrong, say why it was wrong; if the code is wrong, fix the code. Do not
  weaken an assertion to make a suite green.

## Integration tests are slower and start processes

`crates/doppel-cli/tests/` spawns the built binary, binds ports and sends
signals. If you are iterating, `cargo test -p <crate>` on the crate you are
changing is enough; run the whole gate once before you finish.

A change to startup, shutdown or reload deserves five consecutive runs of the
integration suites rather than one, because that is where intermittency lives:

```bash
for i in 1 2 3 4 5; do cargo test -p doppel-cli --tests || break; done
```
