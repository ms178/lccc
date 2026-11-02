#!/usr/bin/env python3
"""Scan lccc's corpus assembly for provably redundant instruction idioms.

Each rule below is a *local* fact: the second instruction repeats work the
first already guaranteed, or a value is copied to itself. These are the shapes
a peephole can delete without any analysis beyond the two lines.
"""
import re
import subprocess
import sys
from collections import Counter
from pathlib import Path

REPO = Path("/home/user/lccc")
PROGS = REPO / "tests/benchmark/programs"
LCCC = REPO / "target/fastbuild/lccc"
GCC = Path("/tmp/tc/opt/gcc-16/bin/gcc-16")

INSN = re.compile(r"^    ([a-z][a-z0-9]*)\s+(.*)$")
R32 = {"eax": "rax", "ebx": "rbx", "ecx": "rcx", "edx": "rdx", "esi": "rsi",
       "edi": "rdi", "ebp": "rbp", "esp": "rsp"}
for i in range(8, 16):
    R32[f"r{i}d"] = f"r{i}"


def norm(reg):
    return R32.get(reg, reg)


def scan(text):
    """Return list of (rule, line_no, line) for redundant pairs."""
    ins = []
    for line in text.splitlines():
        if line.startswith("#") or line.startswith(".") or line.endswith(":"):
            ins.append(None)
            continue
        m = INSN.match(line)
        ins.append((m.group(1), m.group(2).split("#")[0].strip()) if m else None)
    out = []
    for i in range(1, len(ins)):
        a, b = ins[i - 1], ins[i]
        if not (a and b):
            continue
        (op1, arg1), (op2, arg2) = a, b
        # 1. self-move: mov %X, %X
        if op1 in ("movq", "movl", "movw", "movb"):
            parts = [p.strip() for p in arg1.split(",")]
            if len(parts) == 2 and parts[0] == parts[1]:
                out.append(("self-move", i, f"{op1} {arg1}"))
        # 2. movzbl then and $0xff (or movzwl then and $0xffff)
        if op1 == "movzbl" and op2 in ("andl", "andq"):
            m2 = re.match(r"\$0x(ff|FF),", arg2)
            if m2 and arg1.split(",")[1].strip() in arg2:
                out.append(("zext-then-mask", i, f"{op1} {arg1} / {op2} {arg2}"))
        if op1 == "movzwl" and op2 in ("andl", "andq"):
            if re.match(r"\$0x(ffff|FFFF),", arg2) and arg1.split(",")[1].strip() in arg2:
                out.append(("zext-then-mask", i, f"{op1} {arg1} / {op2} {arg2}"))
        # 3. two consecutive identical masks on the same register
        if op1 in ("andl", "andq") and op2 == op1:
            m1 = re.match(r"\$(\w+), (%\w+)", arg1)
            m2 = re.match(r"\$(\w+), (%\w+)", arg2)
            if m1 and m2 and m1.group(2) == m2.group(2):
                out.append(("double-mask", i, f"{op1} {arg1} / {op2} {arg2}"))
        # 4. mov to a register that the next insn overwrites before reading
        if op1 in ("movl", "movq"):
            parts = [p.strip() for p in arg1.split(",")]
            if len(parts) == 2:
                dst = parts[1]
                d32 = dst.replace("%r", "%e") if dst.startswith("%r") and dst[2:].rstrip("d").isdigit() else None
                if op2.startswith("mov") and arg2.rstrip().endswith(dst.lstrip("%")):
                    src2 = arg2.split(",")[0].strip()
                    if src2 != dst:
                        out.append(("dead-copy-before-overwrite", i, f"{op1} {arg1} / {op2} {arg2}"))
    return out


def main():
    matches = sys.argv[1:] or ["self-move", "zext-then-mask", "double-mask"]
    total = Counter()
    hits = Counter()
    for src in sorted(PROGS.glob("*.c")):
        r = subprocess.run([str(LCCC), "-O2", "-S", str(src), "-o", "-"],
                           capture_output=True, text=True)
        if r.returncode:
            continue
        for rule, ln, txt in scan(r.stdout):
            if rule in matches:
                total[rule] += 1
                if hits[(src.stem, rule)] == 0:
                    pass
                hits[(src.stem, rule)] += 1
    print("rule                          count")
    for rule in matches:
        print(f"{rule:30} {total[rule]:5d}")
    print("\nby program:")
    for (prog, rule), n in sorted(hits.items(), key=lambda kv: -kv[1])[:20]:
        print(f"  {prog:26} {rule:24} {n}")


if __name__ == "__main__":
    main()
