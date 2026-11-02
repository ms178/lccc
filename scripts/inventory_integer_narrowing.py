#!/usr/bin/env python3
"""Inventory explicit i64 extraction/evaluation for semantic review, not proof.

Scan tracked Rust source at a pinned revision (default HEAD), emitting stable
CSV on stdout. Count lexical method invocations, excluding whole-line comments;
inline strings/block comments can still produce candidates. `test_region` is
only a heuristic based on the first #[cfg(test)] marker. The inventory never
labels a backend call safe merely because it emits bits.

Usage: python3 scripts/inventory_integer_narrowing.py --revision origin/main
"""
from __future__ import annotations

import argparse
import csv
import re
import subprocess
import sys

CALL = re.compile(r"\.(to_i64|eval_i64)\s*\(")
FUNCTION = re.compile(r"\bfn\s+([A-Za-z_][A-Za-z_0-9]*)\s*(?:<[^>]*>)?\s*\(")


def git(*args: str) -> str:
    return subprocess.check_output(["git", *args], text=True)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--revision", default="HEAD")
    args = parser.parse_args()
    revision = git("rev-parse", "--verify", args.revision + "^{commit}").strip()
    paths = sorted(p for p in git("ls-tree", "-r", "--name-only", revision, "src").splitlines()
                   if p.endswith(".rs"))
    writer = csv.writer(sys.stdout, lineterminator="\n")
    writer.writerow(["revision", "path", "line", "column", "method", "region",
                     "preceding_function", "test_region", "source"])
    for path in paths:
        text = git("show", revision + ":" + path)
        function = ""
        test_region = False
        if path.startswith("src/backend/"):
            region = "backend-width-or-emission-review"
        elif path.startswith(("src/passes/", "src/common/const", "src/ir/lowering/const")):
            region = "semantic-decision-priority"
        else:
            region = "other-semantic-or-representation-review"
        for number, line in enumerate(text.splitlines(), 1):
            if "#[cfg(test)]" in line:
                test_region = True
            if line.lstrip().startswith("//"):
                continue
            declaration = FUNCTION.search(line)
            if declaration:
                function = declaration.group(1)
            for call in CALL.finditer(line):
                writer.writerow([revision, path, number, call.start() + 1, call.group(1),
                                 region, function, str(test_region).lower(), line.strip()])


if __name__ == "__main__":
    main()
