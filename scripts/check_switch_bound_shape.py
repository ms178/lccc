#!/usr/bin/env python3
"""Check emitted dispatch guards, independent of operand width/immediate spelling.

The runtime matrix checks semantics; emitter unit tests check normalization,
holes and code models. This integration control proves the disable flag reaches
code generation, without adding a production-only debug counter.
"""
from pathlib import Path
import re
import sys


def dispatch_prefix(text, name="dispatch16"):
    body = text.split(name + ":", 1)
    if len(body) != 2:
        raise ValueError(f"missing function {name}")
    instructions = []
    for raw in body[1].splitlines():
        line = raw.split("#", 1)[0].strip()
        if line.startswith(".size") or line == ".cfi_endproc":
            break
        if not line or line.startswith(".") or line.endswith(":"):
            continue
        fields = line.split(None, 1)
        op, args = fields[0], fields[1] if len(fields) > 1 else ""
        if op in ("jmp", "jmpq") and args.startswith("*"):
            return instructions
        instructions.append(op)
    raise ValueError("missing indirect switch dispatch")


def check(proved, unproved):
    for text, guards in ((proved, 0), (unproved, 1)):
        ops = dispatch_prefix(text)
        compares = sum(bool(re.fullmatch(r"cmp[bwlq]?", op)) for op in ops)
        branches = sum(op.startswith("j") and op not in ("jmp", "jmpq") for op in ops)
        if (compares, branches) != (guards, guards):
            raise ValueError(f"expected {guards} guard(s), got {compares} comparisons / {branches} branches: {ops}")


if __name__ == "__main__":
    check(*(Path(p).read_text() for p in sys.argv[1:]))
