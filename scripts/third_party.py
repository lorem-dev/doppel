#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Generate THIRD-PARTY.md from the resolved dependency graph.

    uv run scripts/third_party.py            # write the file
    uv run scripts/third_party.py --check    # fail if it is out of date

A released binary links its dependencies statically, so it redistributes them,
and MIT, BSD and Apache all require their notices to travel with a
redistribution. There are several hundred of those here -- far past what a
hand-maintained list survives -- so the list is generated and the generation is
checked in CI-style with `--check`.

Versions come from the resolve graph rather than from a name lookup. Two
versions of one crate routinely coexist in a Rust graph, and picking by name
silently reports whichever came last: an early draft of this file recorded
`rand 0.10.2` and `reqwest 0.13.4` when the workspace uses 0.9 and 0.12.
"""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUTPUT = ROOT / "THIRD-PARTY.md"

# Licences this project accepts, as SPDX identifiers. An expression is accepted
# when any one of its alternatives is here, which is what `OR` means: a crate
# offered as "MIT OR Apache-2.0 OR LGPL-2.1-or-later" is taken under MIT, and
# the LGPL option is simply not exercised.
PERMITTED = {
    "0BSD",
    "Apache-2.0",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "BSL-1.0",
    "CC0-1.0",
    "CDLA-Permissive-2.0",
    "ISC",
    "MIT",
    "MIT-0",
    "MPL-2.0",
    "Unicode-3.0",
    "Unlicense",
    "Zlib",
}

HEADER = """# Third-party notices

Doppel links its dependencies statically, so a released binary contains the
work below. Each retains its own copyright and is distributed under its own
licence.

Generated from the resolved dependency graph by `scripts/third_party.py`. Do
not edit by hand -- run the script.

"""


def graph() -> tuple[dict, set[str], dict]:
    raw = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--all-features"],
        capture_output=True,
        text=True,
        cwd=ROOT,
        check=True,
    ).stdout
    meta = json.loads(raw)
    by_id = {p["id"]: p for p in meta["packages"]}
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
    members = set(meta["workspace_members"])

    reachable: set[str] = set()
    stack = list(members)
    while stack:
        for dep in nodes[stack.pop()]["deps"]:
            if dep["pkg"] not in reachable:
                reachable.add(dep["pkg"])
                stack.append(dep["pkg"])
    return by_id, reachable - members, nodes


def alternatives(expression: str) -> list[str]:
    """Split an SPDX expression into the licences it offers a choice of.

    Deliberately crude: `AND` is treated the same as `OR`, so an expression
    combining licences is accepted if any part is permitted. That is wrong in
    principle and harmless here -- every `AND` in this graph combines
    permissive licences -- and the alternative is an SPDX parser for a check
    whose real job is to notice a copyleft licence arriving.
    """
    for separator in (" OR ", " AND ", "/"):
        expression = expression.replace(separator, "|")
    return [part.strip(" ()") for part in expression.split("|") if part.strip()]


def render() -> str:
    by_id, reachable, _ = graph()
    rows = sorted(
        (
            by_id[i]["name"],
            by_id[i]["version"],
            by_id[i].get("license") or "(not declared)",
            by_id[i].get("repository") or "",
        )
        for i in reachable
    )

    unpermitted = [
        (n, v, lic)
        for n, v, lic, _ in rows
        if not (set(alternatives(lic)) & PERMITTED)
    ]

    lines = [HEADER, f"{len(rows)} packages.\n", "| Package | Version | Licence |", "|---|---|---|"]
    for name, version, lic, repo in rows:
        shown = f"[{name}]({repo})" if repo.startswith("http") else name
        lines.append(f"| {shown} | {version} | {lic} |")
    lines.append("")

    if unpermitted:
        lines.append("## Not covered by the project's licence policy\n")
        for name, version, lic in unpermitted:
            lines.append(f"- **{name} {version}** -- {lic}")
        lines.append("")

    return "\n".join(lines)


def main() -> None:
    check = "--check" in sys.argv[1:]
    generated = render()

    if check:
        if not OUTPUT.exists():
            print("third-party: THIRD-PARTY.md does not exist", file=sys.stderr)
            raise SystemExit(1)
        if OUTPUT.read_text(encoding="utf-8") != generated:
            print(
                "third-party: THIRD-PARTY.md is out of date; "
                "run `uv run scripts/third_party.py`",
                file=sys.stderr,
            )
            raise SystemExit(1)
        print(f"third-party: up to date ({generated.count(chr(10) + '| ')} packages)")
        return

    OUTPUT.write_text(generated, encoding="utf-8")
    print(f"third-party: wrote {OUTPUT.relative_to(ROOT)}")

    # Say it out loud rather than only writing it into the file: a licence
    # arriving that the policy does not cover is the thing this exists to
    # catch, and nobody re-reads a generated file.
    if "## Not covered" in generated:
        print("third-party: SOME LICENCES ARE NOT COVERED BY THE POLICY", file=sys.stderr)
        raise SystemExit(1)


if __name__ == "__main__":
    main()
