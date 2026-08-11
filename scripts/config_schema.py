#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Regenerate doppel-config.schema.json from the Rust configuration types.

    uv run scripts/config_schema.py            # write the file
    uv run scripts/config_schema.py --check    # fail if it is out of date

The schema is derived from the same `utoipa::ToSchema` implementations the admin
API's OpenAPI document uses, so there is no second description of the types to
keep in step. This script only moves the bytes: `doppel config schema` produces
them.

`--check` exists because the file is what editors fetch and what a release
attaches, and a stale one is worse than none -- it reports mistakes that are not
mistakes and accepts fields that no longer exist. CI runs it.
"""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUTPUT = ROOT / "doppel-config.schema.json"


def fail(message: str) -> None:
    print(f"config-schema: {message}", file=sys.stderr)
    raise SystemExit(1)


def generate() -> str:
    """The schema as `doppel config schema` prints it.

    Built in release-less debug mode on purpose: this runs in CI right after the
    test job has already compiled the workspace, so the artifacts are warm, and
    a `--release` build here would double the CI time to produce identical
    bytes.
    """
    build = subprocess.run(
        ["cargo", "build", "--quiet", "-p", "doppel-cli"],
        cwd=ROOT,
        capture_output=True,
        text=True,
    )
    if build.returncode != 0:
        fail(f"cannot build doppel-cli:\n{build.stderr}")

    result = subprocess.run(
        ["cargo", "run", "--quiet", "-p", "doppel-cli", "--", "config", "schema"],
        cwd=ROOT,
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        fail(f"`doppel config schema` failed:\n{result.stderr}")
    if not result.stdout.strip():
        fail("`doppel config schema` printed nothing")
    return result.stdout


def main() -> None:
    check = "--check" in sys.argv[1:]
    generated = generate()

    if check:
        if not OUTPUT.exists():
            fail(f"{OUTPUT.name} does not exist")
        if OUTPUT.read_text(encoding="utf-8") != generated:
            fail(
                f"{OUTPUT.name} is out of date; "
                "run `uv run scripts/config_schema.py`"
            )
        print(f"config-schema: {OUTPUT.name} is up to date")
        return

    OUTPUT.write_text(generated, encoding="utf-8")
    print(f"config-schema: wrote {OUTPUT.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
