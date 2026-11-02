#!/usr/bin/env python3
"""Archived bulk-evidence gate: no path listed in ARCHIVED-BULK.md may be tracked.

Why this exists
---------------
PR #829 re-added 89 files / 749,607 lines of raw evidence that the archive
policy had already moved out of the tree: the 76 pr-only 2026-10-14D
shared-binding profiles and the 13 pr824-final-audit bundles. Every one of
those paths is a row in ``engineering/evidence/ARCHIVED-BULK.md``, whose
preface states the files are preserved byte-exact in
``artifacts/evidence-archive/engineering-evidence-bulk-s59.tar.xz`` and that
no source, script, test or workflow reads them. A three-way merge happily
resurrects them because the PR side is a pure addition -- the merge sees
nothing to conflict with. Reviewing that by hand is exactly the kind of
audit that does not scale; the index is machine-readable, so enforce it.

This is a path-absence check, deliberately not a size or extension check:
the tree still tracks 13 smaller ``.tar.xz`` bundles and 83 files over
100 KiB under ``engineering/evidence/`` that were never archived, so a
generic size/extension rule would be red on a clean tree and would get
disabled instead of trusted.

What it checks
--------------
Parse the ``| sha256 | bytes | path |`` table in ARCHIVED-BULK.md, fail
closed if the table is missing or parses to zero rows (the index format
changed: update this gate, do not let it pass silently), then fail if any
listed path is tracked by git. Untracked working-tree copies are out of
scope: the working tree is not the deliverable, the index is.

Exit codes: 0 all archived paths absent, 1 a path is tracked again or the
index failed to parse, 2 could not run the check.

Test seam: ``--paths-file FILE`` substitutes the tracked-path list for
``git ls-files``; ``--self-test`` exercises the parser and the offender
logic against synthetic input.
"""
from __future__ import annotations

import argparse
import os
import re
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
INDEX = os.path.join(ROOT, "engineering", "evidence", "ARCHIVED-BULK.md")

# One table row exactly as the archive index writes it: sha256, byte count,
# backticked repo-relative path. Requiring the full row (not just a
# backticked path) means prose that merely mentions an archived file in a
# bullet list cannot flip the gate either way.
ROW = re.compile(r"^\| `([0-9a-f]{64})` \| ([0-9]+) \| `([^`]+)` \|$")


def parse_rows(text: str) -> list[tuple[str, str]]:
    """Return [(sha256, path)] for every archive-index table row."""
    rows: list[tuple[str, str]] = []
    for line in text.splitlines():
        m = ROW.match(line.strip())
        if m:
            sha, _bytes, path = m.groups()
            rows.append((sha, path))
    return rows


def load_index(path: str = INDEX) -> list[tuple[str, str]]:
    """Parse the archive index, fail-closed on a missing or empty table."""
    try:
        with open(path, encoding="utf-8") as fh:
            text = fh.read()
    except OSError as exc:
        raise SystemExit(f"check_archived_bulk_absent: cannot read {path}: {exc}")
    rows = parse_rows(text)
    if not rows:
        raise SystemExit(
            f"check_archived_bulk_absent: 0 table rows parsed from {path}; "
            "the archive index format changed -- update this gate's parser "
            "before trusting a pass"
        )
    return rows


def tracked_paths(paths_file: str | None = None) -> set[str]:
    """Tracked, non-deleted paths -- the working tree is not the deliverable."""
    if paths_file is not None:
        with open(paths_file, encoding="utf-8") as fh:
            return {line.strip() for line in fh if line.strip()}
    out = subprocess.run(
        ["git", "-C", ROOT, "ls-files", "-z", "--cached"],
        capture_output=True,
        text=True,
    )
    if out.returncode != 0:
        raise SystemExit(
            "check_archived_bulk_absent: git ls-files failed: "
            + (out.stderr.strip() or f"exit {out.returncode}")
        )
    return {p for p in out.stdout.split("\0") if p}


def offenders(rows: list[tuple[str, str]], tracked: set[str]) -> list[tuple[str, str]]:
    """[(sha256, path)] for archived paths that are tracked again."""
    return [(sha, path) for sha, path in rows if path in tracked]


def self_test() -> int:
    """Parser + offender logic against synthetic input, no git required."""
    good = (
        "| sha256 | bytes | path |\n"
        "|---|---:|---|\n"
        "| `" + "a" * 64 + "` | 10 | `engineering/evidence/x/a.bin` |\n"
        "| `" + "b" * 64 + "` | 20 | `engineering/evidence/y/b.bin` |\n"
    )
    rows = parse_rows(good)
    assert len(rows) == 2, f"expected 2 rows, got {rows}"
    assert not parse_rows("| not a table | row |"), "unparsable line must not parse"
    if offenders(rows, set()):
        raise AssertionError("clean tracked set must yield no offenders")
    hit = offenders(rows, {"engineering/evidence/y/b.bin", "unrelated.c"})
    if [(s, p) for s, p in hit] != [("b" * 64, "engineering/evidence/y/b.bin")]:
        raise AssertionError(f"offender mismatch: {hit}")
    print("PASS check_archived_bulk_absent self-test")
    return 0


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument(
        "--paths-file",
        metavar="FILE",
        help="read the tracked-path list from FILE (one per line) instead of git",
    )
    ap.add_argument(
        "--self-test",
        action="store_true",
        help="run parser/offender unit checks against synthetic input",
    )
    args = ap.parse_args(argv)
    if args.self_test:
        return self_test()

    rows = load_index()
    tracked = tracked_paths(args.paths_file)
    bad = offenders(rows, tracked)
    if bad:
        print(
            f"FAIL check_archived_bulk_absent: {len(bad)} archived path(s) "
            "are tracked again:",
            file=sys.stderr,
        )
        for sha, path in bad:
            print(f"  {path}  (sha256 {sha[:12]}...)", file=sys.stderr)
        print(
            "Remove them from the tree; the bytes live in the archive named "
            "by engineering/evidence/ARCHIVED-BULK.md. Tracked files may not "
            "come back just because a merge resurrected them.",
            file=sys.stderr,
        )
        return 1
    print(
        f"PASS check_archived_bulk_absent: {len(rows)} archived paths, none tracked"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
