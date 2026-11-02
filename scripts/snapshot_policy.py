#!/usr/bin/env python3
"""Fail-closed staged-diff policy for lccc-snapshot.sh (no compiler required).

ALLOW_MODES means *only* regular-file executable-bit changes. Symlink/submodule
changes and deletions require the separately named structural override.
Raw -z Git output avoids ambiguous paths containing whitespace or newlines.
"""
from __future__ import annotations

import os
import subprocess
import sys


def inspect(raw: bytes, *, allow_modes: bool = False,
            allow_structural: bool = False) -> tuple[list[str], list[str]]:
    fields = raw.split(b"\0")
    errors, allowed = [], []
    i = 0
    while i < len(fields) and fields[i]:
        header = fields[i].decode("ascii").split()
        if len(header) != 5 or not header[0].startswith(":"):
            raise ValueError("malformed git --raw -z record")
        old, new, status = header[0][1:], header[1], header[4]
        i += 1
        if i >= len(fields):
            raise ValueError("missing git path")
        path = os.fsdecode(fields[i])
        i += 1
        if status[0] in "RC":
            if i >= len(fields):
                raise ValueError("missing rename destination")
            path += " -> " + os.fsdecode(fields[i])
            i += 1
        if (status[0] == "D" or (old != "000000" and new != old)
                or (old == "000000" and new not in {"100644", "100755"})):
            detail = f"{status}: {path!r} ({old} -> {new})"
            exec_bit_only = {old, new} == {"100644", "100755"}
            if allow_structural or (allow_modes and exec_bit_only):
                allowed.append(detail)
            else:
                errors.append(detail)
    return errors, allowed


def main() -> int:
    raw = subprocess.check_output(["git", "diff", "--cached", "--raw", "-z",
                                   "--no-renames", "--no-abbrev"])
    errors, allowed = inspect(
        raw, allow_modes=os.environ.get("LCCC_SNAPSHOT_ALLOW_MODES") == "1",
        allow_structural=os.environ.get("LCCC_SNAPSHOT_ALLOW_STRUCTURAL") == "1")
    for item in allowed:
        print(f"snapshot guard: explicit override ALLOWED {item}", file=sys.stderr)
    for item in errors:
        print(f"snapshot guard: REJECTED {item}", file=sys.stderr)
    return bool(errors)


if __name__ == "__main__":
    sys.exit(main())
