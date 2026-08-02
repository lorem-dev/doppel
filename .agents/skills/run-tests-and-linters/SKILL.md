---
name: run-tests-and-linters
description: Use before marking any task done, before opening a pull request, or whenever someone needs to know whether the workspace is currently green. Runs the full gate and captures its output.
---

# Run tests and linters

The gate is three commands, in this order, all of which must pass:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

## Capture the output, do not recall it

Run the gate so its output lands in a file, and quote the file:

```bash
{ cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test; } 2>&1 | tee /tmp/doppel-gate.txt
```

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
