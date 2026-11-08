#!/usr/bin/env python3
"""Ratchet: the x86 peephole must not allocate a line before a filter rejects it.

MB-20 step 3 removed 25 sites of the form

    let text = infos[i].trimmed(store.get(i)).to_string();   // heap copy
    let Some(..) = parse_x(&text) else { i += 1; continue; }; // ...usually rejected

These are per-line loops inside a fixpoint pass, so the wasted allocation is
multiplied by line count and iteration count. The measured cost of the class was
185,110 allocations (9.571%) over 21 benchmark inputs.

Two compliant shapes exist, and the ratchet accepts both:

  * the parser yields no borrow of its input (`parse_reg_copy`, `parse_vec_copy`,
    ...): parse the borrowed text directly, no owned copy at all;
  * the parser's result borrows the text and outlives a `store` mutation
    (`parse_memory_load`, ...): probe on borrowed text first, then allocate and
    re-parse only for candidates.

What must never come back is allocating first and filtering second. This is a
source contract rather than a timing gate: it is deterministic, needs no
profiler, and pins the exact defect instead of a machine-dependent number.
"""
from __future__ import annotations

import argparse
from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parents[2]
PEEPHOLE = ROOT / "src/backend/x86/codegen/peephole"

# `let NAME = <expr>.trimmed(store.get(..)).to_string();`
# NOTE: `.*` and not `[^)]*` -- the argument is `store.get(i)`, which nests
# parentheses. A `[^)]*` here silently matched nothing and made this whole
# ratchet vacuous (it passed on the pre-fix tree); both directions are now
# pinned by tests/regression/test_check_peephole_alloc_before_filter.py.
ALLOC = re.compile(
    r"^\s*let\s+(\w+)\s*=\s*.*\.trimmed\(.*\)\.to_string\(\);\s*$"
)
# `let Some(..) = parser(&NAME) else {`  /  `if !parser(&NAME) {`
REJECT_LET = re.compile(r"^\s*let\s+Some\(.*\)\s*=\s*(\w+)\(&(\w+)\)\s+else\s*\{")
REJECT_IF = re.compile(r"^\s*if\s+!(\w+)\(&(\w+)\)\s*\{")


def violations(text: str) -> list[tuple[int, str, str]]:
    """Return (1-based line, variable, parser) for every allocate-then-reject site."""
    lines = text.split("\n")
    found = []
    for idx, line in enumerate(lines):
        m = ALLOC.match(line)
        if not m:
            continue
        var = m.group(1)
        # The filter is the next non-blank, non-comment statement.
        nxt = idx + 1
        while nxt < len(lines) and (
            not lines[nxt].strip() or lines[nxt].lstrip().startswith("//")
        ):
            nxt += 1
        if nxt >= len(lines):
            continue
        for pat in (REJECT_LET, REJECT_IF):
            g = pat.match(lines[nxt])
            if g and g.group(2) == var:
                found.append((idx + 1, var, g.group(1)))
                break
    return found


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--root", type=Path, default=PEEPHOLE,
                    help="directory to scan (default: the x86 peephole)")
    args = ap.parse_args()
    if not args.root.is_dir():
        print(f"FAIL: not a directory: {args.root}", file=sys.stderr)
        return 2
    files = sorted(args.root.rglob("*.rs"))
    if not files:
        print(f"FAIL: no Rust sources under {args.root}", file=sys.stderr)
        return 2
    total = 0
    for path in files:
        for line, var, parser in violations(path.read_text()):
            rel = path.relative_to(ROOT) if path.is_relative_to(ROOT) else path
            print(f"FAIL: {rel}:{line}: `{var}` is heap-copied before `{parser}` "
                  f"can reject it; probe on borrowed text first "
                  f"(or drop the copy if `{parser}` does not borrow its input)")
            total += 1
    if total:
        print(f"check_peephole_alloc_before_filter: {total} violation(s)")
        return 1
    print(f"check_peephole_alloc_before_filter: PASS ({len(files)} files, "
          f"no allocation before a rejecting filter)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
