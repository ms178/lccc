#!/usr/bin/env python3
"""Ratchet gate for `TODO`/`FIXME` markers in the compiler source tree.

WHY THIS EXISTS
---------------
`backlog.md` §15 (the 2026-11-13 code-debt census) adjudicated every marker in
`src/`.  Two of the 47 were *stale*: they described capabilities that already
existed elsewhere in the tree, so a reader would either duplicate finished work
(CD-02: global register-variable writes, implemented in
`expr_assign::lower_assign`) or, worse, "fix" a TODO into a miscompile (CD-01:
re-enabling the i686 `div_by_const` pass would re-introduce the I64 widening
sequence the i686 backend truncates).

A census that is re-derived by hand every session decays.  This gate makes the
decay mechanical:

  * a marker must name its backlog item — ``TODO(CD-21):`` — or declare that it
    has none — ``TODO(no-item): <reason>``;
  * every named id must exist in ``backlog.md``;
  * a marker whose id is adjudicated CLOSED/fixed/stale must be deleted, not
    left behind;
  * and the *legacy* unqualified markers present when the census was taken are
    pinned in ``scripts/todo_census_baseline.txt`` by file and count, so the
    count can only shrink.  Encoding the historical debt as a number rather than
    as text means ordinary refactors (which move lines) do not invalidate it,
    while adding a new unqualified marker always does.

The baseline is a RATCHET: lowering a count requires editing the baseline in the
same commit (that is the point — the file stays an accurate description of the
tree), and raising it is refused outright.

Usage::

    scripts/check_todo_census.py              # report only (default)
    scripts/check_todo_census.py --gate       # exit 1 on any violation
    scripts/check_todo_census.py --self-test  # mutation-verify the gate itself
    scripts/check_todo_census.py --write-baseline   # print a fresh baseline

Exit status: 0 clean/shrunk-and-reported, 1 violation (gate mode), 2 usage.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SRC = ROOT / "src"
BACKLOG = ROOT / "backlog.md"
BASELINE = ROOT / "scripts" / "todo_census_baseline.txt"

# `TODO(id):` / `FIXME(id):`. A bare `XXX` is deliberately NOT part of the
# census: it is used as a placeholder inside string literals in the lexer
# (`\uXXXX`) and in assembly-pattern parsers (`movslq %XXXd, %XXX`), both of
# which are code, not claims about the future.
MARKER = re.compile(r"\b(TODO|FIXME)\b(\s*\(([A-Za-z0-9_.-]+)\))?")
# An item is closed when its backlog heading or its first line says so in the
# vocabulary the census uses (§15.1).
CLOSED_MARK = re.compile(r"\b(CLOSED|fixed|STALE|DONE)\b", re.IGNORECASE)


def backlog_text() -> str:
    try:
        return BACKLOG.read_text(encoding="utf-8", errors="replace")
    except OSError as error:  # pragma: no cover - the backlog is part of the tree
        raise SystemExit(f"error: cannot read {BACKLOG}: {error}")


def item_status(text: str) -> dict[str, bool]:
    """Map item id -> closed(True)/open(False) from the backlog headings.

    Headings look like ``#### CD-01 · STALE (fixed) · R — …``.  A segment is
    delimited by the next heading, so only the item's own prose can close it.
    """
    status: dict[str, bool] = {}
    heading = re.compile(r"^#{3,4}\s+([A-Z]{2}-\d+)\b(.*)$", re.MULTILINE)
    matches = list(heading.finditer(text))
    for index, match in enumerate(matches):
        end = matches[index + 1].start() if index + 1 < len(matches) else len(text)
        body = text[match.end():end]
        first = (match.group(2) + "\n" + body.split("\n\n", 1)[0])[:400]
        status[match.group(1)] = bool(CLOSED_MARK.search(first))
    return status


def comment_spans(text: str):
    """Yield (start, end) offsets of comment text, skipping string literals.

    A marker only counts when it is *comment text*.  Without this, the generated
    Unicode-name blob in `frontend/lexer/ucn_name_table.rs` counts twice: it
    contains real character names with the substring `TODO` inside a byte-string
    literal, which is data, not a claim about the future.  Rust literals are
    handled for the four forms that occur in this tree (`"…"`, `b"…"`, `r"…"`,
    `r#"…"#`) with backslash escaping in the non-raw forms; `'a'` lifetimes are not
    relevant because they cannot contain a marker word.
    """
    index, size = 0, len(text)
    while index < size:
        char = text[index]
        if char == "/" and text.startswith("//", index):
            end = text.find("\n", index)
            end = size if end < 0 else end
            yield (index, end)
            index = end
        elif char == "/" and text.startswith("/*", index):
            end = text.find("*/", index + 2)
            end = size if end < 0 else end + 2
            yield (index, end)
            index = end
        elif char == '"' or (char in "br" and _literal_start(text, index)):
            index = _skip_literal(text, index)
        else:
            index += 1


def _literal_start(text: str, index: int) -> bool:
    """True when a `b`/`r` prefix at `index` starts a string literal."""
    if text[index] == "b":
        return text.startswith('b"', index)
    # raw string: r"…" or r#"…"#
    cursor = index + 1
    while cursor < len(text) and text[cursor] == "#":
        cursor += 1
    return cursor > index and cursor < len(text) and text[cursor] == '"'


def _skip_literal(text: str, index: int) -> int:
    """Advance past the string literal starting at `index`."""
    cursor = index
    raw = 0
    while cursor < len(text) and text[cursor] == "#":
        raw += 1
        cursor += 1
    if text[cursor] == "b" or text[cursor] == "r":
        cursor += 1
        if text[cursor - 1] == "r":
            while cursor < len(text) and text[cursor] == "#":
                raw += 1
                cursor += 1
    if cursor >= len(text) or text[cursor] != '"':
        return index + 1
    cursor += 1
    if raw:  # raw string: closes at "###… with `raw` hashes
        terminator = '"' + "#" * raw
        end = text.find(terminator, cursor)
        return len(text) if end < 0 else end + len(terminator)
    while cursor < len(text):
        if text[cursor] == "\\":
            cursor += 2
            continue
        if text[cursor] == '"':
            return cursor + 1
        cursor += 1
    return len(text)


def scan(root: Path) -> dict[str, list[tuple[int, str, str | None]]]:
    """Return {relative path: [(line, kind, qualified id or None), …]}."""
    found: dict[str, list[tuple[int, str, str | None]]] = {}
    for path in sorted(root.rglob("*.rs")):
        try:
            text = path.read_text(encoding="utf-8", errors="replace")
        except OSError:  # pragma: no cover
            continue
        if "TODO" not in text and "FIXME" not in text:
            continue
        spans = list(comment_spans(text))
        if not spans:
            continue
        hits: list[tuple[int, str, str | None]] = []
        for match in MARKER.finditer(text):
            start = match.start()
            if not any(begin <= start < end for begin, end in spans):
                continue
            number = text.count("\n", 0, start) + 1
            hits.append((number, match.group(1), match.group(3)))
        if hits:
            found[path.relative_to(root).as_posix()] = hits
    return found


def read_baseline(path: Path) -> dict[str, int]:
    counts: dict[str, int] = {}
    if not path.exists():
        return counts
    for raw in path.read_text(encoding="utf-8").splitlines():
        line = raw.split("#", 1)[0].strip()
        if not line:
            continue
        try:
            name, count = line.rsplit(":", 1)
            counts[name.strip()] = int(count)
        except ValueError as error:
            raise SystemExit(f"error: malformed baseline line {raw!r}: {error}")
    return counts


def evaluate(root: Path, baseline: dict[str, int], status: dict[str, bool]) -> list[str]:
    problems: list[str] = []
    found = scan(root)
    unqualified: dict[str, int] = {}
    for name, hits in found.items():
        for line, kind, ident in hits:
            where = f"{name}:{line}"
            if ident is None:
                unqualified[name] = unqualified.get(name, 0) + 1
                continue
            if ident == "no-item":
                continue
            if ident not in status:
                problems.append(f"{where}: {kind}({ident}) names no item in backlog.md")
            elif status[ident]:
                problems.append(
                    f"{where}: {kind}({ident}) but the item is closed/fixed/stale"
                    " — delete the marker (§15.1)"
                )
    for name, count in sorted(unqualified.items()):
        allowed = baseline.get(name, 0)
        if count > allowed:
            problems.append(
                f"{name}: {count} unqualified marker(s), baseline allows {allowed}"
                " — qualify it as TODO(<item-id>): or TODO(no-item): <reason>"
            )
        elif count < allowed:
            problems.append(
                f"{name}: {count} unqualified marker(s), baseline still says {allowed}"
                " — lower the baseline in the same commit (it is a ratchet)"
            )
    for name, allowed in sorted(baseline.items()):
        if name not in unqualified and allowed:
            problems.append(
                f"{name}: baseline lists {allowed} unqualified marker(s) but the file"
                " has none — remove the baseline entry (ratchet)"
            )
    return problems


def write_baseline(root: Path) -> str:
    found = scan(root)
    lines = [
        "# Unqualified TODO/FIXME ratchet for src/ (see backlog.md §15.6).",
        "#",
        "# Counts may only SHRINK.  Qualify a marker as TODO(<item-id>): or",
        "# TODO(no-item): <reason>, then lower this count in the same commit.",
        "# Regenerate with: scripts/check_todo_census.py --write-baseline",
    ]
    for name in sorted(found):
        count = sum(1 for _, _, ident in found[name] if ident is None)
        if count:
            lines.append(f"{name}:{count}")
    return "\n".join(lines) + "\n"


def self_test() -> int:
    """Mutation-verify: each rule must fail on a fixture that breaks it."""
    import tempfile

    failures = 0

    def check(name: str, tree: dict[str, str], backlog: str,
              baseline: dict[str, int], expect_clean: bool) -> None:
        nonlocal failures
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            for relative, text in tree.items():
                target = root / relative
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_text(text, encoding="utf-8")
            saved_backlog = globals()["BACKLOG"]
            globals()["BACKLOG"] = root / "backlog.md"
            (root / "backlog.md").write_text(backlog, encoding="utf-8")
            try:
                problems = evaluate(root, baseline, item_status(backlog))
            finally:
                globals()["BACKLOG"] = saved_backlog
        clean = not problems
        ok = clean == expect_clean
        print(f"  {'ok  ' if ok else 'FAIL'} {name}: {problems if problems else 'clean'}")
        if not ok:
            failures += 1

    open_item = "#### CD-21 · P0 · R — unroll loops\n\n**Why.** body text.\n"
    closed_item = "#### CD-01 · STALE (fixed) · R — stale claim\n\n**Why.** body.\n"
    check("qualified open marker passes", {"src/a.rs": "// TODO(CD-21): do it\n"},
          open_item, {}, True)
    check("qualified marker for a missing item fails", {"src/a.rs": "// TODO(CD-99): x\n"},
          open_item, {}, False)
    check("qualified marker on a closed item fails",
          {"src/a.rs": "// TODO(CD-01): x\n"}, closed_item, {}, False)
    check("no-item escape hatch passes", {"src/a.rs": "// TODO(no-item): reason\n"},
          open_item, {}, True)
    check("unqualified marker within baseline passes", {"src/a.rs": "// TODO later\n"},
          open_item, {"src/a.rs": 1}, True)
    check("unqualified marker above baseline fails",
          {"src/a.rs": "// a TODO\n// another TODO\n"}, open_item, {"src/a.rs": 1}, False)
    check("baseline that no longer matches fails (ratchet must shrink)",
          {"src/a.rs": "// clean\n"}, open_item, {"src/a.rs": 1}, False)
    check("string-literal placeholders are not markers",
          {"src/a.rs": 'let s = "\\uXXXX"; let p = "movslq %XXXd, %XXX";\n'},
          open_item, {}, True)
    check("a marker inside a byte-string blob is data, not a claim",
          {"src/a.rs": 'static B: &[u8] = b"\\x00ABACUS\\x00TODO THING\\x00";\n'},
          open_item, {}, True)
    check("a marker in a line comment counts", {"src/a.rs": "fn f() {} // TODO(x)\n"},
          "#### CD-21 · P0 — x\n", {}, False)
    check("a marker in a block comment counts",
          {"src/a.rs": "/* TODO(CD-21): later */\nfn f() {}\n"}, open_item, {}, True)
    print(f"self-test: {'PASS' if failures == 0 else f'{failures} FAILED'}")
    return 0 if failures == 0 else 1


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--gate", action="store_true",
                        help="exit non-zero on any violation (CI mode)")
    parser.add_argument("--self-test", action="store_true", help="mutation-verify the gate")
    parser.add_argument("--write-baseline", action="store_true",
                        help="print a fresh baseline for the current tree")
    parser.add_argument("--root", type=Path, default=SRC, help=argparse.SUPPRESS)
    args = parser.parse_args()

    if args.self_test:
        return self_test()
    if args.write_baseline:
        sys.stdout.write(write_baseline(args.root))
        return 0

    problems = evaluate(args.root, read_baseline(BASELINE), item_status(backlog_text()))
    if not problems:
        print("todo census: clean")
        return 0
    for problem in problems:
        print(("error: " if args.gate else "note: ") + problem)
    if args.gate:
        print(f"todo-census gate FAILED: {len(problems)} problem(s) (backlog.md §15.6)")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
