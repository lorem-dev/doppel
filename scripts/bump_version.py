#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Set the workspace version and promote the CHANGES.md Development section.

    uv run scripts/bump_version.py 0.2.0

Does not run `cargo check`, does not commit, does not tag and does not push.
The `bump-version` skill drives those; keeping them out of here means the
script can be run and its diff read before anything is recorded.
"""

from __future__ import annotations

import datetime as dt
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# The version lives in exactly one place; every crate carries
# `version.workspace = true`. If that stops being true this script is the wrong
# tool, so it checks rather than assumes -- see `check_single_source`.
SEMVER = re.compile(
    r"^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)"
    r"(?:-((?:0|[1-9]\d*|\d*[a-zA-Z-][0-9a-zA-Z-]*)"
    r"(?:\.(?:0|[1-9]\d*|\d*[a-zA-Z-][0-9a-zA-Z-]*))*))?"
    r"(?:\+([0-9a-zA-Z-]+(?:\.[0-9a-zA-Z-]+)*))?$"
)


def fail(message: str) -> None:
    print(f"bump-version: {message}", file=sys.stderr)
    raise SystemExit(1)


def check_single_source() -> None:
    """Refuse to run if a crate pins its own version.

    A crate that stopped saying `version.workspace = true` would keep its old
    number through this script and be published under it -- silently, because
    nothing else compares the two.
    """
    offenders = []
    for manifest in sorted((ROOT / "crates").glob("*/Cargo.toml")):
        text = manifest.read_text(encoding="utf-8")
        if not re.search(r"^version\.workspace\s*=\s*true", text, re.MULTILINE):
            offenders.append(manifest.relative_to(ROOT))
    if offenders:
        listed = ", ".join(str(p) for p in offenders)
        fail(f"these crates do not inherit the workspace version: {listed}")


def set_workspace_version(version: str) -> str:
    path = ROOT / "Cargo.toml"
    text = path.read_text(encoding="utf-8")

    # Anchored to the `[workspace.package]` table so a `version = ` line in a
    # dependency specification cannot be hit instead.
    pattern = re.compile(
        r"(?P<lead>\[workspace\.package\][^\[]*?^version\s*=\s*)"
        r"\"(?P<version>[^\"]*)\"",
        re.MULTILINE | re.DOTALL,
    )
    found = pattern.search(text)
    if not found:
        fail("no version under [workspace.package] in Cargo.toml")

    previous = found.group("version")
    if previous == version:
        # Not a failure. The first release is exactly this case: the manifest
        # has carried the version since the workspace was created and only the
        # changelog needs promoting. Refusing here would send the one release
        # that matters most around the tooling and into a hand-edited
        # CHANGES.md.
        return previous

    path.write_text(pattern.sub(rf'\g<lead>"{version}"', text, count=1), encoding="utf-8")
    return previous


def promote_development(version: str) -> None:
    """Rename `## Development` to `## <version> -- <date>` and open a fresh one.

    The entries themselves are not touched. If one of them is wrong that is
    `check-changes`' job and belongs in its own commit, so the release commit
    stays reviewable as a mechanical change.
    """
    path = ROOT / "CHANGES.md"
    text = path.read_text(encoding="utf-8")

    if re.search(rf"^## {re.escape(version)}(\s|$)", text, re.MULTILINE):
        fail(f"CHANGES.md already has a section for {version}")

    # `[ \t]*` rather than `\s*`: `\s` matches a newline, so a `\s*$` here
    # swallowed the blank line after the heading and left the promoted
    # version welded to the `### Added` beneath it.
    heading = re.compile(r"^## Development[ \t]*$", re.MULTILINE)
    if not heading.search(text):
        fail("no `## Development` section in CHANGES.md")

    body_start = heading.search(text).end()
    next_heading = re.search(r"^## ", text[body_start:], re.MULTILINE)
    body_end = body_start + (next_heading.start() if next_heading else len(text) - body_start)
    if not text[body_start:body_end].strip():
        # Releasing with nothing written down produces empty release notes,
        # and `release_notes.py` refuses those -- so refuse here, where the
        # fix is still cheap.
        fail("the Development section is empty; there is nothing to release")

    today = dt.date.today().isoformat()
    replacement = f"## Development\n\n## {version} -- {today}"
    path.write_text(heading.sub(replacement, text, count=1), encoding="utf-8")


def main() -> None:
    if len(sys.argv) != 2:
        fail("usage: bump_version.py <version>")
    version = sys.argv[1].removeprefix("v")
    if not SEMVER.match(version):
        fail(f"not a semantic version: {version}")

    check_single_source()
    previous = set_workspace_version(version)
    promote_development(version)

    if previous == version:
        print(f"Cargo.toml: already {version}, left alone")
    else:
        print(f"Cargo.toml: {previous} -> {version}")
    print(f"CHANGES.md: Development promoted to {version}")
    print()
    print("Next, in this order:")
    print("  cargo check                    # updates Cargo.lock")
    print("  git add Cargo.toml Cargo.lock CHANGES.md")
    print(f'  git commit -m "chore: release {version}"')
    print()
    print("Tagging is deliberately not done here: CONTRIBUTING.md decides which")
    print("branch a tag may be cut from, and that is a judgement about where the")
    print("work sits rather than something to infer from a version string.")


if __name__ == "__main__":
    main()
