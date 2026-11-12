#!/usr/bin/env python3
"""Static coverage check for the x86 / i686 intrinsic emitter instruction tables.

Both backends lower the pointer-form ("dest_ptr") SIMD intrinsics through a
table of match arms shaped like::

    IntrinsicOp::AddU8x16
    | IntrinsicOp::SubU8x16
    | ...                           <- outer pattern: the ops the arm handles
    | IntrinsicOp::Punpckhqdq128 => {
        if let Some(dptr) = result_dest(dest, dest_ptr) {
            let inst = match op {
                IntrinsicOp::AddU8x16 => "paddb",
                ...
                _ => unreachable!(),
            };                      <- inner match: the exact mnemonic per op
            self.emit_sse_binary_128(dptr, args, inst);
        }
    }

The outer pattern decides *whether* an op reaches the arm; the inner match
decides *which instruction* it becomes.  When the two drift apart the failure
mode is silent and late: an op listed only in the inner match never matches the
arm, falls through to the catch-all at the end of `emit_intrinsic`, and the
compiler ICEs at run time with

    unhandled intrinsic op Punpckhqdq128 (dest_ptr=true, args=2)

exactly the bug this check exists to prevent (a refactor that re-emitted the
outer list without one of its members shipped that panic to CI).  An op listed
only in the outer pattern instead hits `unreachable!()`.

This check therefore enforces, for every such arm in both backends:

  * every op in the inner mnemonic match is listed in the outer pattern
    (otherwise: reachable ICE / missing lowering), and
  * every op in the outer pattern has a mnemonic in the inner match
    (otherwise: `unreachable!()` panic on a user-reachable path).

Usage:
    python3 scripts/check_intrinsic_lowering_coverage.py [--self-test]

Exit codes: 0 = all arms consistent, 1 = drift found (or self-test failure).
"""

from __future__ import annotations

import argparse
import re
import sys
from dataclasses import dataclass
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]

FILES = (
    "src/backend/x86/codegen/intrinsics.rs",
    "src/backend/i686/codegen/intrinsics.rs",
)

INNER_ANCHOR = "let inst = match op {"

# Outer pattern lines look like `IntrinsicOp::Foo` or `| IntrinsicOp::Foo`.
# Keep this strictly anchored: anything else (e.g. a comment or a closing
# brace) terminates the pattern list.
# A pattern line is `IntrinsicOp::Foo`, optionally prefixed with `|` and
# optionally carrying a trailing `|`; the last line of the pattern also opens
# the arm body with `=> {` (there is no separate `=> {` line in this codebase).
_PATTERN_LINE = re.compile(r"^\s*\|?\s*IntrinsicOp::([A-Za-z0-9_]+)\s*\|?\s*(=>\s*\{\s*)?$")
_ARM_END = re.compile(r"^\s*=>\s*\{\s*$")
# Inner-match arms may name several ops per mnemonic
# (`IntrinsicOp::Dpbusd128 | IntrinsicOp::Dpbusd256 => "vpdpbusd",`) and the
# mnemonic may carry assembler prefixes (`"{vex} vpdpbusd"`).
_INNER_OP = re.compile(r"IntrinsicOp::([A-Za-z0-9_]+)")
_INNER_ARM = re.compile(r"^(?P<pattern>[^\"]*?)=>\s*(?P<body>.*)$")


@dataclass
class TableArm:
    """One `IntrinsicOp` group arm with its outer pattern and inner mnemonic map."""

    path: Path
    line: int  # 1-based line of the outer pattern's first op
    outer: list[str]
    inner: dict[str, str]


def parse_tables(text: str, path: Path) -> list[TableArm]:
    lines = text.splitlines()
    arms: list[TableArm] = []
    for idx, line in enumerate(lines):
        if INNER_ANCHOR not in line:
            continue
        head = _find_arm_head(lines, idx)
        if head is None:
            continue
        pattern, first_line = _collect_pattern(lines, head)
        if not pattern:
            continue
        inner = _collect_inner(lines, idx)
        arms.append(TableArm(path=path, line=first_line, outer=pattern, inner=inner))
    return arms


# The arm body prologue between the pattern and the inner match is at most a
# couple of lines (`=> {`, `if let Some(dptr) = ...`); a generous bound keeps
# the backward scan from wandering into the previous arm.
_MAX_PROLOGUE = 8


def _find_arm_head(lines: list[str], anchor: int) -> int | None:
    """Index of the line that opens the arm enclosing `lines[anchor]`."""
    for cursor in range(anchor - 1, max(anchor - _MAX_PROLOGUE, 0) - 1, -1):
        candidate = lines[cursor]
        match = _PATTERN_LINE.match(candidate)
        if match and match.group(2):
            return cursor  # `IntrinsicOp::X => {` opens the body on the same line
        if _ARM_END.match(candidate):
            return cursor  # bare `=> {` line
    return None


def _collect_pattern(lines: list[str], head: int) -> tuple[list[str], int]:
    """All ops of the pattern ending at `head`, plus its 1-based line number."""
    match = _PATTERN_LINE.match(lines[head])
    if not match:
        return [], 0
    pattern = [match.group(1)]
    cursor = head - 1
    while cursor >= 0:
        match = _PATTERN_LINE.match(lines[cursor])
        if not match or match.group(2):
            break
        pattern.append(match.group(1))
        cursor -= 1
    pattern.reverse()
    return pattern, cursor + 2  # 1-based line of the pattern's first op


