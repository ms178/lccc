#!/usr/bin/env python3
"""mixed_width_slots.py — stack-slot width-consistency auditor for x86-64 asm.

Per-function analysis: prints every stack slot accessed with more than one
memory width (e.g. a slot stored with `movq` but also loaded with `movl`).
Mixed-width slots are the signature of small-slot (4-byte spill) miscompiles:
a 4-byte store into an 8-byte slot leaves stale upper bytes that a later
64-bit read observes, and an 8-byte access of a 4-byte slot reads or writes
the neighbour's bytes.

Width model: zero/sign-extending loads count at their SOURCE width
(`movslq`/`movzbl`/`movswq`/`movzwl` read 1-4 bytes from memory); `movq`,
`addq mem`, `xorq mem` etc. touch 8 bytes.

Usage:
    python3 mixed_width_slots.py file.s [file2.s ...]
Exit status: 0 always (audit tool); the interesting signal is the report.
"""
import re
import sys

WMAP = {"movb": 1, "movw": 2, "movl": 4, "movq": 8,
        "movsbq": 1, "movzbl": 1, "movswq": 2, "movzwl": 2, "movslq": 4,
        "movsbl": 1, "movswl": 2,
        "cmpl": 4, "cmpq": 8, "cmpw": 2, "cmpb": 1,
        "addl": 4, "addq": 8, "subl": 4, "subq": 8,
        "andl": 4, "andq": 8, "orl": 4, "orq": 8,
        "xorl": 4, "xorq": 8, "imull": 4, "imulq": 8,
        "shll": 4, "shlq": 8, "testl": 4, "testq": 8}


def audit(path: str) -> int:
    cur_fn = None
    fn_slots = {}
    for line in open(path):
        m = re.match(r"^([A-Za-z_][\w.$]*):", line)
        if m and not line.startswith("."):
            cur_fn = m.group(1)
            continue
        m = re.match(r"^\s+([a-z]+)\s+(.*)$", line)
        if not m or cur_fn is None:
            continue
        mnem, rest = m.group(1), m.group(2)
        w = WMAP.get(mnem)
        if w is None:
            continue
        for off in re.findall(r"(-?\d+)\(%rbp\)", rest):
            fn_slots.setdefault(cur_fn, {}).setdefault(int(off), set()).add((w, mnem))
    total = 0
    for fn, slots in fn_slots.items():
        mixed = {o: ws for o, ws in slots.items() if len({w for w, _ in ws}) > 1}
        if mixed:
            total += len(mixed)
            print(f"{path}: {fn}:")
            for o in sorted(mixed):
                print(f"  {o}(%rbp): {sorted(mixed[o])}")
    return total


def main() -> None:
    grand = 0
    for p in sys.argv[1:]:
        grand += audit(p)
    print(f"mixed-width slots (per function): {grand}")


if __name__ == "__main__":
    main()
