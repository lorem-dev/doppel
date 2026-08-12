---
name: check-docs
description: Use before a release, or after changing a command, a flag, a configuration field, a validation rule or an error code. Verifies docs/ and README.md still describe what the code does, and that each page is in the right section.
---

# Check the documentation

Documentation in this repository has five homes, and they go stale in
different ways:

- `README.md` -- the entry point. What Doppel is, what it does and does not do
  yet, how to run it.
- `docs/` -- the mkdocs site, in three sections (below).
- `DOCKERHUB.md` -- the Docker Hub overview, pushed by the release workflow.
  Written for someone who has already chosen the image, so it opens with
  `docker run` and never mentions building from source. Check it against
  `docs/usage/docker.md`, which covers the same ground at length: a fact that
  changed in one and not the other is the failure mode here.
- `AGENTS.md` -- addressed to agents. Holds rules and pointers, never a second
  copy of anything in `docs/`.
- Doc comments in the code -- where the reasoning lives.

## The three sections

The site is split by the question a reader arrives with, and a page in the
wrong section is a defect even when every sentence in it is true.

| Section | Answers | Holds |
|---|---|---|
| `docs/overview/` | What is this? | What Doppel is, the vocabulary, how a request is handled. No configuration syntax. |
| `docs/usage/` | How do I do X? | Worked examples ordered from getting started to the awkward cases, then the configuration and CLI references. |
| `docs/development/` | How does it work inside, and how do I change it? | Architecture, the crates and their dependency direction, the gate, tests, commits, building the site. |

`docs/index.md` is the site home and belongs to Overview.

Check the placement, not only the content:

- **Anything naming a crate, a module path or an internal type belongs in
  `development/`.** A crate name in `usage/` is the common mistake -- a reader
  following an example does not have the source open.
- **Anything with a runnable example belongs in `usage/`.** An example in
  `overview/` means the overview has started teaching instead of orienting.
- **`usage/` is ordered.** Its `nav` runs first-run to advanced, then the two
  references. A new page has to be placed in that order, not appended.

```bash
grep -rn 'doppel-core\|doppel-proxy\|doppel-render\|doppel-admin\|crates/' docs/overview docs/usage
```

Anything that returns is either misplaced or has to justify itself.

## What to check, and against what

Do not read the docs and ask whether they sound right. Compare them to the
thing they describe.

**Configuration fields.** Every field in `crates/doppel-core/src/config/` must
appear in `docs/usage/configuration.md`, and every field in the reference must
exist.

```bash
grep -rn 'pub [a-z_]*:' crates/doppel-core/src/config/
```

**Validation rules.** `docs/usage/configuration.md` carries a retired-rules
table, and `crates/doppel-core/src/validate/mod.rs` carries `LIVE` and
`RETIRED`. Three tests already compare them to each other and to the source
markers, so run the suite rather than eyeballing it:

```bash
cargo test -p doppel-core --lib validate::tests
```

What the tests do not check is whether a rule's *description* still matches
what it does. Read the module headers.

**CLI surface.** Every subcommand and flag in `crates/doppel-cli/src/cli.rs`
must be in `docs/usage/cli.md`, with its environment variable and default.

**Environment variables.** Anything read through `std::env::var` must be
documented where it is used, not only in the CLI reference.

```bash
grep -rn 'env::var\|env = "DOPPEL' crates/
```

**Error codes.** The closed set in `crates/doppel-core/src/error.rs`, with its
statuses, must match the table in the docs. A code added without a doc row is
invisible to whoever has to handle it.

**The parameter reference.** `docs/usage/parameters.md` is generated from
`doppel-config.schema.json` by `scripts/parameters_doc.py`. Never edit it by hand
-- regenerate it and read the diff:

```bash
uv run scripts/config_schema.py && uv run scripts/parameters_doc.py
```

