#!/usr/bin/env python3
"""Instruction lines per function in a `-S` dump: no directives, labels or
comments, the way every table in SCALAR-PREDICATE-FOLDS.md counts."""
import re
import sys

NAMES = ["pc64_and1", "par64ll", "par_u8", "par_u16", "pc_u8", "pc_u16",
         "pc32_and1", "np2c"]


def body(text, name):
    lines = text.splitlines()
    out, inside = [], False
    for line in lines:
        if re.match(rf"^{re.escape(name)}\s*:", line):
            inside = True
            continue
        if inside and re.match(r"^\s*\.size\b", line):
            break
        if inside:
            out.append(line)
    return [l for l in out
            if l.strip() and not l.strip().startswith((".", "#", ";"))
            and not re.match(r"^[A-Za-z_.$][A-Za-z0-9_.$]*:", l)]


for path in sys.argv[1:]:
    text = open(path).read()
    print(path, " ".join(f"{n}={len(body(text, n))}" for n in NAMES))
