#!/usr/bin/env python3
"""Per-instruction dynamic attribution for Callgrind profiles.

lccc does not emit DWARF line tables, so `callgrind_annotate --auto=yes`
cannot attribute executed instructions to source lines.  It can, however,
attribute them to *instruction addresses*: collect with

    valgrind --tool=callgrind --dump-instr=yes --cache-sim=yes \\
             --callgrind-out-file=cg.out ./binary

(`positions: instr` makes the cost-line positions hexadecimal instruction
addresses) and join them here with `objdump -d` to see exactly which machine
instructions burn the dynamic count, in address order, with source lines for
whichever compiler does have debug info.

Usage:
  hot_instr.py --cg cg.out --binary ./prog [--object SUBSTR] [--top N]
               [--json out.json] [--min-share 0.001]

The --object filter works the same way as tools/oracle/callgrind_own_ir.py:
by default the profile's own executable is used (the `cmd:` header), so the
loader's and libc's instructions are excluded from every total.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from collections import defaultdict

HDR = re.compile(r"^([a-z]{2,3})=\((\d+)\)\s*(.*)$")
DIS = re.compile(r"^\s*([0-9a-f]+):\s+(.*?)\s*$")

# A cost line is `pos... count...`: one token per position declared by the
# `positions:` header (hex/dec numbers, or `*`, `+n`, `-n`), then one token per
# event declared by `events:`.  `--dump-instr=yes` gives `positions: instr line`,
# so the line is e.g. `0x401136 40 12` or `+7 40 12` (address relative to the
# previous instruction, which is how un-executed padding is compressed).
POS_TOKEN = re.compile(r"^(0x[0-9a-f]+|\d+|[*]|[+\-]\d+)$")
EV_TOKEN = re.compile(r"^\d[\d ]*$")


def disassemble(binary: str) -> dict[int, str]:
    out = subprocess.run(["objdump", "-d", "--no-show-raw-insn", binary],
                         capture_output=True, text=True, check=False)
    insns: dict[int, str] = {}
    for line in out.stdout.splitlines():
        m = DIS.match(line)
        if m:
            insns[int(m.group(1), 16)] = m.group(2)
    return insns


def profile(path: str):
    """Parse a Callgrind profile into (per_addr_ir, per_addr_line, fns, objs, cmd).

    Only `positions: instr [line]` profiles are useful here -- they are
    produced by `--dump-instr=yes`.  Costs are attributed to the instruction
    address, and the caller filters by binary address range.
    """
    objs: dict[int, str] = {}
    fns: dict[int, str] = {}
    per_addr: dict[int, int] = defaultdict(int)
    per_addr_line: dict[int, int] = defaultdict(int)
    cur_ob = cur_fn = None
    pos = [0, 0]        # current instr address, current line
    npos, cmd = 0, ""
    call_record = False  # previous line was `calls=`: next cost line is the
                         # callee's *inclusive* cost at the call site, not a
                         # self cost, and must not be summed
    with open(path, "r", errors="replace") as fh:
        for line in fh:
            line = line.rstrip("\n")
            if not line:
                continue
            if line.startswith("cmd:"):
                cmd = line[4:].strip()
                continue
            if line.startswith("calls="):
                call_record = True
                continue
            if line.startswith("positions:"):
                npos = len([w for w in line.split()[1:] if w])
                continue
            m = HDR.match(line)
            if m:
                kind, idx, nm = m.group(1), int(m.group(2)), m.group(3).strip()
                if kind == "ob":
                    if nm:
                        objs[idx] = nm
                    cur_ob = idx
                elif kind in ("fn", "cfn"):
                    if nm:
                        fns[idx] = nm
                    cur_fn = idx
                elif kind == "cob":
                    cur_ob = idx
                continue
            if npos == 0 or cur_ob is None or cur_fn is None:
                continue
            if call_record:
                call_record = False
                continue
            toks = line.split()
            if len(toks) <= npos or not all(POS_TOKEN.match(t) for t in toks[:npos]):
                continue
            if not all(EV_TOKEN.match(t) for t in toks[npos:]):
                continue
            for i, t in enumerate(toks[:npos]):
                if t == "*":
                    continue
                if t[0] in "+-":
                    pos[i] += int(t)
                else:
                    pos[i] = int(t, 0)
            per_addr[pos[0]] += int(toks[npos])
            per_addr_line[pos[0]] = pos[1] if npos > 1 else 0
    return per_addr, per_addr_line, fns, objs, cmd


LOAD = re.compile(r"^(mov|movz|movs)[a-z]*\s")
STORE_MNEMONICS = ("mov", "movz", "movs")
ALU = re.compile(r"^(add|sub|xor|and|or|not|neg|shl|shr|sar|sal|rol|ror|rol|imul|mul|div|idiv|adc|sbb|inc|dec|cmov[a-z]+)\s")
CONTROL = re.compile(r"^(cmp|test|j[a-z]+|call|ret|nop|hlt)\b")


def classify(mnemonic: str, operands: str) -> str:
    """Coarse class for one disassembled instruction (operands from objdump)."""
    mem_dst = re.match(r"^[^,]*%[a-z0-9]+\b", operands) is None
    if mnemonic == "lea":
        return "lea"
    if mnemonic.startswith(("mov", "movz", "movs")):
        if "(" in operands.split(",")[0]:
            return "load"
        if len(operands.split(",")) > 1 and "(" in operands.split(",", 1)[1]:
            return "store"
        return "regmov"
    if CONTROL.match(mnemonic):
        return "control"
    if mnemonic in ("push", "pop"):
        return "stack"
    if ALU.match(mnemonic):
        return "alu"
    if mem_dst and "(" in operands:
        return "memop-other"
    return "other"


def class_totals(kept, insns, total):
    totals: dict[str, int] = defaultdict(int)
    for a, v in kept.items():
        text = insns[a]
        parts = text.split(None, 1)
        mn = parts[0]
        ops = parts[1] if len(parts) > 1 else ""
        totals[classify(mn, ops)] += v
    return sorted(totals.items(), key=lambda kv: -kv[1])


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--cg", required=True)
    ap.add_argument("--binary", required=True)
    ap.add_argument("--object", default=None)
    ap.add_argument("--top", type=int, default=25)
    ap.add_argument("--json", default=None)
    ap.add_argument("--min-share", type=float, default=0.0)
    ap.add_argument("--order", choices=("ir", "addr"), default="ir")
    ap.add_argument("--classes", action="store_true",
                    help="also summarise executed Ir by instruction class")
    args = ap.parse_args()

    per_addr, per_addr_line, _fns, _objs, _cmd = profile(args.cg)
    # Selecting the program's own instructions is an address-range test: an
    # address that objdump reports for this binary cannot come from another
    # object in the process.  This requires a non-PIE executable (lccc's
    # default; `-no-pie` for the reference compilers), because for a PIE the
    # profiled runtime addresses carry a load bias objdump knows nothing about.
    insns = disassemble(args.binary)
    total = sum(v for a, v in per_addr.items() if a in insns)
    kept = {a: v for a, v in per_addr.items() if a in insns and (v / max(total, 1)) >= args.min_share}
    hot = sorted(kept.items(), key=lambda kv: (-kv[1] if args.order == "ir" else kv[0]))
    print(f"binary: {args.binary}   own Ir: {total}   distinct insns: {len(kept)}")
    print(f"{'Ir':>12}  {'share':>6}  {'addr':>10}  {'line':>5}  instruction")
    for a, v in hot[: args.top]:
        ln = per_addr_line.get(a, 0)
        print(f"{v:>12}  {100.0 * v / total:5.2f}%  0x{a:08x}  {ln:>5}  {insns[a]}")
    if args.classes:
        print()
        for name, ir in class_totals(kept, insns, total):
            print(f"{name:>16}  {ir:>12}  {100.0 * ir / max(total, 1):5.2f}%")
    if args.json:
        with open(args.json, "w") as fh:
            json.dump({"binary": args.binary, "own_ir": total,
                       "hot": [{"addr": f"0x{a:08x}", "ir": v, "insn": insns[a]}
                               for a, v in hot]}, fh, indent=1)
        print(f"wrote {args.json}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
