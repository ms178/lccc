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

# An arm pattern is one or more `IntrinsicOp::X` alternatives -- one per line
# (`| IntrinsicOp::X`) or several on one line -- and the pattern's last line
# opens the body with `=> {`.  Patterns are located STRUCTURALLY (brace
# matching), not by a fixed window: the mnemonic table can sit several blocks
# deep inside the arm body, and a window that is too small silently skips an
# arm, which is the failure mode this checker exists to prevent.
_PATTERN_ONLY = re.compile(
    r"^\s*\|?\s*IntrinsicOp::[A-Za-z0-9_]+(?:\s*\|\s*IntrinsicOp::[A-Za-z0-9_]+)*\s*\|?\s*$"
)
_PATTERN_HEAD = re.compile(
    r"^\s*(?:\|\s*)?IntrinsicOp::[A-Za-z0-9_]+"
    r"(?:\s*\|\s*IntrinsicOp::[A-Za-z0-9_]+)*\s*=>\s*\{\s*$"
)
_ARM_END = re.compile(r"^\s*=>\s*\{\s*$")
_INNER_OP = re.compile(r"IntrinsicOp::([A-Za-z0-9_]+)")
_INNER_ARM = re.compile(r"^(?P<pattern>[^\"]*?)=>\s*(?P<body>.*)$")


@dataclass
class TableArm:
    """One `IntrinsicOp` group arm with its outer pattern and inner mnemonic map."""

    path: Path
    line: int  # 1-based line of the outer pattern's first op
    outer: list[str]
    inner: dict[str, str]
    wildcard: str | None = None


def _code_lines(text: str) -> list[str]:
    """(`text` with comments and string literals blanked, same line count).

    Brace counting must not see the `{`/`}` inside `format!("... {:?}")` or a
    comment, and pattern matching must not see an op name quoted in prose.
    """
    text = re.sub(r"/\*.*?\*/", lambda m: " " * len(m.group(0)), text, flags=re.S)
    out = []
    for line in text.splitlines():
        line = re.sub(r"//.*$", "", line)
        line = re.sub(r'"(?:[^"\\]|\\.)*"', '""', line)
        out.append(line)
    return out


def _indent(line: str) -> int:
    return len(line) - len(line.lstrip(" "))


def _arm_head_indices(code: list[str]) -> list[int]:
    return [
        index
        for index, line in enumerate(code)
        if _PATTERN_HEAD.match(line) or _ARM_END.match(line)
    ]


def _body_end(code: list[str], head: int) -> int:
    """Last line index of the arm body opened on `head`."""
    depth = 0
    for index in range(head, len(code)):
        depth += code[index].count("{") - code[index].count("}")
        if depth <= 0 and index > head:
            return index
    return len(code) - 1


def _collect_pattern(lines: list[str], head: int) -> tuple[list[str], int]:
    """All ops of the pattern ending at `head`, plus its 1-based line number."""
    names = _INNER_OP.findall(lines[head])
    if not names:
        return [], 0
    column = _indent(lines[head])
    cursor = head - 1
    while cursor >= 0:
        line = lines[cursor]
        if _indent(line) != column or not _PATTERN_ONLY.match(line):
            break
        names = _INNER_OP.findall(line) + names
        cursor -= 1
    return names, cursor + 2  # 1-based line of the pattern's first op


def _collect_inner(lines: list[str], anchor: int) -> tuple[dict[str, str], str | None]:
    """Map each op named by the inner match to its mnemonic literal.

    A wildcard arm that yields a literal (`_ => "vmovmskpd"`) covers every op
    the arm pattern admits, so it is reported separately; a wildcard arm that
    panics (`_ => unreachable!("..")`) covers nothing.
    """
    inner: dict[str, str] = {}
    wildcard: str | None = None
    for inner_line in lines[anchor:]:
        stripped = inner_line.strip()
        if stripped == "};":
            break
        arm = _INNER_ARM.match(stripped)
        if not arm:
            continue
        body = arm.group("body")
        quote = -1
        if not body.lstrip().startswith("unreachable!"):
            quote = body.find('"')
        if quote < 0:
            continue
        end_quote = body.find('"', quote + 1)
        if end_quote < 0:
            continue
        mnemonic = body[quote + 1 : end_quote]
        names = _INNER_OP.findall(arm.group("pattern"))
        if not names:
            wildcard = mnemonic
            continue
        for name in names:
            inner.setdefault(name, mnemonic)
    return inner, wildcard


_FN_START = re.compile(r"^\s*(?:pub(?:\([a-z]+\))?\s+)?fn\s+[A-Za-z0-9_]+")