What the generator cannot judge is whether a field's *description* still says
something true, or whether its example is still the example worth showing. Those
come from doc comments and `examples(...)` in `crates/doppel-core/src/config/`, so
read them there when the behaviour changed.

**The links the dashboard builds.** The (i) beside a field links into that page,
and the link is assembled at runtime from the field's path and the running version
-- so mkdocs cannot see it and `--strict` cannot check it. A wrong fragment is
silent: the browser opens the page at the top and nobody notices.

```bash
uv run scripts/check_docs_links.py
```

It holds three things, and each has a failure it exists for:

- every `info="..."` path in `frontend/src` has a section under the anchor
  `frontend/src/services/docs.ts` derives from it -- a renamed field is the way
  this breaks;
- every documentation URL in the frontend is built through `docsUrl()`, so it
  carries the version. A hand-written link to the site root shows whoever follows
  it the rules of whatever has been released since;
- every relative link and fragment inside the generated page resolves.

Both checks are in `make gate` and in CI, so what is left for a human is the
half a script cannot see: follow two or three of the (i) links in a running
dashboard and read the sections they land on. An anchor that exists but explains
a different field passes every check here.

**`main.example.yaml`.** It is the schema made concrete and is asserted against
by tests, so it cannot be stale -- but it can be *misleading*, which tests do
not catch. Read it as a newcomer copying it: does the ordering of its mocks
still make sense, does every mock it defines still reach requests, do its
comments still describe what the code does?

**Examples in `usage/`.** They are not asserted by anything. Anything a reader
would paste has to be run at least once against a real process, not reasoned
about. A `curl` whose flags no longer match the endpoint is worse than no
example.

## Building the site

Python tooling here is driven by `uv`, never `pip` or `python -m venv`:

```bash
uv run --with-requirements docs/requirements.txt mkdocs build --strict
```

The pins in `docs/requirements.txt` are load bearing -- see the reasoning
there. If you loosen them, check the licence of what you pull in.

`--strict` turns a broken internal link into a failure, which is the point of
running it. It catches a moved page; it does not catch a page moved into the
wrong section, so run it *and* check placement.

## Report

Say what you compared, not just that you checked. "Every config field has a
reference entry; the rule-map tests pass; two `usage/` pages named crates and
were reworded; `mkdocs build --strict` is clean" is a useful report. "Docs look
fine" is not.

## The dashboard and the frontend

`docs/usage/dashboard.md` is the operator-facing page: the two configuration
fields, who may use it, what each screen does, the theme, and what a 503 at the
root means. `docs/development/architecture.md` carries the developer-facing half.

Check these together, because they go stale as a set:

- `admin.dashboard` and `admin.title` in `docs/usage/configuration.md` and in
  `main.example.yaml`.
- `GET /api/v1/access` and the routes in `docs/usage/admin-api.md`, plus
  `DASHBOARD_NOT_BUILT` in its error table. The endpoint table is the one thing
  there that is checked by a test -- `openapi.rs` refuses a document describing a
  route that is not served -- so a path that changed shows up in `cargo test`
  before it shows up here. What does not is prose quoting a path, and every
  healthcheck: the compose file, the Docker Hub overview, `observability.md`.
- The build order in `README.md`: assets first, then the binary. A reader who
  follows a stale version of that gets a 503 and no idea why.
- `make help`. Every target carries its own description, so the list cannot go
  stale on its own -- but a documented command that no longer matches a target is
  worth catching here.
- The four pages a reader reaches for before the dashboard's own: `docs/index.md`
  (the feature list and the reading order), `usage/getting-started.md` (a first run
  ends by opening it), `usage/troubleshooting.md` (the 503, disabled controls, the
  refresh timer) and `DOCKERHUB.md` (the image serves it too). A feature documented
  only on its own page is a feature nobody finds.

The icons live in `assets/` at the repository root and are staged into
`docs/assets/` by `scripts/mkdocs_assets.py` on every build. `docs/assets/` is
git-ignored: a link that points there is fine, a file committed there is not.
