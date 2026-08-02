---
name: bump-version
description: Use to start a release. Sets the version across every manifest, promotes the CHANGES.md Development section to a version heading, and makes the release commit. Does not tag and does not push.
---

# Bump the version

This skill prepares a release commit. Tagging and pushing are deliberately not
part of it -- see the end of this file.

## Where the version lives

One place, inherited by every crate:

```toml
# Cargo.toml
[workspace.package]
version = "0.1.0"
```

Each crate carries `version.workspace = true`, so a single edit moves them all.
Confirm that is still true rather than assuming it:

```bash
grep -rn '^version' Cargo.toml crates/*/Cargo.toml
```

`Cargo.lock` records the workspace crates' versions too, so run `cargo check`
after the edit to let cargo update it rather than editing it by hand.

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

## Promote the Development section

Rename `## Development` to `## <version> -- <date>` and open a fresh, empty
Development section above it. Do not edit the entries while promoting them: if
they are wrong, that is `check-changes`' job and should be a separate commit,
so the release commit stays reviewable as a mechanical change.

## The release commit

```bash
cargo check   # updates Cargo.lock
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
