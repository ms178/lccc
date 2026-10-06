#!/usr/bin/env python3
"""Adversarial operand-legality *spelling* differ: GNU as's verdict vs ours.

The curated matrix (`aarch64_operand_legality_matrix.py`) pins verdicts for
rows a human chose.  This is the complementary instrument: it generates the
cross product of a handful of operand *spellings* -- zero and stack pointer
registers in every load/store slot, FP lanes across every addressing mode, the
system-register and pstate-immediate domains, the whole immediate range of the
shifted/extended forms -- and reports every row where our assembler disagrees
with the pinned GNU as.  It is small enough to run while editing the encoder
and wide enough to reach spellings no curated row covers: `ldrb xzr,[x0]` and
`ldrb wzr,[x0]` are different accesses, `str q31,[x0]` and `str v31,[x0]` are
different spellings of the same one, and only one of each pair is legal.

Disagreements are printed with the direction of the disagreement
(WE-ACCEPT-GAS-REJECTS / WE-REJECT-GAS-ACCEPTS / ENCODING-DIFFERS).  A
non-empty disagreement list exits 1, except for the rows listed in
`PINNED_DIVERGENCES`, which are deliberate and separately argued: they are the
places where the architecture's *architected* encoding differs from what this
assembler chooses to emit for a spelling GNU as does not implement at all.

Usage:
    scripts/aarch64_legality_probe_differ.py [--lccc PATH] [--as PATH]
                                             [--objcopy PATH] [--json]
"""

from __future__ import annotations

import argparse
import shutil
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import aarch64_operand_legality_matrix as M  # noqa: E402

REPO = Path(__file__).resolve().parent.parent
PIN = Path.home() / ".cache" / "gas-2.47-aarch64-linux-gnu" / "bin"

# Deliberate, argued divergences.  Each entry is a spelling whose *architected*
# encoding this assembler emits while GNU as refuses the spelling outright, or
# vice versa; they are pinned so that a new disagreement cannot hide behind
# them.  (The matrix carries the same exceptions as rows for the same reason.)
PINNED_DIVERGENCES: dict[str, str] = {
    # Measured on GNU as 2.47: after the five-field raw spelling
    # `s3_0_c1_c0_1`, GAS ignores *arbitrary* trailing text --
    # `s3_0_c1_c0_1_2`, `s3_0_c1_c0_1_99`, `s3_0_c1_c0_1_xyz` and
    # `s3_0_c1_c0_1_2_3` all assemble to ACTLR_EL1's word 0xd5381020, while a
    # wrong *field* (`s3_9_c9_c9_9`, `s3_0_c1_c0`, `s4_0_c1_c0_1`) is refused.
    # The architected syntax is exactly five fields, so accepting the junk
    # would make `mrs x0,s3_0_c1_c0_1xyz` a valid spelling of a system
    # register: this assembler refuses it, deliberately, and the divergence is
    # pinned here rather than silently tolerated.  Four spellings (mrs/msr of
    # the two over-long forms) reach the probe's generator.
    "s3_0_c1_c0_1_2": "GAS ignores the sixth field; we refuse the spelling",
    "s3_0_c1_c0_1extra": "same, deliberately over-long spelling",
}

PRE = ".text\n"


