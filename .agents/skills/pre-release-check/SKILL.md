---
name: pre-release-check
description: Use before cutting a release. Runs the four check and run skills, then verifies the version bump and commit hygiene. Does not bump the version and does not tag.
---

# Pre-release check

A gate, not a fixer. Anything it finds gets fixed in its own commit before the
release proceeds; do not fold repairs into the release commit, which should
stay a mechanical, reviewable change.

## Run these four first

In this order, because a later one is pointless if an earlier one fails:

1. `run-tests-and-linters` -- the workspace is green, with captured output.
2. `check-licenses` -- every direct dependency is compliant and justified.
3. `check-changes` -- the Development section reflects what landed.
4. `check-docs` -- the reference, CLI surface, error codes and rule table match
   the code.

`bump-version` is deliberately not in this list. This skill checks; that one
changes things.

## Then check the version

```bash
grep -n '^version' Cargo.toml
git tag --list | tail -5
```

The workspace version must be ahead of the newest release tag, and the
CHANGES.md heading for it must exist with a date. If the version is unchanged
since the last tag, the release has not been prepared -- stop and say so.

## Then dry-run the release scripts

The release workflow composes its body from these two. Running them here means
a failure lands before the tag rather than after, when the only fix is a second
tag.

```bash
uv run scripts/release_notes.py <version>
uv run scripts/release_downloads.py <version> <a directory of fake assets>
```

`release_notes.py` fails when CHANGES.md has no section for the version, or has
an empty one. `release_downloads.py` fails on an empty asset directory and puts
anything it cannot classify under "Other" rather than dropping it -- so read its
output rather than only its exit code, and check every platform you expect is
listed.

Read the composed `RELEASE_NOTES.md` before continuing, and delete it: it is a
build artifact, not a file the repository keeps.

## Then check that a release can actually be installed

The asset names are a contract. `scripts/install.sh` builds
`doppel-<target>.tar.gz` from a hardcoded pattern, and
`.github/workflows/release.yml` stages the archives under exactly that name.
Neither reads the other.

```bash
grep -n 'doppel-\${target}\|asset=' scripts/install.sh
grep -n 'asset=' .github/workflows/release.yml
```

The three targets in the workflow matrix, the three in `install.sh`'s `case`,
and the three in `release_downloads.py`'s `TARGETS` must be the same three. A
target added to the build and not to the installer produces an archive nobody
can install with one line, and nothing else notices.

## Then check commit hygiene

Over the range since the last tag:

```bash
git log --format='%H %s' <last-tag>..HEAD
git log --format='%B%n%(trailers)' <last-tag>..HEAD | grep -icE 'claude|codex|copilot|co-authored|generated with|assistant'
```

Every subject must be Conventional Commits -- one of `feat`, `fix`, `chore`,
`docs`, `test`, `refactor`, `perf`, `ci`, `build` -- in English, imperative,
under 72 characters, with a scope only if that scope already exists in the log.
The grep must return zero: no mention of AI tools, agents or assistants
anywhere in any message, body or trailer.

## Report

List each check with its result and the evidence you read. A pass
with no evidence behind it is the failure mode this project has seen most
often: several reports over its history quoted counts and diagnostics that did
not survive being checked. Name the command and the file for each figure.

State plainly whether the release should proceed. "Ready except X" is a useful
answer; "looks good" is not.
