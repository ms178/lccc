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


# Slots are addressed relative to whichever pointer the frame uses: `%rbp`
# when a frame pointer is kept (-O0, -Og, and any frame that needs one) and
# `%rsp` when it is omitted (-O1 and above, which is where the small-slot
# machinery is active). Matching only one of them silently reports an empty
# audit on every -O2 listing.
SLOT_RE = re.compile(r"(-?\d+)\(%r(?:bp|sp)\)")


def audit(path: str, verbose: bool = True) -> int:
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
        for off in SLOT_RE.findall(rest):
            fn_slots.setdefault(cur_fn, {}).setdefault(int(off), set()).add((w, mnem))
    total = 0
    for fn, slots in fn_slots.items():
        mixed = {o: ws for o, ws in slots.items() if len({w for w, _ in ws}) > 1}
        if mixed:
            total += len(mixed)
            if verbose:
                print(f"{path}: {fn}:")
                for o in sorted(mixed):
                    print(f"  {o}(%%rbp/%%rsp): {sorted(mixed[o])}")
    return total


def main() -> None:
    args = [a for a in sys.argv[1:]]
    gate = False
    if "--gate" in args:
        # CI mode: exit non-zero when any mixed-width slot is found, so the
        # check can actually fail a build instead of only ever printing.
        gate = True
        args.remove("--gate")
    quiet = "--quiet" in args
    if quiet:
        args.remove("--quiet")
    grand = 0
    for p in args:
        grand += audit(p, verbose=not quiet)
    print(f"mixed-width slots (per function): {grand}")
    if gate and grand:
        sys.exit(1)


if __name__ == "__main__":
    main()
