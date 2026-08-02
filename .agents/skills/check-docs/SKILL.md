---
name: check-docs
description: Use before a release, or after changing a command, a flag, a configuration field, a validation rule or an error code. Verifies docs/ and README.md still describe what the code does.
---

# Check the documentation

Documentation in this repository has three homes, and they go stale in
different ways:

- `README.md` -- the entry point. What Doppel is, what it does and does not do
  yet, how to run it.
- `docs/` -- the mkdocs site: configuration reference, behaviour, CLI, mocks.
- Doc comments in the code -- where the reasoning lives.

## What to check, and against what

Do not read the docs and ask whether they sound right. Compare them to the
thing they describe.

**Configuration fields.** Every field in `crates/doppel-core/src/config/` must
appear in the configuration reference, and every field in the reference must
exist. A field added without a doc entry is the common failure.

```bash
grep -rn 'pub [a-z_]*:' crates/doppel-core/src/config/
```

**Validation rules.** The rule table in the docs is numbered V1 upward. Check
the highest number in the table against the highest in
`crates/doppel-core/src/validate/`, and check the module doc comments, which
name a range and have drifted before.

**CLI surface.** Every subcommand and flag in `crates/doppel-cli/src/cli.rs`
must be in the CLI reference, with its environment variable and default.

**Error codes.** The closed set in `crates/doppel-core/src/error.rs`, with its
statuses, must match the table in the docs. A code added without a doc row is
invisible to whoever has to handle it.

**`main.example.yaml`.** It is the schema made concrete and is asserted against
by tests, so it cannot be stale -- but it can be *misleading*, which tests do
not catch. Read it as a newcomer copying it: does the ordering of its mocks
still make sense, does every mock it defines still reach requests, do its
comments still describe what the code does?

## Building the site

Python tooling here is driven by `uv`, never `pip` or `python -m venv`:

```bash
uv run --with-requirements docs/requirements.txt mkdocs build --strict
```

The pins in `docs/requirements.txt` are load bearing -- see the reasoning
there. If you loosen them, check the licence of what you pull in.

`--strict` turns a broken internal link into a failure, which is the point of
running it.

## Report

Say what you compared, not just that you checked. "Every config field has a
reference entry; V33 is the highest rule in both places; two module headers
still named V1..V30 and were corrected" is a useful report. "Docs look fine" is
not.
