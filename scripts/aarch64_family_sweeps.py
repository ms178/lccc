#!/usr/bin/env python3
"""
aarch64_family_sweeps.py
========================

Exhaustive per-family differentials for the AArch64 encoder: the spaces that a
curated table cannot cover because they are *generated* rather than written
down.

Why this exists
---------------
`tests/aarch64/operand-legality.tsv` pins a row per *slot shape*: a human chose
the templates and the substitution set, so the table can only catch a defect in
a spelling somebody thought of.  Every family whose rules are combinatorial
(which widths may pair, which bit counts are legal, which operand positions are
pairs) therefore also needs the cross product, generated and compared against
the same pinned GNU as the matrix uses.  This script is that cross product for
three families, each of which produced a real, measured defect:

* **conversions** — the twelve scalar SIMD&FP `fcvt*` mnemonics: the
  register-file form (`fcvtms s0,s1`), the fixed-point forms
  (`fcvtzs s0,s1,#fbits`) at every legal and illegal bit count, the mixed-width
  refusals, and the general-purpose-destination forms that must keep working.
  Measured before the fix: we refused the register-file forms and accepted
  `fcvt s0,s1`, which GNU as refuses (we emitted an undefined word for it).
* **casp** — the two register *pairs*: which second halves are contiguous, where
  the zero register is a legal successor spelling (`casp x30,xzr,...`), and how
  the base register may be spelled.  Measured before the fix: we refused a
  spelling GNU as accepts.
* **addsub** — `add`/`adds`/`sub`/`subs` with a shift or an extend operand: kind
  × amount × operand width × extend kind, including the amounts at and past each
  width's limit and the forms that take SP in the middle slot.  This is the
  sweep the PR #766 review asked for ("a generated sweep of the shifted and
  extended forms").

A disagreement is printed with both verdicts and their encodings; the exit
status is non-zero if there is any, so the script can be used as a gate.

Usage:
    scripts/aarch64_family_sweeps.py [--lccc PATH] [--as PATH] [--objcopy PATH]
                                     [--family NAME ...] [--quiet]

The default `--lccc` is `target/fastbuild/lccc` and the default oracles are the
pinned 2.47 pair under `$HOME/.cache/gas-2.47-aarch64-linux-gnu/bin`, i.e. the
same tools `scripts/ensure_gas_247.sh aarch64-linux-gnu` provisions and the
CI gates use.
"""

from __future__ import annotations

import argparse
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import aarch64_operand_legality_matrix as M  # noqa: E402

REPO = Path(__file__).resolve().parent.parent
PIN = Path.home() / ".cache" / "gas-2.47-aarch64-linux-gnu" / "bin"


# ── family generators ──────────────────────────────────────────────────────
#
# Each generator yields instruction spellings; the sweep is the set, so a
# generator may (and should) over-cover -- including the spellings that must be
# *refused*, because "we accept what GNU as rejects" is the failure mode that
# nothing else in the toolchain sees.