def _function_spans(code: list[str]) -> list[tuple[int, int]]:
    """(start, end) line index pairs for every function body."""
    spans = []
    for index, line in enumerate(code):
        if not _FN_START.match(line):
            continue
        end = _body_end(code, index)
        spans.append((index, end))
    return spans


def parse_tables(text: str, path: Path) -> tuple[list[TableArm], int, int]:
    """Parse every lowering table; also return the number of anchors seen.

    An anchor with no enclosing arm is invisible to this check, so the caller
    compares the two counts and fails when they disagree.
    """
    lines = text.splitlines()
    code = _code_lines(text)
    heads = _arm_head_indices(code)
    spans = [(head, _body_end(code, head)) for head in heads]
    functions = _function_spans(code)
    anchors = [i for i, line in enumerate(code) if INNER_ANCHOR in line]
    arms: list[TableArm] = []
    in_scope = 0
    out_of_scope = 0
    for anchor in anchors:
        enclosing_fn = next(
            ((start, end) for start, end in functions if start < anchor < end), None
        )
        if enclosing_fn is not None and not any(
            enclosing_fn[0] < start < enclosing_fn[1] for start, _ in spans
        ):
            # A mnemonic table in a helper function: it has no arm pattern to
            # compare against, so width/dispatch coverage is checked elsewhere.
            out_of_scope += 1
            continue
        in_scope += 1
        enclosing = [(start, end) for start, end in spans if start < anchor < end]
        if not enclosing:
            continue
        head, _ = max(enclosing, key=lambda span: span[0])
        pattern, first_line = _collect_pattern(lines, head)
        if not pattern:
            continue
        inner, wildcard = _collect_inner(lines, anchor)
        arms.append(
            TableArm(
                path=path,
                line=first_line,
                outer=pattern,
                inner=inner,
                wildcard=wildcard,
            )
        )
    return arms, in_scope, out_of_scope


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
        if arm.wildcard is not None:
            continue  # a literal-yielding wildcard arm covers every remaining op
        problems.append(
            f"{arm.path}:{arm.line}: IntrinsicOp::{name} is matched by the outer "
            "arm pattern but has no mnemonic in the inner match; reaching it "
            "panics with unwrap on `unreachable!()`"
        )
    for name in sorted(outer & inner):
        if not arm.inner[name]:
            problems.append(f"{arm.path}:{arm.line}: IntrinsicOp::{name} mnemonic is empty")
    return problems


def run(paths: list[Path]) -> tuple[int, int, list[str]]:
    findings: list[str] = []
    arms_checked = 0
    helper_anchors = 0
    for path in paths:
        if not path.exists():
            findings.append(f"{path}: file is missing")
            continue
        text = path.read_text()
        arms, anchors, helpers = parse_tables(text, path)
        if not arms:
            findings.append(f"{path}: no lowering tables found (parser out of date?)")
            return 0, 0, findings
        # Every inner mnemonic table must be attributed to an arm.  A skipped
        # anchor is an arm this check does not look at, i.e. silence that reads
        # as approval, so an unparsed anchor is a failure rather than a skip.
        if len(arms) != anchors:
            findings.append(
                f"{path}: parsed {len(arms)} lowering tables but found {anchors} "
                f"'{INNER_ANCHOR}' anchors; an arm this check cannot read is an "
                "arm it cannot defend"
            )
        for arm in arms:
            arms_checked += 1
            findings.extend(check_arm(arm))
        helper_anchors += helpers
    return arms_checked, helper_anchors, findings


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
    # A pattern written on ONE line (`A | B | C => {`) is a table too: the
    # mnemonics are per-op even though the pattern does not use `|` lines.
    "single_line_pattern": (
        """            IntrinsicOp::Pabsb128 | IntrinsicOp::Pabsw128 => {
                if let Some(dptr) = result_dest(dest, dest_ptr) {
                    let inst = match op {
                        IntrinsicOp::Pabsb128 => "pabsb",
                        IntrinsicOp::Pabsw128 => "pabsw",
                        IntrinsicOp::Pabsd128 => "pabsd",
                        _ => unreachable!("unexpected abs op: {:?}", op),
                    };
                    self.emit_sse_unary_128(dptr, args, inst);
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
        arms, _anchors, _helpers = parse_tables(snippet, path)
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
    arms_checked, helper_anchors, findings = run(paths)
    if findings:
        print("intrinsic lowering coverage: FAIL")
        for finding in findings:
            print(f"  {finding}")
        return 1
    print(
        f"intrinsic lowering coverage: OK ({arms_checked} table arms consistent, "
        f"{helper_anchors} mnemonic table(s) in helper functions out of scope)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
