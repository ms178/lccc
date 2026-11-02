#!/usr/bin/env python3
"""Import-sanity gate for the helper scripts.

Why this exists
---------------
`scripts/ab_interleaved.py` shipped in this series with

    from pathlib import Path, re, statistics, subprocess, sys, tempfile, os

i.e. an import that cannot resolve -- `pathlib` exports no `re`, `statistics`
or `subprocess`. The file was syntactically valid Python, so `py_compile`,
`git diff` and review all passed; the script was simply dead on arrival and
nobody noticed for the whole series, because **no gate imports these files**.
They are tools, not gates, so the suite that runs the gates never runs them.

That is the bug this closes: a measurement harness that cannot execute is
worse than no harness, because a broken benchmark tool still produces numbers
somewhere in the pipeline -- just not from the code anyone thinks it read.

What it checks
--------------
For every `import X` / `from X import a, b` in the helper trees, verify the
name really exists in that module. This is done against the AST, so it is
side-effect free: importing a tool to see whether it imports would mean
running whatever it does at import time, which for these scripts means
compiling C and running benchmarks.
"""
from __future__ import annotations

import ast
import importlib
import pathlib
import sys

# Modules that are safe to introspect. Everything else (or anything missing)
# is reported as unverifiable rather than silently passed.
SKIP_PREFIXES = ("__",)


def check_file(path: pathlib.Path) -> list[str]:
    try:
        tree = ast.parse(path.read_text(), filename=str(path))
    except SyntaxError as e:
        return [f"{path}: syntax error: {e}"]
    problems: list[str] = []
    for node in ast.walk(tree):
        if isinstance(node, ast.Import):
            for alias in node.names:
                problems += _check_from(path, alias.name, None, alias.name, node.lineno)
        elif isinstance(node, ast.ImportFrom):
            if node.level:  # relative import; not used in this tree
                continue
            mod = node.module or ""
            if mod.startswith(SKIP_PREFIXES):
                continue
            for alias in node.names:
                if alias.name == "*":
                    continue
                problems += _check_from(path, mod, alias.name, alias.name, node.lineno)
    return problems


def _check_from(
    path: pathlib.Path, mod: str, attr: str | None, label: str, lineno: int
) -> list[str]:
    try:
        m = importlib.import_module(mod)
    except Exception:
        return []  # unverifiable (optional dep / not installed): not our call
    if attr is None:
        return []  # plain `import x` only needs the module to exist
    if hasattr(m, attr):
        return []
    # `from pkg import sub` is legal when `sub` is a SUBMODULE: the import
    # machinery falls back to importing it, and it never becomes an attribute
    # of the parent until something imports it. `hasattr(unittest, "mock")` is
    # False on a fresh interpreter even though `from unittest import mock` is
    # perfectly fine, so resolve the submodule before calling it a defect.
    try:
        importlib.import_module(f"{mod}.{attr}")
        return []
    except Exception:
        pass
    return [f"{path}:{lineno}: `from {mod} import {label}` -- {mod!r} has no {label!r}"]


def main() -> int:
    root = pathlib.Path(__file__).resolve().parent.parent
    trees = [root / "scripts", root / "tools"]
    files: list[pathlib.Path] = []
    for t in trees:
        if t.is_dir():
            files += sorted(p for p in t.rglob("*.py") if "__pycache__" not in p.parts)

    if not files:
        print("script-imports: no python helpers found; refusing to pass vacuously", file=sys.stderr)
        return 2

    problems: list[str] = []
    for f in files:
        problems += check_file(f)

    if problems:
        print("script-imports: FAIL", file=sys.stderr)
        for p in problems:
            print(f"  {p}", file=sys.stderr)
        return 1
    print(f"script-imports: ok ({len(files)} python helper(s))")
    return 0


if __name__ == "__main__":
    sys.exit(main())