def conversions() -> list[str]:
    """The twelve scalar SIMD&FP conversions, both forms, all widths.

    The `fcvt*` family has a general-purpose-destination form
    (`fcvtms w0,s1`) and a scalar register-file form (`fcvtms s0,s1`); the four
    integer-conversion mnemonics add a fixed-point form whose bit count ranges
    over the element width.  Register numbers 0/31 and the pair (31, 0) are
    included because the register fields are the only bits the two forms share.
    """
    out: list[str] = []
    regs = [(0, 1), (31, 1), (0, 31), (30, 2)]
    elements = ("h", "s", "d")
    bits = {"h": 16, "s": 32, "d": 64}
    mnemonics = [
        "fcvtns", "fcvtnu", "fcvtps", "fcvtpu", "fcvtms",
        "fcvtmu", "fcvtzs", "fcvtzu", "fcvtas", "fcvtau",
    ]
    conversions_all = mnemonics + ["scvtf", "ucvtf"]
    fixed_point = ["fcvtzs", "fcvtzu", "scvtf", "ucvtf"]

    for mn in conversions_all:
        for w in elements:
            for a, b in regs:
                out.append(f"{mn} {w}{a}, {w}{b}")
    # Mixed widths: no encoding, and GNU as refuses every one.
    for mn in conversions_all:
        for dw, sw in (("s", "d"), ("d", "s"), ("h", "s"), ("s", "h"), ("h", "d"), ("d", "h")):
            out.append(f"{mn} {dw}0, {sw}1")
    # Fixed-point forms at every bit count, including both out-of-range ends.
    for mn in fixed_point:
        for w, width in bits.items():
            for f in range(0, width + 2):
                out.append(f"{mn} {w}0, {w}1, #{f}")
    # The eight rounding mnemonics have no fixed-point form.
    for mn in mnemonics:
        if mn in fixed_point:
            continue
        for w in elements:
            out.append(f"{mn} {w}0, {w}1, #3")
    # Class boundaries: vector without an arrangement, other widths, GPR.
    for mn in conversions_all:
        for operands in (
            "v0, v1", "q0, q1", "b0, b1", "x0, x1", "s0", "s0, s1, s2",
            "s0, s1, #3, #4",
        ):
            out.append(f"{mn} {operands}")
    # The general-purpose-destination forms must keep working.
    for mn in mnemonics:
        for dst in ("w0", "x0", "w30", "xzr"):
            for src in ("s1", "d1", "h1"):
                out.append(f"{mn} {dst}, {src}")
    for mn in ("scvtf", "ucvtf"):
        for dst in ("s0", "d0", "h0"):
            for src in ("w1", "x1", "wzr"):
                out.append(f"{mn} {dst}, {src}")
    for mn in fixed_point:
        for dst, src in (
            ("w0", "s1"), ("x0", "d1"), ("w0", "h1"),
            ("s0", "w1"), ("d0", "x1"), ("h0", "w1"),
        ):
            for f in (0, 1, 16, 32, 33, 64, 65):
                out.append(f"{mn} {dst}, {src}, #{f}")
    return out


def casp() -> list[str]:
    """CASP's two register pairs, every pair-half spelling and base class."""
    out: list[str] = []
    for mn in ("casp", "caspa", "caspal", "caspl"):
        for r0 in (0, 2, 28, 30):
            for first in (f"x{r0}", f"x{r0 + 1}", "xzr", "sp", "lr"):
                for second in (f"x4", "x5", "xzr", "sp"):
                    out.append(f"{mn} x{r0}, {first}, x4, {second}, [x8]")
        # Odd pair starts, and the zero register as the first half.
        for r0 in (1, 31):
            out.append(f"{mn} x{r0}, x{r0 + 1}, x4, x5, [x8]")
        out.append(f"{mn} xzr, x1, x2, x3, [x8]")
        # Width mixtures within and across pairs.
        for pair in ("w0, w1", "w30, wzr", "w0, x1", "x0, w1", "w0, w31"):
            out.append(f"{mn} {pair}, x2, x3, [x8]")
            out.append(f"{mn} {pair}, w2, w3, [x8]")
        # Bases: SP is legal, XZR is not; no offset and no writeback form.
        for base in ("[x8]", "[sp]", "[xzr]", "[x8, #8]", "[x8, #8]!", "[x8], #8", "[x8, x1]"):
            out.append(f"{mn} x0, x1, x2, x3, {base}")
        # Arity.
        out.append(f"{mn} x0, x1, x2, [x8]")
        out.append(f"{mn} x0, x1, x2, x3, x4, [x8]")
        # No byte/halfword pair forms and no mistyped order letters.
        for bad in ("caspb", "casph", "caspq", "caspalb", "caspaq"):
            out.append(f"{bad} x0, x1, x2, x3, [x8]")
    return out


