#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Check that a tag matches the tree it points at.

    uv run scripts/check_release_tag.py v1.1.0
    uv run scripts/check_release_tag.py            # falls back to $GITHUB_REF_NAME

A single leading `v` is stripped, so a tag name works as-is.

This exists because a tag is a claim about a commit, and nothing else in the
build verifies the claim before acting on it. A tag cut from a branch that
does not carry the release commit -- the release pull request still open, the
branch not pulled -- builds and publishes perfectly well: the version in the
binary, the image tags and the Docker Hub overview all say what the tag said,
and the contents are whatever that commit held. `v1.1.0` was once pushed at a
1.0.0 tree and the image was published under three tags before the notes step
noticed.

So the two things a release cannot be wrong about are checked first, on a bare
runner, in seconds:

1. The workspace version equals the tag. This is what ends up in
   `doppel --version`, in `doppel_build_info` and in the image labels.
2. CHANGES.md has a non-empty section for it. This is the release body, and
   the one part of a release that cannot be regenerated later from the tree.

Standard library only, so this runs with nothing installed but uv. The PEP 723
block above is what lets `uv run` execute it directly.
"""

from __future__ import annotations

import os
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def fail(message: str) -> None:
    print(f"check-release-tag: {message}", file=sys.stderr)
    raise SystemExit(1)


def workspace_version(text: str) -> str:
    """The version under `[workspace.package]`.

    Anchored to that table, the way `bump_version.py` anchors its rewrite, so
    a `version = ` line in a dependency entry cannot be read instead.
    """
    found = re.search(
        r"\[workspace\.package\][^\[]*?^version\s*=\s*\"(?P<version>[^\"]*)\"",
        text,
        re.MULTILINE | re.DOTALL,
    )
    if not found:
        fail("no version under [workspace.package] in Cargo.toml")
    return found.group("version")


def changes_section(changes: str, version: str) -> str:
    """The body under `## <version>`, up to the next `## ` heading.

    Matched the way `release_notes.py` matches it -- trailing boundary so
    `0.1.0` does not also match `0.1.01`, ` -- <date>` suffix tolerated -- so
    that a tag passing this check cannot fail there.
    """
    lines = changes.splitlines()
    heading = re.compile(rf"^## {re.escape(version)}(\s|$)")

    start = next((i for i, line in enumerate(lines) if heading.match(line)), None)
    if start is None:
        return ""

    end = len(lines)
    for i in range(start + 1, len(lines)):
        if lines[i].startswith("## "):
            end = i
            break

    return "\n".join(lines[start + 1 : end]).strip()


def main() -> None:
    raw_tag = sys.argv[1] if len(sys.argv) > 1 else os.environ.get("GITHUB_REF_NAME")
    if not raw_tag:
        fail("no tag given (argument or $GITHUB_REF_NAME)")

    version = raw_tag.removeprefix("v")

    cargo_path = ROOT / "Cargo.toml"
    changes_path = ROOT / "CHANGES.md"
    for path in (cargo_path, changes_path):
        if not path.exists():
            fail(f"no such file: {path}")

    found = workspace_version(cargo_path.read_text(encoding="utf-8"))
    if found != version:
        fail(
            f"tag {raw_tag} names version {version}, but this commit is "
            f"{found} -- is the release commit merged, and is this tag on it?"
        )

    if not changes_section(changes_path.read_text(encoding="utf-8"), version):
        fail(
            f'no "## {version}" section in CHANGES.md, or it is empty -- '
            f"the release body comes from it"
        )

    print(f"check-release-tag: {raw_tag} matches version {found} and CHANGES.md")


if __name__ == "__main__":
    main()
