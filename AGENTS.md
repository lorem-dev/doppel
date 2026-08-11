# AGENTS.md -- Doppel

This file is addressed to AI coding agents. Read it fully before touching code.

**Must read:** [CONTRIBUTING.md](./CONTRIBUTING.md)

---

## Project Overview

Doppel is a CLI-driven HTTP reverse proxy: a doppelganger that stands in for a
real backend so its clients can be developed and tested against a realistic,
deliberately degraded, or entirely absent upstream.

The workspace layout, the crates and what each owns, the dependency direction,
and the design decisions that shape them -- parse-don't-validate, the compiled
runtime, how authorization is scoped, the two stores behind one trait -- are in
[docs/development/architecture.md](./docs/development/architecture.md). Read it
before changing anything structural; it is one file rather than two so the two
cannot drift.

`docs/` is the mkdocs site, in three sections: `overview/` for what Doppel is,
`usage/` for how to use it, `development/` for how to work on it.
`.superpowers/specs/` (git-ignored, not shipped) holds the full design.

`frontend/` is the browser dashboard the admin listener serves; see
[the frontend](#the-frontend) below before touching it.

---

## Running the Gates

Prerequisites: a stable Rust toolchain via rustup (pinned in
`rust-toolchain.toml`, with `rustfmt` and `clippy`), Node for the dashboard, and
`uv` for the Python tooling.

```bash
make gate     # everything below, in the right order
make help     # every target, with a description
```

The Rust half by hand:

```bash
cargo fmt --check                          # formatting
cargo clippy --all-targets -- -D warnings  # lints, warnings are errors
cargo test                                 # tests
```

All of these must pass before a pull request is ready. `make gate` leaves out
`make e2e`, which downloads a browser on first run -- run it too when the change
touches `frontend/` or the dashboard routes.

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

The rule, and where the reasoning lives:
[Architecture](./docs/development/architecture.md#configuration-parse-dont-validate).

What it means when you are adding a constraint:

- A bound on **one value** goes in a type under
  `crates/doppel-core/src/config/`, refusing the value while the document is
  being parsed.
- A check needing **two fields or the whole document** goes in the rule set
  under `crates/doppel-core/src/validate/`.
- Retiring a rule means updating `LIVE`, `RETIRED` and the retired-rules table
  in `docs/usage/configuration.md` together. Three tests will not let you do
  otherwise.

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
| `check-licenses` | After editing any `Cargo.toml` or `frontend/package.json` -- verify every direct dependency is licence-compliant and regenerate LICENSE and THIRD-PARTY.md. |
| `pre-release-check` | Before cutting a release -- runs the five `check-*` and `run-*` skills above (not `bump-version`) plus version-bump and commit-format checks. |

## The frontend

`frontend/` holds the dashboard: TypeScript, React, react-router, zustand,
Tailwind, built by vite. `docs/development/architecture.md` has the reasoning;
what follows is what you need before editing it.

```
frontend/src/
  components/   presentational; may not import a service (eslint enforces it)
  pages/        one screen each: fetches, holds its state, renders components
  services/     the only code that talks to the API
  store/        state that outlives one screen (zustand)
  types/        the API's shapes, and the one error type
frontend/e2e/
  src/          the harness the specs drive: the server, its configurations
  tests/        the Playwright specs
```

Four things that will otherwise cost you an hour each:

- **The assets are embedded at compile time.** `npm run build` alone changes
  nothing about a running binary; `cargo build` after it is what re-embeds them.
  A binary built with no `frontend/dist` compiles and answers 503 at `/`.
- **jest cannot render anything that routes.** react-router ships ESM only and
  uses `import.meta`, which does not survive the transform to CommonJS. Anything
  that navigates belongs in the Playwright suite, against a real browser and a
  real binary.
- **Rights are read through `useMay()`, and that is not decoration.** It
  subscribes to the report so the component re-renders when rights arrive. A hook
  that returned the same callback without subscribing would leave every gated
  control disabled for the life of the page -- which is what happened, and what
  the Playwright suite caught. `permits()` holds the precedence rules; there is
  deliberately no second way to ask.
- **TypeScript is pinned to 5.9 on purpose.** typescript-eslint declares
  `<6.1.0` and ts-jest declares `<7`, so the latest compiler is one no linter
  accepts yet. Raising it breaks the lint, not the build.

Commands: `make frontend`, `make test-frontend`, `make size`, `make e2e`.

`npm ci` prints deprecation warnings for `inflight`, `glob@7`, `glob@10` and
`whatwg-encoding`. All four are inside jest's own tree -- `@jest/transform` and
`@jest/reporters` pull the globs, jsdom pulls the encoder -- so nothing there
ships: the redistributed set is the ten packages in `THIRD-PARTY.md`, none of them
these. They are deliberately not silenced with `overrides`, which would pin
versions jest was not tested against to quieten a message about a package the
project does not distribute.

`frontend/package.json` carries no `version`: the page shows the *binary's*
version, taken from the configuration the listener injects, and a second version
in a second file is a second thing to forget at release time.

The favicon is generated from `assets/favicon.svg`; the command that rebuilds
`assets/favicon.ico` is in a comment at the top of that file. `assets/` is staged
into `docs/assets/` by the mkdocs build hook, and `docs/assets/` is git-ignored.

<!-- CODEGRAPH_START -->
## CodeGraph

In repositories indexed by CodeGraph (a `.codegraph/` directory exists at the repo root), reach for it BEFORE grep/find or reading files when you need to understand or locate code:

- **MCP tool** (when available): `codegraph_explore` answers most code questions in one call — the relevant symbols' verbatim source plus the call paths between them, including dynamic-dispatch hops grep can't follow. Name a file or symbol in the query to read its current line-numbered source. If it's listed but deferred, load it by name via tool search.
- **Shell** (always works): `codegraph explore "<symbol names or question>"` prints the same output.

If there is no `.codegraph/` directory, skip CodeGraph entirely — indexing is the user's decision.
<!-- CODEGRAPH_END -->
