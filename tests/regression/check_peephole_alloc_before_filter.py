#!/usr/bin/env python3
"""Ratchet: no backend peephole may heap-copy a line before a filter rejects it.

MB-20 step 3 removed sites of the form

    let text = infos[i].trimmed(store.get(i)).to_string();   // heap copy
    let Some(..) = parse_x(&text) else { i += 1; continue; }; // ...usually rejected

These are per-line loops inside a fixpoint pass, so the wasted allocation is
multiplied by line count and iteration count.

WHY THIS IS STATEMENT-AWARE AND NOT A PAIR OF REGEXES
----------------------------------------------------
The first version recognised exactly two filter shapes -- `let Some(..) =
freefn(&var) else {` and `if !freefn(&var) {`. That left 23 of the 66 real sites
invisible, including `fold_copy_into_lea_base`, which ablation measured at
**30.4% of the whole peephole** and which copied every non-barrier line before
testing `strip_prefix("leaq ")`. The shapes it could not see:

    let (rest, w) = if let Some(r) = text.strip_prefix("leaq ") { .. }
                    else { continue };          # if-let / else chain
    if has_implicit_reg_usage(&text) { continue; }   # POSITIVE guard
    let Some(..) = text.strip_prefix(..) else { .. } # method, not free function
    if !text.starts_with(..) { continue; }           # method on the variable

So the contract is now expressed the way it actually reads: **the first
statement after the copy must not be able to reject the line based on that
copy's contents.** The scanner walks that one statement with a bracket-depth
counter (so an if-let/else chain is one statement, not five lines) and flags it
when it both mentions the variable and contains a `continue`/`break`/`return`.

Two compliant shapes remain, and both are accepted:

  * the filter yields no borrow of its input: parse the borrowed text directly
    and keep no owned copy at all (56 of 66 sites turned out to be this case --
    `LineStore` has no interior mutability and its API is `get(&self)` /
    `replace(&mut self)`, so a copy that the borrow checker lets you delete
    provably never observes a mutation, and the bytes are identical);
  * the filter's result borrows the text and outlives a `store` mutation: probe
    on borrowed text first, then allocate and re-parse only for candidates.

This is a source contract rather than a timing gate: deterministic, no profiler,
and it pins the defect instead of a machine-dependent number.
"""
from __future__ import annotations

import argparse
from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parents[2]
# Every backend, not just x86-64: the defect class is not architecture-specific,
# and an x86-only default made the gate vacuous for i686.
PEEPHOLE = ROOT / "src/backend"

# `let NAME = <expr>.trimmed(...).to_string();`
# NOTE: `.*` and not `[^)]*` -- the argument is `store.get(i)`, which nests
# parentheses. A `[^)]*` here silently matched nothing and made this whole
# ratchet vacuous. Both idioms must match:
#   x86-64: `infos[i].trimmed(store.get(i))`   (method)
#   i686:   `trimmed(store, &infos[i], i)`     (free function)
# `\b` before `trimmed` matches after `.` as well as at the start of a call.
ALLOC = re.compile(
    r"^\s*let\s+(?:mut\s+)?(\w+)\s*=\s*.*\btrimmed\(.*\)\.to_string\(\);\s*$"
)
# A closure form used by the window matchers: `let text = |k| ....to_string();`
# Every call of it copies, so it is the same defect with the copy hidden behind
# a name. Flagged only when a rejecting statement calls it.
ALLOC_CLOSURE = re.compile(
    r"^\s*let\s+(\w+)\s*=\s*\|[^|]*\|\s*.*\btrimmed\(.*\)\.to_string\(\);\s*$"
)
REJECT = re.compile(r"\b(continue|break|return)\b")
# How far one statement may run. The longest real shape is the 8-line
# `let (w, rest) = if let ... else if let ... else { continue };` chain.
MAX_STATEMENT_LINES = 24


def _statement(lines: list[str], start: int) -> str:
    """The single statement beginning at `start`, joined into one string.

    Stops at the first `;` or `}` that closes back to bracket depth zero, so an
    if-let/else chain is consumed whole. Comments and blank lines are skipped
    without ending the statement.
    """
    depth = 0
    out: list[str] = []
    i = start
    while i < len(lines) and i - start < MAX_STATEMENT_LINES:
        raw = lines[i]
        stripped = raw.strip()
        i += 1
        if not stripped or stripped.startswith("//"):
            continue
        # Strip a trailing line comment so a `// continue` cannot trigger.
        code = raw.split("//", 1)[0]
        out.append(code)
        depth += code.count("(") + code.count("[") + code.count("{")
        depth -= code.count(")") + code.count("]") + code.count("}")
        tail = code.rstrip()
        if depth <= 0 and (tail.endswith(";") or tail.endswith("}")):
            break
    return "\n".join(out)


def _uses(stmt: str, var: str) -> bool:
    """Does the statement read `var` (as a value, a borrow, or a call)?"""
    return re.search(r"\b" + re.escape(var) + r"\b", stmt) is not None


def violations(text: str) -> list[tuple[int, str, str]]:
    """Return (1-based line, variable, reason) for every allocate-then-reject site."""
    lines = text.split("\n")
    found = []
    for idx, line in enumerate(lines):
        m = ALLOC.match(line) or ALLOC_CLOSURE.match(line)
        if not m:
            continue
        var = m.group(1)
        # The filter must be the FIRST statement after the copy. A copy that is
        # stored, returned or otherwise unconditionally consumed is legitimate.
        nxt = idx + 1
        while nxt < len(lines) and (
            not lines[nxt].strip() or lines[nxt].lstrip().startswith("//")
        ):
            nxt += 1
        if nxt >= len(lines):
            continue
        stmt = _statement(lines, nxt)
        if not _uses(stmt, var):
            continue
        if not REJECT.search(stmt):
            continue
        # Name the filter for the message: the first call or method in it.
        call = re.search(r"(\w+)\s*\(\s*&?" + re.escape(var) + r"\b", stmt)
        meth = re.search(re.escape(var) + r"\.(\w+)\s*\(", stmt)
        if call:
            why = call.group(1)
        elif meth:
            why = f"{var}.{meth.group(1)}"
        else:
            why = "a rejecting filter"
        found.append((idx + 1, var, why))
    return found


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--root", type=Path, default=PEEPHOLE,
                    help="directory to scan (default: every backend under src/backend)")
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
        for line, var, why in violations(path.read_text()):
            rel = path.relative_to(ROOT) if path.is_relative_to(ROOT) else path
            print(f"FAIL: {rel}:{line}: `{var}` is heap-copied before `{why}` "
                  f"can reject it; filter on borrowed text first "
                  f"(and drop the copy entirely unless a `store` mutation "
                  f"outlives the borrow)")
            total += 1
    if total:
        print(f"check_peephole_alloc_before_filter: {total} violation(s)")
        return 1
    print(f"check_peephole_alloc_before_filter: PASS ({len(files)} files, "
          f"no allocation before a rejecting filter)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
