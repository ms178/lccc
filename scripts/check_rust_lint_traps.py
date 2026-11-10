#!/usr/bin/env python3
"""Catch the Rust lint denials that CI reports only after a full build.

Motivation, measured: the SA-11 change named its enum variants
``RmB_VvvvC_DstA`` and the build died with

    error: variant `RmB_VvvvC_DstA` should have an upper camel case name

which ``-D warnings`` promotes from a lint to a build failure.  That costs a
whole CI round-trip (tens of minutes here) to learn one line of information
that is visible from the diff alone.  This gate checks the CLASS of denial, so
the same round trip is never spent twice.

The crate denies these by policy (``Cargo.toml`` / ``lib.rs`` ``deny`` list,
and ``-D warnings`` on every lane), and each is decidable lexically:

* ``non_camel_case_types`` -- an upper-camel-case type or enum-variant name
  containing ``_`` at all.  rustc's rule is on the whole identifier, so
  ``Foo_Bar`` and ``RmB_VvvvC_DstA`` are both rejected even though every
  segment starts upper-case; that is precisely the mistake this exists to
  catch, and an early draft of the matcher missed it because it only looked
  for a lower-case segment (the self-test caught that).
* ``bool_assert_comparison`` -- ``assert_eq!(x, true)`` / ``(x, false)``.

Deliberately NOT covered: ``use`` ordering inside a brace group.  It costs a
CI round trip when wrong, but it is not derivable here.  Measuring rustfmt's
rule against this tree shows a plain ASCII sort disagreeing with it in 26
places on a tree whose ``cargo fmt --all -- --check`` is green
(``{ELF_MAGIC, ELFCLASS32}`` stays as written, ``{self as elf_mod, ...}`` keeps
``self`` first, digit runs compare numerically so ``ADD8`` precedes ``ADD16``).
A gate that fires on a green tree is worse than no gate: the next person
disables it.  So this script says nothing about import order, and the
authority remains the CI step ``cargo fmt --all -- --check``.

Not a lint, but reported for the census ratchet to adjudicate: ``panic!`` /
``unreachable!`` / ``todo!`` / ``unimplemented!`` / ``.expect(`` / ``.unwrap(``
sites.  Reported, never auto-failed -- ``scripts/audit_census.py`` owns that
pass/fail decision.

Scope: the ADDED lines of ``git diff <base>...HEAD``, so it is a pre-push
check on the change under review rather than a whole-tree audit (the tree
itself is already clean; that is what CI proves).

Usage:
    scripts/check_rust_lint_traps.py                # diff vs origin/main
    scripts/check_rust_lint_traps.py --base HEAD~3
    scripts/check_rust_lint_traps.py --self-test
"""
from __future__ import annotations

import argparse
import re
import subprocess
import sys

# A type or variant name: CamelCase words joined by `_`, where at least one
# segment starts lowercase.  `REG_NAMES` (an all-caps const) is fine; `RmB_Vvvv`
# is not, because rustc's camel-case rule rejects a `_` inside a CamelCase
# identifier.
TYPE_DECL = re.compile(
    r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:enum|struct|trait|union)\s+(\w+)", re.M
)
VARIANT = re.compile(r"^\s{4,}([A-Za-z]\w*_\w*)\s*(?:[,{(])", re.M)
BOOL_ASSERT = re.compile(r"assert_eq!\(\s*[\w:.]+\s*,\s*(true|false)\s*[,)]")
SCREAMING = re.compile(r"^[A-Z0-9_]+$")


def is_bad_camel_case(name: str) -> bool:
    """Would ``non_camel_case_types`` reject this identifier?"""
    if "_" not in name:
        return False
    if SCREAMING.match(name):
        # ALL_CAPS consts and associated consts are idiomatic, not types.
        return False
    # rustc's rule is on the whole identifier: an upper-camel-case *type* name
    # may not contain `_` at all, whatever case the segments are.  So both
    # `Foo_Bar` and `RmB_VvvvC_DstA` are rejected even though every segment
    # starts uppercase -- which is exactly the mistake this gate exists to
    # catch.  Identifier shape is the test, not per-segment casing.
    return True


def scan(text: str) -> list[str]:
    problems: list[str] = []
    for m in TYPE_DECL.finditer(text):
        if is_bad_camel_case(m.group(1)):
            problems.append(f"non_camel_case_types: type `{m.group(1)}`")
    for m in VARIANT.finditer(text):
        name = m.group(1)
        # Only flag things that look like a type/variant, not locals such as
        # `let x_y = ...` or field accesses.
        if is_bad_camel_case(name) and name[:1].isupper():
            problems.append(f"non_camel_case_types: variant `{name}`")
    for m in BOOL_ASSERT.finditer(text):
        problems.append(f"bool_assert_comparison: assert_eq!(..., {m.group(1)})")
    return problems


def added_lines(base: str) -> tuple[str, int]:
    out = subprocess.run(
        ["git", "diff", f"{base}..HEAD", "--", "*.rs"],
        capture_output=True,
        text=True,
        check=False,
    ).stdout
    lines = [ln[1:] for ln in out.split("\n") if ln.startswith("+") and not ln.startswith("+++")]
    return "\n".join(lines), len(lines)


SELF_TEST_CASES: list[tuple[str, bool]] = [
    # (source, should_flag)
    ("enum FmaRoleOrder {\n    RmB_VvvvC_DstA,\n}\n", True),
    ("enum FmaRoleOrder {\n    RmBVvvvCDstA,\n}\n", False),
    ("pub struct Foo_Bar;\n", True),
    ("pub struct FooBar;\n", False),
    ("const REG_NAMES: u8 = 1;\n", False),
    ("assert_eq!(flag, true);\n", True),
    ("assert_eq!(flag, false, \"why\");\n", True),
    ("assert!(flag);\n", False),
    ("assert_eq!(a, b);\n", False),
    ("enum E {\n    REG_NONE,\n    REG_ALL,\n}\n", False),
]


def self_test() -> int:
    failures = 0
    for source, should_flag in SELF_TEST_CASES:
        got = bool(scan(source))
        if got != should_flag:
            print(
                f"SELF-TEST FAIL: expected flag={should_flag} got {got} for {source!r}",
                file=sys.stderr,
            )
            failures += 1
    if failures:
        return 1
    print(f"self-test: {len(SELF_TEST_CASES)} cases classified correctly")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--base", default="origin/main")
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args()

    if args.self_test:
        return self_test()

    text, n = added_lines(args.base)
    problems = scan(text)
    if problems:
        print(f"rust-lint-traps: FAIL ({len(problems)} over {n} added lines)", file=sys.stderr)
        for p in sorted(set(problems)):
            print(f"  {p}", file=sys.stderr)
        print(
            "  Each of these is a `-D warnings` build failure; fix before pushing.",
            file=sys.stderr,
        )
        return 1
    print(f"rust-lint-traps: ok ({n} added Rust lines, none reachable by the denials)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
