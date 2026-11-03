#!/usr/bin/env python3
"""Differentially check tag redefinitions under C17 and C23.

C23 (N3037) permits compatible tagged-type redefinitions but still rejects
incompatible definitions. Rows compare verdicts, not diagnostic spelling:
GCC and Clang format the same constraint differently.
"""
from __future__ import annotations

import argparse
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

# label, standard, source, expected acceptance
CASES = (
    ("duplicate-struct", "gnu17", "struct S { int x; }; struct S { long y; };", False),
    ("duplicate-union", "gnu17", "union U { int x; }; union U { long y; };", False),
    ("duplicate-enum", "gnu17", "enum E { E0 }; enum E { E1 };", False),
    ("cross-kind-tag", "gnu17", "struct X { int x; }; union X { int y; };", False),
    (
        "macro-expanded-duplicate",
        "gnu17",
        "#define DEF struct M { int x; };\nDEF\nDEF",
        False,
    ),
    ("forward-completion", "gnu17", "struct S; struct S; struct S { int x; };", True),
    (
        "single-definition-reuse",
        "gnu17",
        "struct S { int x; }; int f(void) { struct S s = {1}; return sizeof(s) != sizeof(struct S); }",
        True,
    ),
    (
        "inner-scope-shadow",
        "gnu17",
        "struct S { int x; }; int f(void) { struct S { char y; } inner = {1}; return sizeof(inner) != sizeof(char); }",
        True,
    ),
    ("disjoint-block-tags", "gnu17", "int f(void) { { struct S { int x; }; } { struct S { long y; }; } return 0; }", True),
    ("same-block-duplicate", "gnu17", "int f(void) { struct S { int x; }; struct S { long y; }; return 0; }", False),
    ("c23-compatible-local-struct", "gnu23", "int f(void) { struct S { int x; }; struct S { int x; }; return sizeof(struct S); }", True),
    ("c23-incompatible-local-struct", "gnu23", "int f(void) { struct S { int x; }; struct S { long x; }; return sizeof(struct S); }", False),
    ("c23-compatible-struct", "gnu23", "struct S { int x; }; struct S { int x; };", True),
    ("c23-incompatible-struct", "gnu23", "struct S { int x; }; struct S { long x; };", False),
    ("c23-compatible-enum", "gnu23", "enum E { A }; enum E { A };", True),
    ("c23-incompatible-enum", "gnu23", "enum E { A }; enum E { B };", False),
    ("c23-cross-kind-tag", "gnu23", "struct X { int x; }; union X { int x; };", False),
    # Integration contracts: definition spans, designated-init metadata and
    # scoped enum identities must coexist when main and the ABI branch merge.
    ("designated-duplicate", "gnu17", "struct __attribute__((designated_init)) S { int x; }; struct S { int y; };", False),
    ("designated-inner-shadow", "gnu17", "struct __attribute__((designated_init)) S { int x; }; int f(void) { struct S { char y; }; struct S s = {1}; return s.y; }", True),
    ("enum-disjoint-blocks", "gnu17", "int f(void) { { enum E { A }; } { enum E { A }; } return 0; }", True),
    ("enum-outer-duplicate-after-shadow", "gnu17", "enum E { A }; int f(void) { enum E { B }; return B; } enum E { C };", False),
    ("enum-fixed-forward-completion", "gnu23", "enum E : int; enum E : int { A };", True),
    ("enum-fixed-inner-shadow", "gnu23", "enum E : int { A }; int f(void) { enum E : unsigned { B }; return B; }", True),
)


def verdict(compiler: list[str], standard: str, source: Path) -> tuple[bool, str]:
    try:
        result = subprocess.run(
            [*compiler, f"-std={standard}", "-fsyntax-only", str(source)],
            cwd=ROOT,
            text=True,
            capture_output=True,
            timeout=30,
            check=False,
        )
    except (OSError, subprocess.TimeoutExpired) as exc:
        return False, str(exc)
    return result.returncode == 0, result.stderr


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--lccc", default="target/fastbuild/lccc")
    parser.add_argument("--gcc", default="gcc-16")
    parser.add_argument("--clang", default="clang-23")
    args = parser.parse_args()
    compilers = (("LCCC", [args.lccc]), ("GCC 16.2", [args.gcc]), ("Clang 23.1.4", [args.clang]))
    failures: list[str] = []

    with tempfile.TemporaryDirectory(prefix="lccc-tag-redefinitions-") as directory:
        root = Path(directory)
        for name, standard, text, expected in CASES:
            source = root / f"{name}.c"
            source.write_text(text + "\n", encoding="utf-8")
            results = [
                (label, *verdict(command, standard, source))
                for label, command in compilers
            ]
            for label, accepted, stderr in results:
                if accepted != expected:
                    first_line = stderr.splitlines()[0] if stderr else "<no diagnostic>"
                    failures.append(
                        f"{name} (-std={standard}): {label} expected "
                        f"{'accept' if expected else 'reject'}, got "
                        f"{'accept' if accepted else 'reject'} ({first_line})"
                    )
            if all(accepted == expected for _, accepted, _ in results):
                print(
                    f"tag {name} (-std={standard}): PASS "
                    f"({'accepted' if expected else 'rejected'} by all)"
                )

    if failures:
        for failure in failures:
            print(f"FAIL: {failure}", file=sys.stderr)
        return 1
    print(f"tag redefinitions: PASS ({len(CASES)} cases × {len(compilers)} compilers)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
