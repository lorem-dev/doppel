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

Both ecosystems, because the binary redistributes both: the dashboard's
JavaScript is compiled into `frontend/dist` and then embedded into the
executable, which makes every npm package in that bundle as redistributed as any
crate. `CONTRIBUTING.md` has promised this since before it was true.

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

Doppel links its dependencies statically and embeds the dashboard's compiled
JavaScript, so a released binary contains the work below -- crates and npm
packages alike. Each retains its own copyright and is distributed under its own
licence.

Generated from the resolved dependency graph by `scripts/third_party.py`. Do
not edit by hand -- run the script.

"""


def shipped_npm_names() -> set[str]:
    """The npm packages whose code reaches the bundle.

    `npm ls --omit=dev`, walked: only these are redistributed, because only these
    are compiled into `frontend/dist` and embedded in the binary. Everything else
    -- vite, jest, eslint, tailwind's compiler and the tooling underneath them --
    runs on a developer's machine and ships nowhere, so listing it would claim a
    redistribution that does not happen and would drag the policy over licences
    that never travel.

    That distinction is not cosmetic: the tooling closure carries Blue Oak and
    CC-BY, and adding those to the permitted set to silence the check would have
    widened the policy for code the project does not distribute.
    """
    raw = subprocess.run(
        ["npm", "ls", "--omit=dev", "--all", "--json"],
        capture_output=True,
        text=True,
        cwd=ROOT / "frontend",
        # `npm ls` exits non-zero for an incomplete tree while still printing a
        # usable one, so the exit code is not the signal; an empty or unparseable
        # output is.
        check=False,
    ).stdout
    if not raw:
        raise SystemExit(
            "third-party: `npm ls --omit=dev` printed nothing, so the npm half of "
            "the notices would be silently empty; run `npm --prefix frontend ci`"
        )

    names: set[str] = set()

    def walk(node: dict) -> None:
        for name, child in (node.get("dependencies") or {}).items():
            if name not in names:
                names.add(name)
                if isinstance(child, dict):
                    walk(child)

    walk(json.loads(raw))
    if not names:
        # The manifest has runtime dependencies, so an empty walk means the tree
        # was not read rather than that nothing ships -- and writing the file from
        # it would drop the npm half without saying so.
        raise SystemExit(
            "third-party: no shipped npm packages were found; the tree in "
            "frontend/node_modules does not match frontend/package.json"
        )
    return names


def npm_packages() -> list[tuple[str, str, str, str]]:
    """Every shipped npm package, as (name, version, licence, repository).

    Licences are read from the installed tree: `npm ls --json` does not report
    them, and the registry would make generating this file need the network. A
    missing `node_modules` yields an empty list; `main` decides whether that is a
    failure.
    """
    root = ROOT / "frontend" / "node_modules"
    if not root.is_dir():
        # Refused rather than skipped, in both modes. Returning an empty list here
        # made `--check` report a stale file -- true, but the wrong sentence -- and
        # made a plain run *write* a cargo-only file that then looked correct. The
        # missing half has to be an error where the half goes missing.
        raise SystemExit(
            "third-party: frontend/node_modules is absent, so the npm half of the "
            "notices cannot be read; run `npm --prefix frontend ci`"
        )
    shipped = shipped_npm_names()

    found: list[tuple[str, str, str, str]] = []
    for manifest in sorted(root.glob("*/package.json")) + sorted(
        root.glob("@*/*/package.json")
    ):
        try:
            meta = json.loads(manifest.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError):
            continue
        name = meta.get("name")
        if not name or name not in shipped:
            continue
        licence = meta.get("license") or meta.get("licence")
        if isinstance(licence, dict):
            # The old form, still in a few long-lived packages.
            licence = licence.get("type")
        if isinstance(licence, list):
            licence = " OR ".join(
                entry.get("type", "") if isinstance(entry, dict) else str(entry)
                for entry in licence
            )
        repository = meta.get("repository")
        if isinstance(repository, dict):
            repository = repository.get("url", "")
        url = (repository or "").removeprefix("git+").removesuffix(".git")
        if url.startswith("git://"):
            url = "https://" + url.removeprefix("git://")
        found.append((name, meta.get("version", ""), licence or "(not declared)", url))
    return found


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
            "cargo",
            by_id[i]["name"],
            by_id[i]["version"],
            by_id[i].get("license") or "(not declared)",
            by_id[i].get("repository") or "",
        )
        for i in reachable
    )
    rows += sorted(
        ("npm", name, version, licence, url)
        for name, version, licence, url in npm_packages()
    )

    unpermitted = [
        (f"{ecosystem}:{n}", v, lic)
        for ecosystem, n, v, lic, _ in rows
        if not (set(alternatives(lic)) & PERMITTED)
    ]

    cargo = sum(1 for row in rows if row[0] == "cargo")
    npm = len(rows) - cargo
    lines = [
        HEADER,
        f"{len(rows)} packages: {cargo} from crates.io, {npm} from npm.\n",
        "| Package | Ecosystem | Version | Licence |",
        "|---|---|---|---|",
    ]
    for ecosystem, name, version, lic, repo in rows:
        shown = f"[{name}]({repo})" if repo.startswith("http") else name
        lines.append(f"| {shown} | {ecosystem} | {version} | {lic} |")
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
