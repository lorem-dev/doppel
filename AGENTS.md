# AGENTS.md -- Doppel

This file is addressed to AI coding agents. Read it fully before touching code.

**Must read:** [CONTRIBUTING.md](./CONTRIBUTING.md)

---

## Project Overview

Doppel is a CLI-driven HTTP reverse proxy: a doppelganger that stands in for a
real backend so its clients can be developed and tested against a realistic,
deliberately degraded, or entirely absent upstream. It is a Cargo workspace of
independently owned crates under `crates/`. Dependencies point one way,
converging on `doppel-core`, so the config model and validation are exercised
without a network and the proxy logic is exercised without an admin API.

| Crate | Owns |
|---|---|
| `doppel-core` | Configuration model, YAML loading, validation, the `ConfigStore` trait and its file-backed implementation, the compiled runtime, the error model. |
| `doppel-proxy` | The proxy listener, request resolution, fault injection, mock matching, and upstream forwarding. |
| `doppel-render` | Jinja2 rendering for mock responses. |
| `doppel-admin` | The admin HTTP API: token access control, proxy CRUD, template files, reload, status, metrics, the OpenAPI document. |
| `doppel-store-postgres` | `PostgresStore`, and the sqlx migrations that own its schema. |
| `doppel-telemetry` | Logging initialization and optional Sentry. |
| `doppel-cli` | The `doppel` binary: argument parsing, the control channel, and wiring the other crates together. |

```
doppel-cli -> { doppel-proxy, doppel-admin, doppel-store-postgres,
                doppel-telemetry, doppel-core }
doppel-proxy -> { doppel-render, doppel-core }
doppel-admin -> doppel-core
doppel-store-postgres -> doppel-core
doppel-render -> doppel-core
doppel-telemetry -> doppel-core
doppel-core -> (nothing in this workspace)
```

See `README.md` for what the project does and does not do yet, and
`.superpowers/specs/` (git-ignored, not shipped) for the full design.

---

## Running the Gates

Prerequisites: a stable Rust toolchain via rustup (pinned in
`rust-toolchain.toml`, with `rustfmt` and `clippy`)

```bash
cargo fmt --check                          # formatting
cargo clippy --all-targets -- -D warnings  # lints, warnings are errors
cargo test                                 # tests
```

All of these must pass before a pull request is ready.

### Verification workflow

Run the full gate (`cargo fmt --check`, `cargo clippy`, `cargo test`) as a single
dedicated step at the END of a change -- not after every task. While iterating,
write only the minimal tests needed to guarantee the code works, and run just the
focused test for what you changed (at most a quick `cargo test -p <crate>`).
The comprehensive gate above is the single final check before a change is
considered done or a pull request opened.

Capture the gate's output when you report it. Over this project's history,
several reports stated test counts or quoted compiler diagnostics that did not
survive being checked against a transcript. Run the gate with
`2>&1 | tee <file>` and quote the file, rather than recalling what it said.

---

## Conventions

### Text and Encoding

- All source code and documentation are ASCII-only. No Unicode punctuation
  (curly quotes, em dashes, ellipsis characters) outside of i18n catalogs and
  UI strings.

### Configuration: parse, don't validate

A constraint on a single configuration value belongs in a type, not in a
validation rule. `crates/doppel-core/src/config/` holds one module per kind of
value -- `Name`, `Token`, `Port`, `HttpStatus`, `HttpMethod`, `Ratio`,
`Seconds`, `TimeoutSeconds`, `ByteSize`, `UpstreamUrl`, `HeaderName`,
`HeaderValue`, `Pattern`, `Selector`, `TemplateName` -- each refusing bad
input while the document is being parsed.

The rule set (`crates/doppel-core/src/validate/`) is for what a type cannot
decide: anything needing two fields at once, or the whole document. Duplicate
names, `min <= max`, "this field is required when that one says so", "the two
listeners must not share a port".

When adding a constraint, ask which it is. Getting it wrong the other way is
what the retired-rules table in `docs/configuration.md` records: nineteen
rules that each checked one value, several of them alongside a second copy of
the same check elsewhere in the codebase that could drift out of step.

Retiring a rule means updating `LIVE`, `RETIRED` and that table together --
there are tests that will not let you do otherwise.

A type that carries something expensive to derive should carry it derived:
`Pattern` holds the compiled regex, `UpstreamUrl` the parsed URL, `Selector`
its segments. That is what removes the second parse rather than merely moving
the first one.

### Commit Rules

Follow CONTRIBUTING.md exactly:

- Conventional Commits types: `feat`, `fix`, `chore`, `docs`, `test`,
  `refactor`, `perf`, `ci`, `build`.
- English, imperative mood, subject under 72 characters.
- No AI-tool mentions anywhere in commit messages or trailers.
- Scopes only when already established in `git log`.

### Branching

`feature/*` -> `develop` -> `main` via Merge Request. Direct commits to `main`
are allowed only until the first release.

Tagging follows the same split:

- **Release-candidate tags** (`v<version>-rc.<n>`) are cut from **`develop`**. An
  RC exists to exercise the release pipeline and the installers before the work
  reaches the release branch, so requiring a merge to `main` first would defeat
  its purpose and fill `main` with candidates.
- **Final release tags** (`v<version>`, no pre-release suffix) are cut from
  **`main`**, after `develop` has merged there.

Only RC tags may come from `develop`. The release workflow enforces this, so a
final tag pushed from `develop` fails before anything is built or published.

---

## Superpowers

Design specs and implementation plans live in `.superpowers/` which is
git-ignored. When planning a multi-step task, write a plan there first. The
`superpowers:writing-plans` skill guides the process. CHANGES.md entries are
planned at the plan stage, not after the fact.

---

## Local Development Skills

Skills live under `.agents/skills/`. Invoke them when the situation calls
for it:

| Skill | When to use |
|---|---|
| `bump-version` | To start a release -- set the version across every manifest and promote the CHANGES.md Development section, then make the release commit. Does not tag or push. |
| `check-changes` | After a batch of commits -- verify CHANGES.md (Development section) reflects every change. |
| `check-docs` | Before a release or after updating commands/options -- verify docs/ and README.md are current. |
| `run-tests-and-linters` | Before marking any task done -- run `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test`, and capture the output. |
| `check-licenses` | After editing any `Cargo.toml` -- verify every direct dependency is licence-compliant and update LICENSE. |
| `pre-release-check` | Before cutting a release -- runs the five `check-*` and `run-*` skills above (not `bump-version`) plus version-bump and commit-format checks. |

<!-- CODEGRAPH_START -->
## CodeGraph

In repositories indexed by CodeGraph (a `.codegraph/` directory exists at the repo root), reach for it BEFORE grep/find or reading files when you need to understand or locate code:

- **MCP tool** (when available): `codegraph_explore` answers most code questions in one call — the relevant symbols' verbatim source plus the call paths between them, including dynamic-dispatch hops grep can't follow. Name a file or symbol in the query to read its current line-numbered source. If it's listed but deferred, load it by name via tool search.
- **Shell** (always works): `codegraph explore "<symbol names or question>"` prints the same output.

If there is no `.codegraph/` directory, skip CodeGraph entirely — indexing is the user's decision.
<!-- CODEGRAPH_END -->
