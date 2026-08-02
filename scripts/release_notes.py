#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Extract one version's section from CHANGES.md into RELEASE_NOTES.md.

    uv run scripts/release_notes.py 0.2.0
    uv run scripts/release_notes.py            # falls back to $GITHUB_REF_NAME

A single leading `v` is stripped, so a tag name works as-is.

Standard library only, so this runs on a bare runner with nothing installed but
uv. The PEP 723 block above is what lets `uv run` execute it directly.
"""

from __future__ import annotations

import os
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def fail(message: str) -> None:
    print(f"release-notes: {message}", file=sys.stderr)
    raise SystemExit(1)


def section_for(changes: str, version: str) -> str:
    """The body under `## <version>`, up to the next `## ` heading.

    The heading is matched with a trailing boundary so `0.1.0` does not also
    match `0.1.01`, and a ` -- <date>` suffix is tolerated because that is how
    `bump_version.py` writes it.
    """
    lines = changes.splitlines()
    heading = re.compile(rf"^## {re.escape(version)}(\s|$)")

    start = next((i for i, line in enumerate(lines) if heading.match(line)), None)
    if start is None:
        fail(f'no "## {version}" section in CHANGES.md')

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

    changes_path = ROOT / "CHANGES.md"
    if not changes_path.exists():
        fail(f"no such file: {changes_path}")

    body = section_for(changes_path.read_text(encoding="utf-8"), version)
    if not body:
        # An empty section means the release was prepared but nothing was
        # written down. Publishing empty notes is worse than stopping: nobody
        # goes back and fills them in afterwards.
        fail(f"the section for {version} is empty")

    (ROOT / "RELEASE_NOTES.md").write_text(body + "\n", encoding="utf-8")
    print(body)


if __name__ == "__main__":
    main()
