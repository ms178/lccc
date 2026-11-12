#!/usr/bin/env python3
"""Cross-check `x86_intrinsic_kind` against `IntrinsicOp::vector_home_width`.

A GCC x86 builtin reaches the backend through three independent tables:

  * `BuiltinIntrinsic` -> `IntrinsicOp`  (`builtin_intrinsic_op`)
  * `BuiltinIntrinsic`   -> `X86IntrinsicKind`  (`x86_intrinsic_kind`)
  * `IntrinsicOp`        -> SIMD home width  (`vector_home_width`)

The IR lowering allocates the builtin's result temporary from the SECOND table
(`Vec128` -> a 16-byte alloca, `Vec256` -> 32 bytes), while the backend writes
it according to the THIRD.  Nothing forces the two to agree, and the tables are
edited by different people for different reasons, so a 256-bit op that is
missing from the `Vec256` arm silently falls into the `_ => Vec128` catch-all:

    Alloca { size: 16, align: 16 }          ; the temp the lowering built
    Intrinsic { op: Pminsb256, dest_ptr: .. } ; `vmovdqu %ymm0, (%ptr)`: 32 bytes
    Memcpy { dest: <vector home>, src: <temp>, size: 32 }

The 32-byte store then runs past its slot into whatever the stack layout placed
next -- silent data corruption that appears as "the upper 128-bit lane is
garbage" in a program that never mentions intrinsics in the failing line.  This
check turns that into a build-time failure.

Scope: only builtins whose mapping and kind are both table rows (a builtin
listed as `Scalar`/`PtrStore`/... is deliberately not a vector result, and a
builtin with no `IntrinsicOp` row is not emitted through this path at all).

Usage:
    python3 scripts/check_x86_intrinsic_kind_width.py [--self-test] [--root DIR]

Exit codes: 0 = tables agree, 1 = mismatch found (or self-test failure).
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]

LOWERING = "src/ir/lowering/expr_builtins.rs"
INTRINSICS = "src/ir/intrinsics.rs"

_OP_ROW = re.compile(r"BuiltinIntrinsic::([A-Za-z0-9_]+)\s*=>\s*IntrinsicOp::([A-Za-z0-9_]+)")
_KIND_ROW = re.compile(r"BuiltinIntrinsic::([A-Za-z0-9_]+)")
_KIND_ARM_KIND = re.compile(r"\s*X86IntrinsicKind::([A-Za-z0-9_]+)")

# Width of an op as `vector_home_width` sees it, keyed by the section the op is
# listed in.  The function is split by these two markers.
_SEC_256 = "// ---- 256-bit AVX/AVX2 results ----"
_SEC_128 = "// ---- 128-bit SSE/SSE2/SSSE3/SSE4 results ----"
_SEC_END = "// ---- Non-vector"
_OP_NAME = re.compile(r"\b([A-Z][A-Za-z0-9_]*)\b")


def parse_op_widths(text: str) -> dict[str, int]:
    """Map IntrinsicOp variant -> SIMD home width (32 or 16)."""
    start = text.index("pub fn vector_home_width")
    section = text[start:]
    i256 = section.index(_SEC_256)
    i128 = section.index(_SEC_128)
    end = section.find(_SEC_END, i128)
    tail = section[i128:end if end > 0 else len(section)]
    widths: dict[str, int] = {}
    for part, width in ((section[i256:i128], 32), (tail, 16)):
        for line in part.splitlines():
            code = line.split("//", 1)[0]
            for name in _OP_NAME.findall(code):
                if name in {"Some", "Self", "None", "IntrinsicOp", "use", "match"}:
                    continue
                widths.setdefault(name, width)
    return widths


def _strip_comments(text: str) -> str:
    """Drop `//` and `/* */` comments so a table row cannot hide in one."""
    text = re.sub(r"/\*.*?\*/", " ", text, flags=re.S)
    return re.sub(r"//[^\n]*", "", text)


def parse_kinds(text: str) -> dict[str, str]:
    """Map BuiltinIntrinsic variant -> X86IntrinsicKind from `x86_intrinsic_kind`.

    Parsed by splitting the body on `=>` rather than by matching whole arms:
    the arm patterns carry comments (some of them quoting code), so an
    arm-shaped regex is fragile in exactly the direction that matters -- it
    would silently drop rows and turn this check into a no-op.
    """
    start = text.index("fn x86_intrinsic_kind(")
    brace = text.index("{", start)
    depth = 0
    end = len(text)
    for i in range(brace, len(text)):
        if text[i] == "{":
            depth += 1
        elif text[i] == "}":
            depth -= 1
            if depth == 0:
                end = i
                break
    body = _strip_comments(text[start:end])

    kinds: dict[str, str] = {}
    segments = body.split("=>")
    for index in range(1, len(segments)):
        pattern = segments[index - 1]
        # The pattern is everything after the previous arm's `X86IntrinsicKind::X,`
        # (or the `match intrinsic {` opener of the first arm).
        if "X86IntrinsicKind::" in pattern:
            pattern = pattern.split("X86IntrinsicKind::", 1)[1]
        kind_match = _KIND_ARM_KIND.search(segments[index])
        if not kind_match:
            continue
        kind = kind_match.group(1)
        for name in _KIND_ROW.findall(pattern):
            kinds[name] = kind
    return kinds


def check(text_lowering: str, text_intrinsics: str) -> tuple[list[str], int]:
    """Return (findings, number of builtins compared)."""
    mappings = dict(_OP_ROW.findall(text_lowering))
    kinds = parse_kinds(text_lowering)
    widths = parse_op_widths(text_intrinsics)

    findings: list[str] = []
    compared = 0
    for builtin, op in sorted(mappings.items()):
        kind = kinds.get(builtin)
        if kind is None:
            # Not classified: falls through to the `_ => Vec128` catch-all.
            kind = "Vec128"
        elif kind not in {"Vec128", "Vec256"}:
            continue
        width = widths.get(op)
        if width is None:
            continue
        compared += 1
        want = "Vec256" if width == 32 else "Vec128"
        if kind != want:
            findings.append(
                f"{builtin} -> IntrinsicOp::{op} writes {width} bytes but "
                f"x86_intrinsic_kind classifies it {kind}; the lowering allocates "
                f"a {'16' if kind == 'Vec128' else '32'}-byte result temp and the "
                f"backend stores {width} bytes into it"
            )
    return findings, compared


SELF_TEST = {
    "consistent": (
        """
