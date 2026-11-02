#!/usr/bin/env python3
"""Census of dead callee-saved registers in x86-64 AT&T assembly.

A register counts as dead when the function prologue pushes it but nothing
in the body other than its own push/pop names it. Inline-asm regions
(`#APP`..`#NO_APP`) and instructions with implicit callee-saved writes
(`cpuid` writes %ebx) count as uses of every register, mirroring the
peephole's own conservative liveness, so the census never over-reports.

Usage:
    dead_callee_save_census.py [--max N] FILE.s...

Prints one line per function with dead saves and a TOTAL line. With
`--max N` the exit status is 1 when the total exceeds N (a regression
check after `lccc -O2 -S` over tests/benchmark/programs).
"""
import argparse
import re
import sys

CALLEE_SAVED = {
    "rbx": ("rbx", "ebx", "bx", "bl"),
    "r12": ("r12", "r12d", "r12w", "r12b"),
    "r13": ("r13", "r13d", "r13w", "r13b"),
    "r14": ("r14", "r14d", "r14w", "r14b"),
    "r15": ("r15", "r15d", "r15w", "r15b"),
    "rbp": ("rbp", "ebp", "bp", "bpl"),
}
LABEL = re.compile(r"^([A-Za-z_][\w.$]*):\s*$")
SIZE = re.compile(r"^\s*\.size\s")
IMPLICIT_WRITERS = ("cpuid",)


def functions(lines):
    """Yield (name, body lines) per symbol label, ending at `.size` or the
    next symbol label (data symbols put `.size` before their label)."""
    i = 0
    while i < len(lines):
        m = LABEL.match(lines[i])
        if not m:
            i += 1
            continue
        j = i + 1
        while j < len(lines) and not SIZE.match(lines[j]) and not LABEL.match(lines[j]):
            j += 1
        yield m.group(1), lines[i + 1 : j]
        i = j


def dead_saves(body):
    stripped = [l.strip() for l in body]
    opaque = any(l == "#APP" for l in stripped) or any(
        l.split(None, 1)[0] in IMPLICIT_WRITERS for l in stripped if l
    )
    insns = [l for l in stripped if l and not l.startswith((".", "#"))]
    pushes = []
    for l in insns:
        m = re.match(r"pushq %(\w+)$", l)
        if m and m.group(1) in CALLEE_SAVED:
            pushes.append(m.group(1))
            continue
        if l == "movq %rsp, %rbp":
            continue
        break
    frame_pointer = "movq %rsp, %rbp" in insns[: len(pushes) + 1]
    if opaque:
        return [], frame_pointer
    dead = []
    for reg in pushes:
        if reg == "rbp" and frame_pointer:
            continue
        name = re.compile(r"%(" + "|".join(CALLEE_SAVED[reg]) + r")\b")
        own = re.compile(r"(push|pop)q %" + reg + r"$")
        if not any(name.search(l) and not own.match(l) for l in insns):
            dead.append(reg)
    return dead, frame_pointer


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--max", type=int, default=None, help="fail when the total exceeds N")
    ap.add_argument("files", nargs="+")
    args = ap.parse_args()
    total = funcs = 0
    for path in args.files:
        with open(path, encoding="utf-8", errors="replace") as f:
            lines = f.read().split("\n")
        for name, body in functions(lines):
            dead, fp = dead_saves(body)
            if not dead:
                continue
            funcs += 1
            total += len(dead)
            calls = any(l.strip().startswith("call") for l in body)
            print(f"{path.rsplit('/', 1)[-1]}:{name} dead={dead} calls={calls} fp={fp}")
    print(f"TOTAL dead saves {total} in {funcs} functions")
    if args.max is not None and total > args.max:
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
