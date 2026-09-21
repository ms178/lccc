#!/usr/bin/env python3
"""Instruction-mix histogram of the Linux x86 boot objects: lccc vs oracle.

`boot_size_oracle.sh` reports bytes per object, `boot_fn_census.py` narrows
that to functions.  Neither says *why* the bytes are there.  This script
disassembles every `.text*` section of both object sets — 16-bit aware, which
matters because the boot code is `-m16` and a 32-bit decode of it is noise —
and reports the per-opcode count and byte delta.

That ranking is what turns a size gap into a work item: a code-generation
defect shows up as one opcode family owning a disproportionate share of the
delta (redundant widening moves, spill traffic, missed addressing-mode
folds), while a broad "everything is 20% bigger" shape points at frame
layout instead.

    scripts/boot_size_oracle.sh                 # produces $OUT, $OUT/oracle-gcc
    scripts/boot_isel_hist.py                   # opcode delta ranking
    scripts/boot_isel_hist.py --object printf   # one object
    scripts/boot_isel_hist.py --spill           # stack-traffic breakdown

Environment:
    OUT    lccc object dir   (default /var/tmp/bootbuild)
    OOUT   oracle object dir (default $OUT/oracle-gcc)
"""
from __future__ import annotations

import argparse
import collections
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

DEFAULT_OUT = Path(os.environ.get("OUT", "/var/tmp/bootbuild"))
INSN_RE = re.compile(r"^\s*[0-9a-f]+:\t([0-9a-f][0-9a-f ]*)\t(\S+)\s*(.*)$")


def text_sections(obj: Path) -> list[str]:
    """Non-empty `.text`/`.text.*` sections (`-ffunction-sections`)."""
    if not obj.exists():
        return []
    out = subprocess.run(["readelf", "-SW", str(obj)],
                         capture_output=True, text=True, check=True).stdout
    res = []
    for line in out.splitlines():
        f = line.split()
        idx = [i for i, t in enumerate(f) if t.startswith(".text")]
        # `[ 4] .text.foo PROGBITS 00000000 000034 0002d4 ...`: the size is
        # the 5th token after the name (`[ 4]` splits into two fields).
        if not idx or len(f) < idx[0] + 5:
            continue
        try:
            size = int(f[idx[0] + 4], 16)
        except ValueError:
            continue
        if size:
            res.append(f[idx[0]])
    return res


def disasm(obj: Path, tmp: Path) -> list[tuple[str, str, int]]:
    """[(opcode, operands, encoded length)] over the object's code."""
    insns: list[tuple[str, str, int]] = []
    for sec in text_sections(obj):
        subprocess.run(
            ["objcopy", "-O", "binary", f"--only-section={sec}", str(obj), str(tmp)],
            capture_output=True, check=True)
        if tmp.stat().st_size == 0:      # empty section: objdump refuses the input
            continue
        # `-m i8086` selects the 16-bit default operand/address size, which is
        # exactly what `-m16` code expects; the plain elf32-i386 decode
        # mis-parses it (observed: `pusha`, `frstor`, `(bad)`).
        proc = subprocess.run(
            ["objdump", "-D", "-b", "binary", "-m", "i8086", "--adjust-vma=0",
             str(tmp)],
            capture_output=True, text=True)
        if proc.returncode != 0:
            print(f"warning: objdump failed on {obj} {sec}: "
                  f"{proc.stderr.strip().splitlines()[-1:]}", file=sys.stderr)
            continue
        out = proc.stdout
        for line in out.splitlines():
            m = INSN_RE.match(line)
            if m:
                insns.append((m.group(2), m.group(3).strip(),
                              len(m.group(1).split())))
    return insns


STACK_RE = re.compile(r"0x[0-9a-f]+\(%e?sp\)|0x[0-9a-f]+\(%e?bp\)")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--out", type=Path, default=DEFAULT_OUT)
    ap.add_argument("--oout", type=Path, default=None)
    ap.add_argument("--object", default=None)
    ap.add_argument("--top", type=int, default=30)
    ap.add_argument("--spill", action="store_true",
                    help="break the delta down by stack-referencing opcode")
    args = ap.parse_args()

    oout = args.oout or (args.out / "oracle-gcc")
    for d, what in ((args.out, "lccc"), (oout, "oracle")):
        if not d.is_dir():
            print(f"no {what} object dir: {d}\n"
                  f"run: KERNEL_DIR=... OUT={args.out} scripts/boot_size_oracle.sh",
                  file=sys.stderr)
            return 2

    objects = sorted(p.stem for p in args.out.glob("*.o"))
    if args.object:
        objects = [o for o in objects if args.object in o]

    acc = {k: (collections.Counter(), collections.Counter()) for k in ("lccc", "oracle")}
    insns_total = {k: 0 for k in ("lccc", "oracle")}
    bytes_total = {k: 0 for k in ("lccc", "oracle")}
    # [opcode counts, opcode bytes, total stack bytes]
    stack = {k: [collections.Counter(), collections.Counter(), 0] for k in ("lccc", "oracle")}
    with tempfile.TemporaryDirectory() as td:
        tmp = Path(td) / "sec.bin"
        for name in objects:
            for k, d in (("lccc", args.out), ("oracle", oout)):
                for op, ops, n in disasm(d / f"{name}.o", tmp):
                    acc[k][0][op] += 1
                    acc[k][1][op] += n
                    insns_total[k] += 1
                    bytes_total[k] += n
                    if STACK_RE.search(ops):
                        stack[k][0][op] += 1
                        stack[k][1][op] += n
                        stack[k][2] += n

    rows = sorted(((acc["lccc"][1][op] - acc["oracle"][1][op], op)
                   for op in set(acc["lccc"][0]) | set(acc["oracle"][0])),
                  reverse=True)
    print(f"{'opcode':16} {'lccc#':>6} {'orc#':>6} {'lcccB':>7} {'orcB':>7} {'dB':>7}")
    print("-" * 58)
    for delta, op in rows[: args.top]:
        print(f"{op:16} {acc['lccc'][0][op]:6d} {acc['oracle'][0][op]:6d} "
              f"{acc['lccc'][1][op]:7d} {acc['oracle'][1][op]:7d} {delta:+7d}")
    print(f"{'TOTAL':16} {insns_total['lccc']:6d} {insns_total['oracle']:6d} "
          f"{bytes_total['lccc']:7d} {bytes_total['oracle']:7d} "
          f"{bytes_total['lccc'] - bytes_total['oracle']:+7d}")

    if args.spill:
        print("\nStack-referencing (frame/spill/argument) traffic:")
        for k in ("lccc", "oracle"):
            c, b, tot = stack[k]
            top = ", ".join(f"{op}:{n}({b[op]}B)" for op, n in c.most_common(6))
            print(f"  {k:7} {tot:6d} B  {top}")
        print(f"  delta   {stack['lccc'][2] - stack['oracle'][2]:+6d} B")
    return 0


if __name__ == "__main__":
    sys.exit(main())