fn x86_intrinsic_kind(intrinsic: &BuiltinIntrinsic) -> X86IntrinsicKind {
    match intrinsic {
        BuiltinIntrinsic::X86Pminsb256 => X86IntrinsicKind::Vec256,
        _ => X86IntrinsicKind::Vec128,
    }
}
""",
        """
    pub fn vector_home_width(&self) -> Option<u32> {
        match self {
            // ---- 256-bit AVX/AVX2 results ----
            Pminsb256,
            // ---- 128-bit SSE/SSE2/SSSE3/SSE4 results ----
            Paddb128,
            // ---- Non-vector
        }
    }
""",
        0,
    ),
    "missing_vec256_row": (
        """
fn x86_intrinsic_kind(intrinsic: &BuiltinIntrinsic) -> X86IntrinsicKind {
    match intrinsic {
        BuiltinIntrinsic::X86Pminsb128 => X86IntrinsicKind::Vec128,
        _ => X86IntrinsicKind::Vec128,
    }
}
""",
        """
    pub fn vector_home_width(&self) -> Option<u32> {
        match self {
            // ---- 256-bit AVX/AVX2 results ----
            Pminsb256,
            // ---- 128-bit SSE/SSE2/SSSE3/SSE4 results ----
            Paddb128,
            // ---- Non-vector
        }
    }
""",
        1,
    ),
}


def self_test() -> int:
    failures = 0
    for name, (lowering, intrinsics, expected) in SELF_TEST.items():
        # The mapping table lives in the same file as the kind table in the
        # real tree; the fixture supplies it inline.
        fixture_lowering = lowering + (
            "\nfn builtin_intrinsic_op(b: BuiltinIntrinsic) -> IntrinsicOp {\n"
            "    match b {\n"
            "        BuiltinIntrinsic::X86Pminsb256 => IntrinsicOp::Pminsb256,\n"
            "        BuiltinIntrinsic::X86Pminsb128 => IntrinsicOp::Pminsb128,\n"
            "        _ => IntrinsicOp::Pminsb128,\n"
            "    }\n}\n"
        )
        findings, _ = check(fixture_lowering, intrinsics)
        if len(findings) != expected:
            print(f"SELFTEST FAIL {name}: {len(findings)} finding(s), expected {expected}")
            for finding in findings:
                print(f"  {finding}")
            failures += 1
        else:
            print(f"SELFTEST ok {name}")
    return 1 if failures else 0


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--root", default=str(REPO_ROOT))
    args = parser.parse_args(argv)

    if args.self_test:
        return self_test()

    root = Path(args.root)
    try:
        lowering = (root / LOWERING).read_text()
        intrinsics = (root / INTRINSICS).read_text()
    except OSError as error:
        print(f"x86 intrinsic kind/width: FAIL: {error}")
        return 1

    findings, compared = check(lowering, intrinsics)
    if findings:
        print("x86 intrinsic kind/width: FAIL")
        for finding in findings:
            print(f"  {finding}")
        return 1
    print(f"x86 intrinsic kind/width: OK ({compared} builtins agree)")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
