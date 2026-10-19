#!/usr/bin/env python3
"""Fail if the front end asks for a Phosphor glyph the kit does not ship.

An unknown `ph-*` class renders as **nothing** — no error, no box, just an
invisible icon — which is how `ph-inbox`, `ph-label` and `ph-clock-arrow-ccw`
sat in this app for two shifts looking like empty space.

    python3 scripts/check-icons.py            # exit 1 on an unknown name
"""
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
CSS = ROOT / "vendor/rustex1/kit/web/vendor/phosphor/phosphor.css"

# Every way pages.rs / keys.rs spells an icon name.
PATTERNS = [
    r'"ph ph-([a-z0-9-]+)"',
    r'Node::icon\("([a-z0-9-]+)"\)',
    r'tool_btn\("([a-z0-9-]+)"',
]
# Names returned by a page method feeding `Node::icon_dyn`.
DYNAMIC = ["warning", "warning-circle"]


def main() -> int:
    if not CSS.exists():
        print(f"missing {CSS} — run `git submodule update --init`")
        return 2
    css = CSS.read_text()
    used: set[str] = set(DYNAMIC)
    for src in sorted((ROOT / "app/src").glob("*.rs")):
        text = src.read_text()
        for pat in PATTERNS:
            used.update(re.findall(pat, text))
    bad = sorted(n for n in used if f".ph.ph-{n}:before" not in css)
    print(f"{len(used)} icon names, {CSS.name}")
    for name in bad:
        print(f"  UNKNOWN ph-{name} — renders as nothing")
    if bad:
        return 1
    print("  all defined")
    return 0


if __name__ == "__main__":
    sys.exit(main())
