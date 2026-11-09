#!/usr/bin/env python3
"""Ratchet: the three x86 SIMD width layers must agree on commutativity.

WHY THIS GATE EXISTS
--------------------
`commutative` decides whether the shared binary emitter may fold the LEFT
operand into the instruction's memory slot:

    (Some(m0), _) if commutative => load args[1] to reg; `op m0, reg, dst`
    (Some(m0), None)             => load m0 to reg; load args[1]; `op ...`

Folding the left operand SWAPS which IR argument lands in the register
(EVEX.vvvv / VEX.vvvv) and which lands in r/m.  For a symmetric instruction
that is free performance.  For an asymmetric one it is a silent miscompile.

The decision is made independently in THREE places:

  * `sse128_mnemonic_commutative`      (intrinsics.rs)       -- 128-bit allowlist
  * `emit_avx_binary_256(.., FLAG)`    (intrinsics.rs)       -- 256-bit per-arm
  * `emit_evex_binary_512(.., FLAG)`   (intrinsics_simd.rs)  -- 512-bit per-arm

Nothing forced them to agree, and they did not.  Two real defects came out of
the first run of this gate (SIMD audit SA-05 and SA-36):

  * `vpmaddubsw` was `true` at 512 but `false` at 256 and absent at 128.
    VPMADDUBSW reads vvvv as UNSIGNED bytes and r/m as SIGNED bytes, so the
    512-bit arm computed u(args[1]) * s(args[0]) whenever args[0] was foldable
    -- the ubiquitous `_mm512_maddubs_epi16(_mm512_loadu_si512(p), w)` shape.
    Every lane product with a high bit set in either byte was wrong.  (SA-05)
  * `vpmaddwd` was `false` at 256 and absent at 128 but `true` at 512, with a
    comment claiming the maddubs operand hazard.  PMADDWD multiplies two
    SIGNED word streams, so it is symmetric and the conservative flag only
    cost a memory fold.  (SA-36)

A gate that only checked agreement would have accepted a table that is
uniformly wrong, so this one carries an explicit semantic authority as well.

WHAT IS CHECKED
---------------
1. ASYMMETRIC mnemonics are never marked commutative at ANY width.  This is
   the correctness half and the reason the gate is not just a diff.
2. SYMMETRIC mnemonics are marked commutative at EVERY width where they
   appear.  This is the missed-optimization half; it is what makes the
   pmaddwd fix permanent instead of a one-off.
3. No base mnemonic carries both `true` and `false` across widths.  Redundant
   given (1) and (2) for the listed names, but it covers every OTHER mnemonic
   automatically, so a newly added instruction cannot drift unnoticed.

Names are compared with the optional VEX/EVEX `v` prefix stripped, because
`pmaddwd`/`vpmaddwd` name the same operation.  The `q`-suffixed EVEX.512
spellings (`pandq`, `porq`, `pxorq`) are normalized to their base too.

Exit status: 0 = PASS, 1 = violations found, 2 = the source could not be
parsed (fail-closed: a gate that cannot read its input must not report PASS).
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

# ---------------------------------------------------------------------------
# Semantic authority.  Each entry is an operation whose two sources play
# DIFFERENT roles, so exchanging them changes the result.  Source: Intel SDM
# Vol. 2 per-instruction operation pseudocode; the GCC 16.2 header split and
# the encoder's operand order agree.
# ---------------------------------------------------------------------------
ASYMMETRIC = {
    # vvvv bytes UNSIGNED, r/m bytes SIGNED -- the SA-05 defect.
    "pmaddubsw": "src1 (vvvv) bytes are unsigned, src2 (r/m) bytes are signed",
    # a & ~b
    "pandn": "computes src1 AND NOT src2",
    # subtraction and its saturating forms
    "psubb": "subtraction is not symmetric",
    "psubw": "subtraction is not symmetric",
    "psubd": "subtraction is not symmetric",
    "psubq": "subtraction is not symmetric",
    "psubsb": "signed saturating subtraction is not symmetric",
    "psubsw": "signed saturating subtraction is not symmetric",
    "psubusb": "unsigned saturating subtraction is not symmetric",
    "psubusw": "unsigned saturating subtraction is not symmetric",
    # data vs shuffle-control / shift-count
    "pshufb": "src2 supplies the shuffle control, src1 the data",
    "palignr": "concatenation order of the two sources is significant",
    "psllw": "src2 is the shift count, src1 the data",
    "psllq": "src2 is the shift count, src1 the data",
    "pslld": "src2 is the shift count, src1 the data",
    "psrlw": "src2 is the shift count, src1 the data",
    "psrld": "src2 is the shift count, src1 the data",
    "psrlq": "src2 is the shift count, src1 the data",
    "psraw": "src2 is the shift count, src1 the data",
    "psrad": "src2 is the shift count, src1 the data",
    "psllvq": "per-lane variable shift: data vs count",
    "psllvd": "per-lane variable shift: data vs count",
    "psrlvq": "per-lane variable shift: data vs count",
    "psrlvd": "per-lane variable shift: data vs count",
    "psravd": "per-lane variable shift: data vs count",
    "psravq": "per-lane variable shift: data vs count",
    # interleave / pack: which source supplies the low half is significant
    "punpcklbw": "interleaves src1 and src2 in a fixed order",
    "punpcklwd": "interleaves src1 and src2 in a fixed order",
    "punpckldq": "interleaves src1 and src2 in a fixed order",
    "punpcklqdq": "interleaves src1 and src2 in a fixed order",
    "punpckhbw": "interleaves src1 and src2 in a fixed order",
    "punpckhwd": "interleaves src1 and src2 in a fixed order",
    "punpckhdq": "interleaves src1 and src2 in a fixed order",
    "punpckhqdq": "interleaves src1 and src2 in a fixed order",
    "packsswb": "src1 fills the low half of each destination group",
    "packssdw": "src1 fills the low half of each destination group",
    "packuswb": "src1 fills the low half of each destination group",
    "packusdw": "src1 fills the low half of each destination group",
    # ordered compare
    "pcmpgtb": "ordered greater-than, not equality",
    "pcmpgtw": "ordered greater-than, not equality",
    "pcmpgtd": "ordered greater-than, not equality",
    "pcmpgtq": "ordered greater-than, not equality",
    # index vector vs data vector
    "permd": "src1 is the index vector, src2 the data",
    "permq": "src1 is the index vector, src2 the data",
    "permps": "src1 is the index vector, src2 the data",
    "permpd": "src1 is the index vector, src2 the data",
    # IEEE min/max and subtraction/division are operand-order sensitive for
    # NaN and signed-zero payloads: min(a,b) returns b when either is NaN.
    "minps": "IEEE min returns the SECOND source on NaN/signed-zero ties",
    "minpd": "IEEE min returns the SECOND source on NaN/signed-zero ties",
    "maxps": "IEEE max returns the SECOND source on NaN/signed-zero ties",
    "maxpd": "IEEE max returns the SECOND source on NaN/signed-zero ties",
    "subps": "subtraction is not symmetric",
    "subpd": "subtraction is not symmetric",
    "divps": "division is not symmetric",
    "divpd": "division is not symmetric",
}

# Operations proven symmetric whose permissive flag must not be lost.  Only
# names that actually appear in the tables belong here; adding an unused name
# is harmless but pointless.
SYMMETRIC = {
    # two SIGNED word streams -> symmetric (the SA-36 defect).
    "pmaddwd": "both sources are signed word streams; products commute",
    # |a-b| is symmetric
    "psadbw": "sum of absolute differences is symmetric",
}

# EVEX.512 renames the bitwise ops with a `q` suffix; same operation.
Q_ALIASES = {"pandq": "pand", "porq": "por", "pxorq": "pxor"}


def base_mnemonic(name: str) -> str:
    """Normalize a mnemonic to its width-independent operation name."""
    n = name[1:] if name.startswith("v") else name
    return Q_ALIASES.get(n, n)


class ParseError(RuntimeError):
    pass


def parse_tables(intrinsics: str, intrinsics_simd: str) -> dict[str, dict[str, set[bool]]]:
    """Return {base_mnemonic: {width: {flags seen}}}.

    Fail-closed: every extractor must find its anchor, otherwise the source
    moved and the gate must not silently pass on an empty table.
    """
    table: dict[str, dict[str, set[bool]]] = {}

    def record(name: str, width: str, flag: bool) -> None:
        table.setdefault(base_mnemonic(name), {}).setdefault(width, set()).add(flag)

    m = re.search(
        r"fn sse128_mnemonic_commutative\(.*?\n    \}", intrinsics, re.S
    )
    if not m:
        raise ParseError("sse128_mnemonic_commutative not found in intrinsics.rs")
    allow = re.findall(r'"([a-z0-9]+)"', m.group(0))
    if not allow:
        raise ParseError("sse128_mnemonic_commutative allowlist parsed empty")
    for name in allow:
        record(name, "128", True)

    calls256 = re.findall(
        r'emit_avx_binary_256\(\s*\w+,\s*args,\s*"([^"]+)",\s*(true|false)',
        intrinsics,
    )
    if not calls256:
        raise ParseError("no emit_avx_binary_256 call sites found")
    for name, flag in calls256:
        record(name, "256", flag == "true")

    calls512 = re.findall(
        r'emit_evex_binary_512\(\s*\w+,\s*args,\s*"([^"]+)",\s*(true|false)',
        intrinsics_simd,
    )
    if not calls512:
        raise ParseError("no emit_evex_binary_512 call sites found")
    for name, flag in calls512:
        record(name, "512", flag == "true")

    return table


def _brace_arms(src: str, header: re.Pattern[str]):
    """Yield (capture, arm_body) with real brace matching.

    A greedy `\\{(.{0,N}?)emit_...\\}` regex silently pairs an arm with a
    LATER arm's emit call (it matched `LoadI32x4` to `vaddpd`), which would
    make this gate report fiction.  Depth counting is the only correct scan.
    """
    for m in header.finditer(src):
        start = m.end() - 1
        depth = 0
        for k in range(start, len(src)):
            if src[k] == "{":
                depth += 1
            elif src[k] == "}":
                depth -= 1
                if depth == 0:
                    yield m.group(1), src[start : k + 1]
                    break


def parse_emitter_op_flags(intrinsics: str) -> dict[str, tuple[str, bool]]:
    """{IntrinsicOp variant: (mnemonic, commutative)} from the 256-bit arms."""
    out: dict[str, tuple[str, bool]] = {}
    header = re.compile(r"IntrinsicOp::(\w+)\s*=>\s*\{")
    call = re.compile(
        r'emit_avx_binary_256\(\s*\w+,\s*args,\s*"([^"]+)",\s*(true|false)'
    )
    for op, body in _brace_arms(intrinsics, header):
        m = call.search(body)
        if m:
            out[op] = (m.group(1), m.group(2) == "true")
    if not out:
        raise ParseError("no IntrinsicOp -> emit_avx_binary_256 arms recovered")
    # Scan-integrity invariant on the REAL file, not just on synthetic input.
    # `_brace_arms` counts braces without lexing string literals or comments,
    # so a stray `}` in a comment could in principle end an arm early and pair
    # it with a later arm's emit call.  That failure has a unmistakable
    # signature: a pure data-movement op mapped to an arithmetic mnemonic
    # (the greedy-regex bug mapped `LoadI32x4` -> `vaddpd`).  A load, store or
    # constant materialisation can never emit an arithmetic opcode, so if one
    # ever does, the scan is broken and every number this gate prints would be
    # fiction.  Fail loudly instead of reporting it.
    arithmetic = re.compile(
        r"^v(?:p?(?:add|sub|mul|madd|maddubs|min|max|and|or|xor|sad|avg)"
        r"|add|sub|mul|div|min|max|and|or|xor)"
    )
    for op, (mnemonic, _flag) in sorted(out.items()):
        if ("Load" in op or "Store" in op or "Const" in op) and arithmetic.match(
            mnemonic
        ):
            raise ParseError(
                f"{op} mapped to arithmetic mnemonic {mnemonic!r}: the brace "
                f"scan mis-paired a match arm, so the whole table is untrustworthy"
            )
    return out


def parse_admission(coalescing: str, fname: str) -> dict[str, bool]:
    """{IntrinsicOp variant: commutative} from a memfold_consumer_* table.

    Handles grouped arms (`A | B | C => Some(true)`), which is how these
    tables are written.
    """
    i = coalescing.find(f"pub(crate) fn {fname}(")
    if i < 0:
        raise ParseError(f"{fname} not found in copy_coalescing.rs")
    j = coalescing.find("\n}\n", i)
    if j < 0:
        raise ParseError(f"{fname} body not terminated")
    body = coalescing[i:j]
    out: dict[str, bool] = {}
    pat = re.compile(r"((?:O::\w+\s*\|\s*)*O::\w+)\s*=>\s*Some\((true|false)\)")
    for m in pat.finditer(body):
        for op in re.findall(r"O::(\w+)", m.group(1)):
            out[op] = m.group(2) == "true"
    if not out:
        raise ParseError(f"{fname} parsed empty")
    return out


def check_cross_layer(
    emitter: dict[str, tuple[str, bool]], admissions: dict[str, dict[str, bool]]
) -> list[str]:
    """The IR admission gate and the emitter must agree on the SAME op.

    This is the fourth authority.  `memfold_consumer_256` withheld the
    `VecMaddwdI16x16` fold under a comment that reasoned about `maddubs`'
    mixed signedness, while the 512-bit emitter already said `true`.  Fixing
    only the emitter would have left the optimization inert: the analysis
    decides whether args[0] is marked foldable at all.
    """
    problems: list[str] = []
    for op, (mnemonic, emit_flag) in sorted(emitter.items()):
        for fname, table in sorted(admissions.items()):
            if op not in table:
                continue
            if table[op] != emit_flag:
                problems.append(
                    f"{op} ({mnemonic}): emitter says commutative={emit_flag} "
                    f"but {fname} says {table[op]}. The analysis is the "
                    f"admission authority, so the emitter flag alone cannot "
                    f"gain or protect a fold -- they must agree."
                )
    return problems


def check(table: dict[str, dict[str, set[bool]]]) -> list[str]:
    problems: list[str] = []
    for name in sorted(table):
        widths = table[name]
        if name in ASYMMETRIC:
            for width, flags in sorted(widths.items()):
                if True in flags:
                    problems.append(
                        f"{name} ({width}-bit) is marked commutative but is "
                        f"ASYMMETRIC: {ASYMMETRIC[name]}. Folding the left "
                        f"operand swaps the two operand roles -> wrong code."
                    )
        if name in SYMMETRIC:
            for width, flags in sorted(widths.items()):
                if flags == {False}:
                    problems.append(
                        f"{name} ({width}-bit) is marked non-commutative but is "
                        f"SYMMETRIC: {SYMMETRIC[name]}. This only costs a legal "
                        f"left-operand memory fold."
                    )
        flat: set[bool] = set()
        for flags in widths.values():
            flat |= flags
        if len(flat) > 1:
            detail = ", ".join(
                f"{w}={'/'.join(str(sorted(f)) for f in [fl])[1:-1]}"
                for w, fl in sorted(widths.items())
            )
            problems.append(
                f"{name} disagrees across widths ({detail}). Either the "
                f"operation is asymmetric and the permissive arm is wrong "
                f"code, or it is symmetric and the conservative arm loses a "
                f"fold. Add it to ASYMMETRIC or SYMMETRIC with the SDM reason."
            )
    return problems


def run(root: Path) -> int:
    ip = root / "src/backend/x86/codegen/intrinsics.rs"
    sp = root / "src/backend/x86/codegen/intrinsics_simd.rs"
    try:
        intrinsics = ip.read_text()
        simd = sp.read_text()
    except OSError as exc:
        print(f"ERROR: cannot read source: {exc}", file=sys.stderr)
        return 2
    try:
        table = parse_tables(intrinsics, simd)
    except ParseError as exc:
        print(f"ERROR: {exc}", file=sys.stderr)
        return 2

    problems = check(table)
    try:
        coalescing = (
            root / "src/backend/stack_layout/copy_coalescing.rs"
        ).read_text()
        emitter = parse_emitter_op_flags(intrinsics)
        admissions = {
            f: parse_admission(coalescing, f)
            for f in ("memfold_consumer_128", "memfold_consumer_256")
        }
    except (OSError, ParseError) as exc:
        print(f"ERROR: {exc}", file=sys.stderr)
        return 2
    cross = check_cross_layer(emitter, admissions)
    problems += cross
    widths = sum(len(v) for v in table.values())
    if problems:
        print(f"check_simd_commutativity_agreement: FAIL ({len(problems)} problems)")
        for p in problems:
            print(f"  - {p}")
        return 1
    print(
        f"check_simd_commutativity_agreement: PASS "
        f"({len(table)} operations, {widths} width rows, "
        f"{len(ASYMMETRIC)} asymmetric + {len(SYMMETRIC)} symmetric pinned, "
        f"{sum(len(v) for v in admissions.values())} IR admission rows "
        f"cross-checked against {len(emitter)} emitter arms)"
    )
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--root", type=Path, default=ROOT)
    args = ap.parse_args()
    return run(args.root)


if __name__ == "__main__":
    sys.exit(main())
