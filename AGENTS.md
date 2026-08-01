# AGENTS.md -- Doppel

This file is addressed to AI coding agents. Read it fully before touching code.

**Must read:** [CONTRIBUTING.md](./CONTRIBUTING.md)

---

## Project Overview

Doppel is a CLI-driven HTTP reverse proxy: a doppelganger that stands in for a
real backend so its clients can be developed and tested against a realistic,
deliberately degraded, or entirely absent upstream. It is a Cargo workspace of
independently owned crates under `crates/`; only four exist as of phase 1
(config/store/runtime, the proxy itself, telemetry, and the CLI binary), with
a templating crate and an admin-API crate to follow in phases 2 and 3.
Dependencies point one way, converging on `doppel-core`, so the config model
and validation are exercised without a network and the proxy logic is
exercised without an admin API.

| Crate | Owns |
|---|---|
| `doppel-core` | Configuration model, YAML loading, validation, the `ConfigStore` trait and its file-backed implementation, the compiled runtime, the error model. |
| `doppel-proxy` | The proxy listener, request resolution, fault injection, and upstream forwarding. |
| `doppel-telemetry` | Logging initialization. |
| `doppel-cli` | The `doppel` binary: argument parsing, the control channel, and wiring the other three crates together. |

```
doppel-cli -> { doppel-proxy, doppel-telemetry, doppel-core }
doppel-proxy -> doppel-core
doppel-telemetry -> doppel-core
doppel-core -> (nothing in this workspace)
```

See `README.md` for what phase 1 does and does not do yet, and
`.superpowers/specs/` (git-ignored, not shipped) for the full design.

---

## Running the Gates

Prerequisites: a stable Rust toolchain via rustup (pinned in
`rust-toolchain.toml`, with `rustfmt` and `clippy`)

```bash
# Rust (crates + Tauri backend)
cargo fmt --check   # formatting
cargo clippy        # lints
cargo test          # tests
```

All of these must pass before a pull request is ready.

### Verification workflow

Run the full gate (`cargo fmt --check`, `cargo clippy`, `cargo test`) as a single
dedicated step at the END of a change -- not after every task. While iterating,
write only the minimal tests needed to guarantee the code works, and run just the
focused test for what you changed (at most a quick `cargo test -p <crate>` or a
typecheck of the renderer). The comprehensive gate above is the single final
check before a change is considered done or a pull request opened.

---

## Conventions

### Text and Encoding

- All source code and documentation are ASCII-only. No Unicode punctuation
  (curly quotes, em dashes, ellipsis characters) outside of i18n catalogs and
  UI strings.

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

## CodeGraph

The project uses CodeGraph MCP tools (`codegraph_*`) for structural code
navigation. The index lives in `.codegraph/` which is git-ignored and not
shipped.

CodeGraph is an optional accelerator, not a project dependency. If the
`codegraph_*` tools are not installed or are otherwise unavailable in the
environment, do not require them and do not prompt the user to set them up:
silently fall back to the native search and read tools and continue. (This
differs from "installed but not initialized" - the no-index case below, where
offering to build the index is appropriate.)

When to use each tool:

- `codegraph_search` -- find a symbol by name (returns kind, location,
  signature).
- `codegraph_context` -- get focused context for a task or area (composes
  search + node + callers + callees in one call; use this first).
- `codegraph_callers` -- what calls a given function or method.
- `codegraph_callees` -- what a given function or method calls.
- `codegraph_impact` -- what would break if a symbol changed.
- `codegraph_node` -- a symbol's source, signature, or docstring.
- `codegraph_explore` -- deep survey of an unfamiliar module or pattern
  (token-heavy; use a subagent for large explorations).
- `codegraph_files` -- list files under a path.
- `codegraph_status` -- check index health.

Do not grep for symbol names when `codegraph_search` will answer faster and
more accurately.

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
| `run-tests-and-linters` | Before marking any task done -- run the full gate (lint, typecheck, test:cov at 90%). |
| `check-licenses` | After editing any `package.json` or `Cargo.toml` -- verify all npm and cargo dependencies are license-compliant and update LICENSE. |
| `pre-release-check` | Before cutting a release -- runs the five `check-*` and `run-*` skills above (not `bump-version`) plus version-bump and commit-format checks. |

<!-- CODEGRAPH_START -->
## CodeGraph

In repositories indexed by CodeGraph (a `.codegraph/` directory exists at the repo root), reach for it BEFORE grep/find or reading files when you need to understand or locate code:

- **MCP tool** (when available): `codegraph_explore` answers most code questions in one call — the relevant symbols' verbatim source plus the call paths between them, including dynamic-dispatch hops grep can't follow. Name a file or symbol in the query to read its current line-numbered source. If it's listed but deferred, load it by name via tool search.
- **Shell** (always works): `codegraph explore "<symbol names or question>"` prints the same output.

If there is no `.codegraph/` directory, skip CodeGraph entirely — indexing is the user's decision.
<!-- CODEGRAPH_END -->
