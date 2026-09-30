#!/usr/bin/env python3
"""Conflict-marker gate: no tracked file may contain a merge conflict marker.

Why this exists
---------------
A rebase onto a moving upstream auto-merges most files and stops only on real
content conflicts. Everything else is merged silently -- *including* files
where the merge driver could not reconcile the hunks and therefore wrote the
markers into the result. If those files are then staged and committed, the
markers ship.

The failure mode is nasty because it is not always loud. A marker inside a
Rust source is a compile error, but inside a shell script it is a *syntax
error at the line the shell happens to reach*: the gates registered before it
still run and still report PASS, so a green run can be a run that silently
never executed half the suite. The rebase that motivated this gate broke
`ci_local.sh` mid-file, and the run before the abort had already printed PASS
for the gates above the damage.

So: check every tracked text file, report file:line, fail closed.

What counts as a marker
-----------------------
The three lines git writes -- ``<<<<<<<``, ``=======``, ``>>>>>>>`` -- but
only when ``=======`` is preceded by an opening marker and followed by a
closing one, i.e. the real three-line shape. That keeps ordinary prose and
reStructuredText/RustDoc underlines (``====`` under a heading) out of the
report, which a naive "any line of seven ``=``" check would not.

Opt-outs
--------
A path listed in ``scripts/conflict_marker_allowlist.txt`` (one glob per line,
``#`` comments allowed) is skipped. It exists for fixtures that must contain a
real conflict to test a merge tool; there are none today, and an unused
allowlist entry is itself worth deleting rather than leaving to rot.
"""
from __future__ import annotations

import fnmatch
import os
import re
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ALLOWLIST = os.path.join(ROOT, "scripts", "conflict_marker_allowlist.txt")

# git's three markers. The capture group keeps the seven-character run intact
# so a pathologically long line of '<' is not mistaken for one.
OPEN = re.compile(r"^<{7}(?: .*)?$")
MID = re.compile(r"^={7}$")
CLOSE = re.compile(r"^>{7}(?: .*)?$")

# Only text we could plausibly have merged. Binary is skipped by extension
# rather than by sniffing, which is enough here and keeps the gate fast.
BINARY_EXT = {
    ".png", ".jpg", ".jpeg", ".gif", ".pdf", ".zip", ".gz", ".bz2", ".xz",
    ".tar", ".o", ".a", ".so", ".pyc", ".class", ".jar", ".wasm", ".bin",
}
SKIP_DIRS = {".git", "target", "node_modules", ".venv", "__pycache__"}


def tracked_files() -> list[str]:
    """Tracked, non-deleted files -- the working tree is not the deliverable."""
    out = subprocess.run(
        ["git", "-C", ROOT, "ls-files", "-z", "--cached"],
        capture_output=True, text=True, check=True,
    ).stdout
    return [p for p in out.split("\0") if p]


def load_allowlist() -> list[str]:
    if not os.path.exists(ALLOWLIST):
        return []
    pats = []
    with open(ALLOWLIST) as fh:
        for line in fh:
            line = line.strip()
            if line and not line.startswith("#"):
                pats.append(line)
    return pats


def scan(path: str) -> list[tuple[int, str]]:
    """Return [(lineno, marker_kind)] for every complete conflict in `path`."""
    full = os.path.join(ROOT, path)
    if not os.path.isfile(full):
        return []
    if os.path.splitext(path)[1].lower() in BINARY_EXT:
        return []
    try:
        with open(full, encoding="utf-8") as fh:
            lines = fh.read().splitlines()
    except (UnicodeDecodeError, OSError):
        return []  # not UTF-8 text; nothing to match

    hits: list[tuple[int, str]] = []
    open_at: int | None = None
    for i, line in enumerate(lines, 1):
        if open_at is None:
            if OPEN.match(line):
                open_at = i
        else:
            # A second `<<<<<<<` means the first never closed. Restart at the
            # new one rather than keeping a stale anchor: the stale line is
            # almost certainly prose (a quoted diff), and reporting it would
            # blame the wrong line.
            if OPEN.match(line):
                open_at = i
                continue
            # Require the real three-line shape before reporting anything, so
            # a stray '=====' underline in prose is not flagged. The search is
            # BOUNDED: a conflict is `<<<<<<<`, `=======`, `>>>>>>>` in that
            # order, so hitting another OPEN or another MID first means this
            # region is not a conflict. Scanning to EOF instead let an opening
            # marker in one paragraph pair with a closing marker in a completely
            # unrelated part of the file.
            if MID.match(line):
                closed = False
                for j in range(i, len(lines)):
                    if CLOSE.match(lines[j]):
                        hits.append((open_at, lines[open_at - 1].strip()))
                        closed = True
                        break
                    if OPEN.match(lines[j]) or MID.match(lines[j]):
                        break  # another region begins; this one is not a conflict
                if not closed:
                    open_at = None  # opening with no closing: not a conflict
    return hits


def main() -> int:
    allow = load_allowlist()
    findings: list[tuple[str, int, str]] = []
    for path in tracked_files():
        if any(fnmatch.fnmatch(path, pat) for pat in allow):
            continue
        for lineno, marker in scan(path):
            findings.append((path, lineno, marker))

    if not findings:
        print("PASS: no merge conflict markers in tracked files")
        return 0

    print(f"FAIL: {len(findings)} unresolved merge conflict(s) in tracked files",
          file=sys.stderr)
    for path, lineno, marker in findings:
        print(f"  {path}:{lineno}: {marker}", file=sys.stderr)
    print(
        "\nA marker here means the file was committed with the conflict still\n"
        "in it. In a shell script that is a syntax error only when the shell\n"
        "reaches the line, so gates above it can report PASS for a suite that\n"
        "never ran. Resolve with `git checkout --ours/--theirs`, or `git add`\n"
        "after deleting the markers, then re-run.",
        file=sys.stderr,
    )
    return 1


if __name__ == "__main__":
    sys.exit(main())
