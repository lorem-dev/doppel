#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Append a Downloads section to RELEASE_NOTES.md for the staged assets.

    uv run scripts/release_downloads.py v0.2.0 dist

    tag       release tag (falls back to $GITHUB_REF_NAME)
    dist-dir  directory of staged assets (default: dist)

    $GITHUB_REPOSITORY  owner/repo for the URLs (default: lorem-dev/doppel)

Two invariants this is built around:

1. **It never renames anything.** Asset file names are the download contract:
   `scripts/install.sh` builds `releases/<tag>/download/doppel-<target>.tar.gz`
   from a hardcoded pattern, without reading the release body or listing
   assets. This only reads the staged names and writes links to them, so the
   installer cannot be affected by anything here.

2. **It never silently drops an asset.** Anything it cannot classify still
   appears, under "Other", labelled with its raw file name. A release listing
   that quietly omitted a platform would be worse than an ugly label.
"""

from __future__ import annotations

import os
import sys
from pathlib import Path
from urllib.parse import quote

ROOT = Path(__file__).resolve().parent.parent

# Rust target triple -> how it is described, and the order it is listed in.
# Ordered by what a reader is most likely to be on.
TARGETS: dict[str, tuple[str, int]] = {
    "aarch64-apple-darwin": ("macOS, Apple Silicon", 1),
    "x86_64-unknown-linux-gnu": ("Linux, x86-64", 2),
    "aarch64-unknown-linux-gnu": ("Linux, arm64", 3),
}


def fail(message: str) -> None:
    print(f"release-downloads: {message}", file=sys.stderr)
    raise SystemExit(1)


def main() -> None:
    raw_tag = sys.argv[1] if len(sys.argv) > 1 else os.environ.get("GITHUB_REF_NAME")
    if not raw_tag:
        fail("no tag given (argument or $GITHUB_REF_NAME)")

    dist_arg = sys.argv[2] if len(sys.argv) > 2 else "dist"
    dist = Path(dist_arg)
    if not dist.is_absolute():
        dist = ROOT / dist
    if not dist.is_dir():
        fail(f"no such directory: {dist}")

    repo = os.environ.get("GITHUB_REPOSITORY", "lorem-dev/doppel")

    def url(name: str) -> str:
        return (
            f"https://github.com/{repo}/releases/download/"
            f"{quote(raw_tag)}/{quote(name)}"
        )

    binaries: list[tuple[int, str, str]] = []
    verification: list[tuple[int, str, str]] = []
    schema: list[tuple[int, str, str]] = []
    other: list[tuple[int, str, str]] = []

    for path in sorted(dist.iterdir()):
        name = path.name
        if name in {"checksums.txt", "checksums.txt.asc"}:
            verification.append((0, name, name))
            continue

        if name == "doppel-config.schema.json":
            schema.append((0, name, name))
            continue

        stem = name.removesuffix(".tar.gz")
        if stem != name and stem.startswith("doppel-"):
            triple = stem.removeprefix("doppel-")
            described, rank = TARGETS.get(triple, (triple, 99))
            binaries.append((rank, described, name))
            continue

        other.append((0, name, name))

    if not binaries and not schema and not other:
        fail(f"no assets found in {dist}")

    def bullets(entries: list[tuple[int, str, str]]) -> str:
        entries.sort(key=lambda e: (e[0], e[1]))
        return "\n".join(f"- [{label}]({url(name)})" for _, label, name in entries)

    sections: list[str] = []

    if binaries:
        sections.append(
            f"### Binaries\n\n{bullets(binaries)}\n\n"
            "Or install it in one line, no download needed:\n\n"
            "```bash\n"
            f"curl -fsSL https://raw.githubusercontent.com/{repo}/main/scripts/install.sh | sh\n"
            "```\n\n"
            "macOS marks anything downloaded from a browser as quarantined and "
            "refuses to run it unsigned. The installer is not affected, but a "
            "manual download is -- see "
            f"[Troubleshooting](https://lorem-dev.github.io/doppel/usage/troubleshooting/)."
        )

    if schema:
        sections.append(
            f"### Configuration schema\n\n{bullets(schema)}\n\n"
            "The JSON Schema for `main.yaml` as of this release. Point an editor "
            "at it to get completion, per-field descriptions and errors as you "
            "type:\n\n"
            "```yaml\n"
            f"# yaml-language-server: $schema=https://github.com/{repo}/releases/download/"
            f"{quote(raw_tag)}/doppel-config.schema.json\n"
            "```\n\n"
            "Or follow `main` instead of pinning a release:\n\n"
            "```yaml\n"
            f"# yaml-language-server: $schema=https://raw.githubusercontent.com/{repo}/main/doppel-config.schema.json\n"
            "```"
        )

    if other:
        sections.append(f"### Other\n\n{bullets(other)}")

    if verification:
        signed = any(name == "checksums.txt.asc" for _, _, name in verification)
        body = (
            f"### Verification\n\n{bullets(verification)}\n\n"
            "SHA-256 sums for every asset above:\n\n"
            "```bash\n"
            "shasum -a 256 -c checksums.txt --ignore-missing\n"
            "```"
        )
        if signed:
            # The sums are only worth anything if the file carrying them is
            # itself trustworthy, so the signature check comes with them
            # rather than being left for the reader to look up.
            body += (
                "\n\n`checksums.txt.asc` is a detached signature over "
                "`checksums.txt`, made with the "
                f"[Lorem Dev release key](https://github.com/{repo}/blob/main/.github/release-key.asc):\n\n"
                "```bash\n"
                f"curl -fsSL https://raw.githubusercontent.com/{repo}/main/.github/release-key.asc | gpg --import\n"
                "gpg --verify checksums.txt.asc checksums.txt\n"
                "```"
            )
        sections.append(body)

    notes_path = ROOT / "RELEASE_NOTES.md"
    existing = (
        notes_path.read_text(encoding="utf-8").rstrip() if notes_path.exists() else ""
    )
    downloads = "## Downloads\n\n" + "\n\n".join(sections)
    body = f"{existing}\n\n{downloads}\n" if existing else f"{downloads}\n"

    notes_path.write_text(body, encoding="utf-8")
    print(downloads)


if __name__ == "__main__":
    main()