def gen():
    out = []
    # 1. zero/special register spellings across the whole load/store space
    for rt in ("wzr", "xzr", "w31", "x31", "sp", "wsp", "q31", "q0", "b31", "h31",
               "s31", "d31", "v31"):
        for mn, form in (("ldr", "{} [x0]"), ("str", "{} [x0]"),
                         ("ldrb", "{} [x0]"), ("strb", "{} [x0]"),
                         ("ldrh", "{} [x0]"), ("strh", "{} [x0]"),
                         ("ldrsb", "{} [x0]"), ("ldrsh", "{} [x0]"),
                         ("ldur", "{} [x0,#8]"), ("stur", "{} [x0,#8]"),
                         ("ldtr", "{} [x0,#8]"), ("sttr", "{} [x0,#8]")):
            out.append(f"{mn} {form.format(rt)}")
    # 2. q31 / FP lanes with every addressing mode
    for mn in ("ldr", "str"):
        for rt in ("q0", "q31", "d0", "s0", "h0", "b0"):
            for am in ("[x0]", "[x0,#16]", "[x0,#1]", "[x0],#16", "[x0],x1",
                       "[x0,#16]!"):
                out.append(f"{mn} {rt},{am}")
    # 3. sp as base, sp as data, in the scaled and FP spaces
    for mn in ("ldr", "str"):
        for rt in ("x0", "w0", "q0", "d0"):
            out.append(f"{mn} {rt},[sp]")
            out.append(f"{mn} {rt},[sp,#16]")
    # 4. sign-extending trio destination rules
    for mn in ("ldrsb", "ldrsh", "ldrsw"):
        for rt in ("w0", "x0", "wzr", "xzr", "sp"):
            out.append(f"{mn} {rt},[x0]")
    # 5. ldp/stp with sp / wzr / xzr / FP
    for mn in ("ldp", "stp"):
        for a, b in (("x0", "x1"), ("w0", "w1"), ("x0", "sp"), ("wzr", "xzr"),
                     ("d0", "d1"), ("q0", "q1"), ("s0", "s1"), ("x0", "w1")):
            out.append(f"{mn} {a},{b},[sp]")
            out.append(f"{mn} {a},{b},[x0,#8]")
    # 6. system-register spellings: trailing fields, mixed case, junk
    for name in ("s3_0_c1_c0_1_2", "s3_0_c1_c0_1extra", "S3_0_C1_C0_1",
                 "s3_0_c1_c0_1", "s03_0_c01_c0_01", "s3_0_C1_c0_1",
                 "s0_0_c0_c0_0", "s3_7_c15_c15_7", "DBGBVR7_EL1",
                 "DbgBvr7_El1", "pmevtyper10_el0", "PMEVTYPER10_EL0"):
        out.append(f"mrs x0,{name}")
        out.append(f"msr {name},x0")
    # 7. pstate immediates: the full immediate domain
    for field in ("daifset", "daifclr", "spsel", "pan", "uao", "ssbs", "dit",
                  "tco", "allint", "svcrsm", "svcrza", "svcrsmza", "nzcv",
                  "daif", "currentel", "spsr_el1"):
        for imm in ("#0", "#1", "#7", "#8", "#15", "#16"):
            out.append(f"msr {field},{imm}")
    return out


def _direction(gas: str, lc: str) -> str:
    if gas == "REJECT":
        return "WE-ACCEPT-GAS-REJECTS"
    if lc == "REJECT":
        return "WE-REJECT-GAS-ACCEPTS"
    return "ENCODING-DIFFERS"


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--lccc", default=str(REPO / "target" / "fastbuild" / "lccc"))
    ap.add_argument("--as", dest="as_bin", default=str(PIN / "as"))
    ap.add_argument("--objcopy", default=str(PIN / "objcopy"))
    ap.add_argument("--json", action="store_true")
    args = ap.parse_args()

    for tool in (args.as_bin, args.objcopy):
        if not shutil.which(tool) and not Path(tool).exists():
            print(f"legality probe: {tool} not found", file=sys.stderr)
            return 2
    lccc = Path(args.lccc)
    if not lccc.exists():
        print(f"legality probe: no such lccc: {lccc}", file=sys.stderr)
        return 2

    with tempfile.TemporaryDirectory() as td:
        tmp = Path(td)
        link = tmp / "aarch64-linux-gnu-ccc"
        link.symlink_to(lccc.resolve())
        bad, pinned, n = [], [], 0
        for insn in gen():
            n += 1
            gas = M.assemble(insn, [args.as_bin], args.objcopy, tmp, prologue=PRE)
            lc = M.assemble(insn, [str(link), "-c"], args.objcopy, tmp, prologue=PRE)
            if gas != lc:
                # A pinned divergence is keyed on any operand spelling, not on
                # the first one: for `mrs` the register is the *second*
                # operand, and keying on the destination would silently report
                # the pinned rows as new defects.
                operands = (
                    insn.split(None, 1)[1].split(",") if " " in insn else [insn]
                )
                if any(op.strip() in PINNED_DIVERGENCES for op in operands):
                    pinned.append((insn, gas, lc))
                else:
                    bad.append((insn, gas, lc))

    if args.json:
        import json
        print(json.dumps({
            "rows": n,
            "disagreements": [{"insn": i, "gas": g, "lccc": l, "direction": _direction(g, l)}
                              for i, g, l in bad],
            "pinned": [{"insn": i, "gas": g, "lccc": l} for i, g, l in pinned],
        }, indent=2))
    else:
        print(f"legality probe: {n} spellings, {len(bad)} unpinned "
              f"disagreement(s), {len(pinned)} pinned")
        for insn, gas, lc in bad:
            print(f"  {_direction(gas, lc)}: [{insn}] gas={gas} lccc={lc}")
        for insn, gas, lc in pinned:
            print(f"  (pinned) [{insn}] gas={gas} lccc={lc}")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
