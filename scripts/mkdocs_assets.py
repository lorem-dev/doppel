"""Stage the repository's icons into the documentation tree before a build.

The icons live in `assets/` at the repository root because two builds need
them: mkdocs, which only serves what is under `docs/`, and the frontend, whose
favicon is the same file. Copying them here rather than in a workflow step is
what keeps `mkdocs build --strict` behaving identically on a laptop and in CI --
a step that exists only in a workflow is a step that fails for whoever builds
the site by hand.

`docs/assets/` is git-ignored, so this copy is never committed and cannot drift
from `assets/`.
"""

from __future__ import annotations

import shutil
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SOURCE = ROOT / "assets"
DESTINATION = ROOT / "docs" / "assets"


def on_pre_build(config) -> None:  # noqa: ARG001 -- mkdocs passes its config
    DESTINATION.mkdir(parents=True, exist_ok=True)
    for source in sorted(SOURCE.iterdir()):
        if source.is_file():
            shutil.copy2(source, DESTINATION / source.name)