def _collect_inner(lines: list[str], anchor: int) -> dict[str, str]:
    """Map each op named by the inner `match op` to its mnemonic literal."""
    inner: dict[str, str] = {}
    for inner_line in lines[anchor:]:
        stripped = inner_line.strip()
        if stripped == "};":
            break
        arm = _INNER_ARM.match(stripped)
        if not arm:
            continue
        names = _INNER_OP.findall(arm.group("pattern"))
        if not names:
            continue  # wildcard arm (`_ => unreachable!(..)`)
        body = arm.group("body")
        quote = body.find('"')
        if quote < 0:
            continue
        end_quote = body.find('"', quote + 1)
        if end_quote < 0:
            continue
        mnemonic = body[quote + 1 : end_quote]
        for name in names:
            inner.setdefault(name, mnemonic)
    return inner


def check_arm(arm: TableArm) -> list[str]:
    """Return human-readable drift findings for one table arm."""
    problems: list[str] = []
    outer = set(arm.outer)
    inner = set(arm.inner)
    for name in sorted(inner - outer):
        problems.append(
            f"{arm.path}:{arm.line}: IntrinsicOp::{name} has an inner mnemonic "
            f'("{arm.inner[name]}") but is missing from the outer arm pattern; '
            "the op falls through to the catch-all and ICEs at run time"
        )
    for name in sorted(outer - inner):
        problems.append(
            f"{arm.path}:{arm.line}: IntrinsicOp::{name} is matched by the outer "
            "arm pattern but has no mnemonic in the inner match; reaching it "
            "panics with unwrap on `unreachable!()`"
        )
    for name in sorted(outer & inner):
        if not arm.inner[name]:
            problems.append(f"{arm.path}:{arm.line}: IntrinsicOp::{name} mnemonic is empty")
    return problems


def run(paths: list[Path]) -> tuple[int, list[str]]:
    findings: list[str] = []
    arms_checked = 0
    for path in paths:
        if not path.exists():
            findings.append(f"{path}: file is missing")
            continue
        arms = parse_tables(path.read_text(), path)
        if not arms:
            findings.append(f"{path}: no lowering tables found (parser out of date?)")
            return 1, findings
        for arm in arms:
            arms_checked += 1
            findings.extend(check_arm(arm))
    return arms_checked, findings


SELF_TEST_CASES = {
    "consistent": (
        """            IntrinsicOp::Punpcklqdq128
            | IntrinsicOp::Punpckhqdq128 => {
                if let Some(dptr) = result_dest(dest, dest_ptr) {
                    let inst = match op {
                        IntrinsicOp::Punpcklqdq128 => "punpcklqdq",
                        IntrinsicOp::Punpckhqdq128 => "punpckhqdq",
                        _ => unreachable!(),
                    };
                    self.emit_sse_binary_128(dptr, args, inst);
                }
            }
""",
        0,
    ),
    # The shipped defect: one op was dropped from the outer pattern while
    # keeping its inner mnemonic.
    "outer_dropped_one": (
        """            IntrinsicOp::Punpcklqdq128 => {
                if let Some(dptr) = result_dest(dest, dest_ptr) {
                    let inst = match op {
                        IntrinsicOp::Punpcklqdq128 => "punpcklqdq",
                        IntrinsicOp::Punpckhqdq128 => "punpckhqdq",
                        _ => unreachable!(),
                    };
                    self.emit_sse_binary_128(dptr, args, inst);
                }
            }
""",
        1,
    ),
    # The mirror image: an op admitted by the arm but with no mnemonic.
    "inner_dropped_one": (
        """            IntrinsicOp::Punpcklqdq128
            | IntrinsicOp::Punpckhqdq128 => {
                if let Some(dptr) = result_dest(dest, dest_ptr) {
                    let inst = match op {
                        IntrinsicOp::Punpcklqdq128 => "punpcklqdq",
                        _ => unreachable!(),
                    };
                    self.emit_sse_binary_128(dptr, args, inst);
                }
            }
""",
        1,
    ),
    # Arms that group two ops onto one mnemonic must still be seen as covered.
    "grouped_ops": (
        """            IntrinsicOp::Dpbusd128
            | IntrinsicOp::Dpbusd256 => {
                if let Some(dptr) = dest_ptr {
                    let inst = match op {
                        IntrinsicOp::Dpbusd128 | IntrinsicOp::Dpbusd256 => "{vex} vpdpbusd",
                        _ => unreachable!("unexpected VNNI op: {:?}", op),
                    };
                    self.emit_sse_binary_128(dptr, "%v0", inst);
                }
            }
""",
        0,
    ),
}


def self_test() -> int:
    failures = 0
    for name, (snippet, expect_problems) in SELF_TEST_CASES.items():
        path = Path(f"<self-test:{name}>")
        arms = parse_tables(snippet, path)
        problems: list[str] = []
        for arm in arms:
            problems.extend(check_arm(arm))
        if len(arms) != 1:
            print(f"SELFTEST FAIL {name}: parsed {len(arms)} arms, expected 1")
            failures += 1
            continue
        if expect_problems and not problems:
            print(f"SELFTEST FAIL {name}: drift not detected")
            failures += 1
        elif not expect_problems and problems:
            print(f"SELFTEST FAIL {name}: false positive {problems}")
            failures += 1
        else:
            print(f"SELFTEST ok {name}")
    return 1 if failures else 0


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true", help="verify the checker itself")
    parser.add_argument(
        "--root",
        default=str(REPO_ROOT),
        help="repository root (defaults to this script's parent)",
    )
    args = parser.parse_args(argv)

    if args.self_test:
        return self_test()

    root = Path(args.root)
    paths = [root / rel for rel in FILES]
    arms_checked, findings = run(paths)
    if findings:
        print("intrinsic lowering coverage: FAIL")
        for finding in findings:
            print(f"  {finding}")
        return 1
    print(f"intrinsic lowering coverage: OK ({arms_checked} table arms consistent)")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