def addsub() -> list[str]:
    """`add`/`adds`/`sub`/`subs`: shifted and extended forms, widths, SP/ZR."""
    out: list[str] = []
    for mn in ("add", "adds", "sub", "subs"):
        for sh in ("lsl", "lsr", "asr", "ror"):
            for amt in (0, 1, 3, 31, 32, 63, 64, 65):
                out.append(f"{mn} x0, x1, x2, {sh} #{amt}")
            for amt in (0, 1, 3, 31, 32, 63, 64):
                out.append(f"{mn} w0, w1, w2, {sh} #{amt}")
        # Extended register form: kind x amount x width, SP in the middle slot.
        for ext in ("uxtb", "uxth", "uxtw", "uxtx", "sxtb", "sxth", "sxtw", "sxtx"):
            for amt in (None, 0, 1, 4, 5):
                suffix = "" if amt is None else f" #{amt}"
                for dst, a, b in (
                    ("x0", "x1", "x2"), ("x0", "x1", "w2"),
                    ("w0", "w1", "w2"), ("x0", "sp", "x2"),
                    ("w0", "sp", "w2"),
                ):
                    out.append(f"{mn} {dst}, {a}, {b}, {ext}{suffix}")
        # Encoding 31 in each slot, plain form.
        for dst, a, b in (
            ("sp", "x1", "x2"), ("x0", "sp", "x2"), ("x0", "x1", "sp"),
            ("x0", "x1", "xzr"), ("xzr", "x1", "x2"), ("x30", "x1", "x2"),
            ("wsp", "w1", "w2"), ("w0", "wsp", "w2"), ("w0", "w1", "wsp"),
            ("wzr", "w1", "w2"),
        ):
            out.append(f"{mn} {dst}, {a}, {b}")
        # Width mixtures.
        for a, b in (("x1", "w2"), ("w1", "x2"), ("x1", "x2")):
            out.append(f"{mn} x0, {a}, {b}")
            out.append(f"{mn} w0, {a}, {b}")
        # Immediate forms, with and without the 12-bit shift.
        for amt in (0, 1, 4095, 4096, 4097, 0xFFFFFF):
            out.append(f"{mn} x0, x1, #{amt}, lsl #12")
            out.append(f"{mn} x0, x1, #{amt}, lsl #1")
            out.append(f"{mn} x0, x1, #{amt}")
        out.append(f"{mn} sp, sp, #16")
        out.append(f"{mn} sp, x1, #16")
        out.append(f"{mn} x0, sp, #16")
    return out


FAMILIES = {
    "conversions": conversions,
    "casp": casp,
    "addsub": addsub,
}


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--lccc", default=str(REPO / "target" / "fastbuild" / "lccc"))
    ap.add_argument("--as", dest="as_bin", default=None)
    ap.add_argument("--objcopy", default=None)
    ap.add_argument(
        "--family",
        action="append",
        choices=sorted(FAMILIES),
        help="restrict to one family (repeatable; default: all)",
    )
    ap.add_argument("--quiet", action="store_true", help="print only the summary")
    args = ap.parse_args()

    as_bin = args.as_bin or str(PIN / "as")
    objcopy = args.objcopy or str(PIN / "objcopy")
    for tool, name in ((as_bin, "--as"), (objcopy, "--objcopy")):
        if not Path(tool).exists():
            print(f"aarch64 family sweeps: {name} {tool} does not exist", file=sys.stderr)
            return 2
    lccc = Path(args.lccc)
    if not lccc.exists():
        print(f"aarch64 family sweeps: no such lccc: {lccc}", file=sys.stderr)
        return 2
    # LCCC picks its backend from argv[0], so it is invoked through a symlink
    # named for the target; it also has no `.arch` directive, so the GNU
    # prologue is dropped on its side.
    with tempfile.TemporaryDirectory() as td:
        tmp = Path(td)
        link = tmp / "aarch64-linux-gnu-ccc"
        link.symlink_to(lccc.resolve())
        bad = 0
        total = 0
        for name in args.family or sorted(FAMILIES):
            spellings = sorted(set(FAMILIES[name]()))
            disagreements = []
            for insn in spellings:
                gas = M.assemble(insn, [as_bin], objcopy, tmp)
                ours = M.assemble(insn, [str(link), "-c"], objcopy, tmp, prologue=".text\n")
                if gas != ours:
                    disagreements.append((insn, gas, ours))
            total += len(spellings)
            bad += len(disagreements)
            print(f"{name}: {len(spellings)} spellings, {len(disagreements)} disagreement(s)")
            if not args.quiet:
                for insn, gas, ours in disagreements:
                    print(f"  {insn:36} gas={gas:20} lccc={ours}")
        print(f"total: {total} spellings, {bad} disagreement(s)")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
