#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Check that every documentation link the dashboard builds goes somewhere.

    uv run scripts/check_docs_links.py

A wrong fragment is silent. A browser given `.../parameters/#proxies-timeuot`
opens the page at the top and nobody notices, so an (i) that goes nowhere survives
review, release and use. This is the check that fails instead.

Three things it holds:

1.  Every `info="..."` path in the frontend has a section in
    `docs/usage/parameters.md`, under the anchor `services/docs.ts` derives from it.
2.  Every documentation URL in the frontend is versioned. The site is published per
    version, so a link without one shows whoever follows it the rules of whatever
    has been released since.
3.  Every relative link and fragment inside `parameters.md` resolves -- the page is
    generated, and a generator that emitted a dead link would do it 80 times.

It reads the frontend as text on purpose: the alternative is running the bundle,
and what is being checked is a literal in the source either way. Every path it looks
for is a plain string literal, which is why `MockEditor` spells `mocks[].name` out
rather than building it.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
FRONTEND = ROOT / "frontend" / "src"
PARAMETERS = ROOT / "docs" / "usage" / "parameters.md"
DOCS = ROOT / "docs"

#: `info="mocks[].request.url"` on a field.
INFO = re.compile(r'info="([^"]+)"')
#: An anchor the generated page defines: `### `x` { #anchor }`.
ANCHOR = re.compile(r"^#{2,4} .*\{ #([A-Za-z0-9_-]+) \}\s*$", re.MULTILINE)
#: Any link to the published site.
SITE = re.compile(r"https://lorem-dev\.github\.io/doppel/([^'\"`\s)]*)")
#: A markdown link, for the third check.
LINK = re.compile(r"\]\(([^)]+)\)")

problems: list[str] = []


def anchor_for(path: str) -> str:
    """The same rule as `parameterAnchor` in `frontend/src/services/docs.ts`."""
    return f"proxies.{path}".replace("[]", "").replace(".", "-")


def sources() -> list[Path]:
    return sorted(p for p in FRONTEND.rglob("*.ts*") if "__tests__" not in p.parts)


def check_field_links(anchors: set[str]) -> None:
    for source in sources():
        for path in INFO.findall(source.read_text()):
            anchor = anchor_for(path)
            if anchor not in anchors:
                problems.append(
                    f"{source.relative_to(ROOT)}: info=\"{path}\" links to "
                    f"#{anchor}, which {PARAMETERS.relative_to(ROOT)} does not define"
                )


def check_versioned() -> None:
    """A documentation URL in the page has to carry a version.

    `services/docs.ts` builds them from the running version, so the only literals
    left should be the site root in that one file. Anything else is a link somebody
    wrote by hand, and it will age.
    """
    for source in sources():
        text = source.read_text()
        for tail in SITE.findall(text):
            if source.name == "docs.ts":
                continue
            problems.append(
                f"{source.relative_to(ROOT)}: documentation URL "
                f"'.../doppel/{tail}' is written out; build it with docsUrl() so it "
                "carries the running version"
            )


def check_generated_page(anchors: set[str]) -> None:
    text = PARAMETERS.read_text()
    for target in LINK.findall(text):
        if target.startswith(("http://", "https://", "mailto:")):
            continue
        page, _, fragment = target.partition("#")
        if page and not (DOCS / "usage" / page).is_file():
            problems.append(f"parameters.md links to {page}, which is not a page")
        if fragment and not page and fragment not in anchors:
            problems.append(f"parameters.md links to #{fragment}, which it does not define")


def main() -> None:
    if not PARAMETERS.is_file():
        print(
            "check-docs-links: docs/usage/parameters.md is missing; run "
            "`uv run scripts/parameters_doc.py`",
            file=sys.stderr,
        )
        raise SystemExit(1)

    anchors = set(ANCHOR.findall(PARAMETERS.read_text()))
    if len(anchors) < 20:
        print(
            f"check-docs-links: only {len(anchors)} anchors in parameters.md, which "
            "cannot be right -- the page or the anchor pattern changed",
            file=sys.stderr,
        )
        raise SystemExit(1)

    check_field_links(anchors)
    check_versioned()
    check_generated_page(anchors)

    if problems:
        for problem in problems:
            print(f"check-docs-links: {problem}", file=sys.stderr)
        raise SystemExit(1)
    print(f"check-docs-links: every dashboard link resolves ({len(anchors)} anchors)")


if __name__ == "__main__":
    main()
