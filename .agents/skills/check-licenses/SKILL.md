---
name: check-licenses
description: Use after editing any Cargo.toml or frontend/package.json, adding or removing a dependency, or before cutting a release. Verifies every direct dependency's licence against the project policy and updates LICENSE and THIRD-PARTY.md.
---

# Check licences

`CONTRIBUTING.md` makes this a merge gate, not a release-time chore: every
direct dependency must carry a licence compatible with Apache 2.0, and every
new one must be justified in the pull request.

## Permitted and forbidden

Permitted: MIT, BSD-2-Clause, BSD-3-Clause, ISC, Apache-2.0, 0BSD, CC0-1.0,
and dual licences offering any of these.

Forbidden: GPL-2.0, GPL-3.0, AGPL-3.0, LGPL-2.1, SSPL-1.0, BSL-1.1, any
Creative Commons `-NC-` variant, and anything carrying a Commons Clause
addendum. LGPL is on that list deliberately -- this project links statically,
so the dynamic-linking exception does not apply.

## How to check

Both halves are generated, so start by running the generator and reading what it
says:

```bash
uv run scripts/third_party.py           # rewrites THIRD-PARTY.md
uv run scripts/third_party.py --check   # what CI runs
```

It exits non-zero and prints `SOME LICENCES ARE NOT COVERED BY THE POLICY` with
the offending packages named. That is the check; the rest of this skill is for
deciding what to do about a new dependency.

### The cargo half

Read the licence from the registry rather than from memory:

```bash
for c in $(list of direct dependencies); do printf '%-24s' "$c"; cargo info "$c" 2>/dev/null | grep -i '^license:'; done
```

The direct set is what appears in `[workspace.dependencies]` and in each
crate's `[dependencies]` and `[dev-dependencies]` -- not the whole lock file.
Transitive crates come in under their parents' justification; if one of them
carries a forbidden licence that is a real problem, but it is found by auditing
the tree, not by this skill.

### The npm half

The direct set is `dependencies` and `devDependencies` in
`frontend/package.json`. The two are not equivalent, and the difference decides
whether a licence matters at all:

- `dependencies` is compiled into `frontend/dist` and embedded in the binary, so
  it is **redistributed**. Its licences bind, and it belongs in `LICENSE` and
  `THIRD-PARTY.md`.
- `devDependencies` -- vite, jest, eslint, tailwind's compiler, the tooling
  underneath them -- runs on a developer's machine and ships nowhere. It is
  deliberately absent from both files.

A build tool in `dependencies` is therefore a real mistake and not a matter of
taste: it drags the policy over licences the project never distributes.
Tailwind sat there once, and the tooling closure it pulled in carried Blue Oak
and CC-BY -- which would have meant widening the permitted set for code nobody
ships.

```bash
npm view <package> license                 # one package
npm ls --omit=dev --all --prefix frontend  # exactly what ships
```

## Ask the question the policy is actually for

For each dependency that is new since the last check, answer in one line: what
does it do that the standard library and the existing set cannot? This project
has removed dependencies on exactly that ground -- `fs4` went away once
`std::fs::File::lock` was found to be stable on the pinned toolchain, and
`libc` was never added because the signal a test needed could be sent by
shelling out. An unused or redundant direct dependency is a policy violation
even though it breaks nothing.

Also check for dependencies that have become unused. Four were found in one
sweep late in phase 1, each declared and referenced by no source file.

```bash
# a crate declared in a manifest but named in no source file is suspect
grep -rn "<crate_name>" crates --include='*.rs' | head
```

## Update LICENSE and THIRD-PARTY.md

`LICENSE` carries the Apache 2.0 text plus a table of the *direct* set, from both
ecosystems. `THIRD-PARTY.md` carries the whole closure and is generated -- never
edit it by hand.

```bash
uv run scripts/third_party.py
```

Both modes need `frontend/node_modules` present and refuse to run without it,
rather than writing a cargo-only file that looks complete or reporting a stale one
that is not. Report what you changed and
what you found -- including "nothing changed", which is a useful result.
