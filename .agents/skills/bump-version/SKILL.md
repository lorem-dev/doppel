---
name: bump-version
description: Use to start a release. Sets the version across every manifest, promotes the CHANGES.md Development section to a version heading, and makes the release commit. Does not tag and does not push.
---

# Bump the version

This skill prepares a release commit. Tagging and pushing are deliberately not
part of it -- see the end of this file.

## Run the script

```bash
uv run scripts/bump_version.py <version>
```

There is exactly one place a version lives: `[workspace.package]` in
`Cargo.toml`. `frontend/package.json` deliberately has no `version` field -- the
dashboard reports the binary's, read from the configuration the listener injects --
so nothing there needs bumping, and adding a version to it would create a second
thing to forget.

It sets the version under `[workspace.package]` in `Cargo.toml`, renames
`## Development` in CHANGES.md to `## <version> -- <date>`, and opens a fresh
empty Development section above it. It prints what it changed and what to do
next.

It refuses rather than guessing, in four cases worth knowing:

- a crate that no longer says `version.workspace = true`, because it would
  keep its old number through the bump and be published under it;
- a version that is not semver;
- a version CHANGES.md already has a section for;
- an empty Development section -- releasing with nothing written down produces
  empty release notes, and `release_notes.py` refuses those, so it is refused
  here where the fix is still cheap.

Read the diff before committing. The script is deliberately the only thing
that edits; it does not run `cargo check`, commit, tag or push.

## Choosing the number

Semantic versioning, read from the CHANGES.md Development section rather than
from a feeling:

- a `Changed` or `Removed` entry describing something an operator's existing
  configuration or script would notice is a major bump before 1.0 means a minor
  bump, and after 1.0 a major one;
- new behaviour with nothing broken is a minor bump;
- fixes only is a patch.

Before 1.0 this project treats a breaking change as a minor bump, which is the
common convention and the one `0.x` semantics allow.

## Do not edit the entries while promoting them

If an entry is wrong, that is `check-changes`' job and belongs in its own
commit, so the release commit stays reviewable as a mechanical change.

## The release commit

```bash
cargo check   # updates Cargo.lock; the script does not
git add Cargo.toml Cargo.lock CHANGES.md
git commit -m "chore: release <version>"
```

Conventional Commits, no scope, and no mention of AI tools, agents or
assistants anywhere in the message, body or trailer.

## What this skill will not do

It does not tag and it does not push. Both are intentional: the branching rules
in `CONTRIBUTING.md` decide where a tag may be cut from -- release candidates
from `develop`, final tags from `main` only -- and that is a judgement about
where the work currently sits, not something to infer from a version string.
Print the tag command for the human to run rather than running it.

Pushing the tag is what starts `.github/workflows/release.yml`, which builds
the three targets, composes the release body from the CHANGES.md section this
skill just wrote, and publishes. So a mistake here becomes a published release;
that is the reason for the split.

Print the check that goes with the tag command, to be run **on the branch being
tagged, after pulling it**:

```bash
uv run scripts/check_release_tag.py v<version>
```

It says whether that checkout carries the version and the CHANGES.md section
the tag claims. A final tag is cut from `main` after the release pull request
is merged, and a tag on a `main` that has not been pulled builds and publishes
the previous release under the new number -- `v1.1.0` was once pushed at a
1.0.0 tree, and three image tags were published before anything noticed. The
`verify` job in the release workflow now refuses that, but it refuses it after
the tag exists; this catches it before.
