#!/usr/bin/env python3
"""Per-function instruction and stack-reference counts of a GAS .s file.

usage: asm_fn_counts.py A.s [B.s]
With one file, prints `name insns stackrefs` per function; with two, prints
the functions that differ and the totals. Stack references are operands
addressed off %rsp or %rbp.

Every non-blank line inside a function that is not a label, a directive or a
comment is an instruction, whatever its indentation: LCCC emits some
instructions (conditional jumps among them) at column 0, and counting only
indented lines would silently drop them from one compiler's side.
"""
import re
import sys

LABEL = re.compile(r'^[A-Za-z_.$][\w.$]*:\s*(#.*)?$')


def load(path):
    text = open(path).read()
    out = {}
    for m in re.finditer(r'^([A-Za-z_][\w.$]*):[^\n]*\n(.*?)\n\s*\.size\s+\1\b', text, re.S | re.M):
        ins = []
        for line in m.group(2).splitlines():
            s = line.split('#', 1)[0].strip()
            if s and not s.startswith('.') and not LABEL.match(s):
                ins.append(s)
        out[m.group(1)] = (len(ins), sum(('(%rsp)' in l) or ('(%rbp)' in l) for l in ins))
    return out


def main():
    a = load(sys.argv[1])
    if len(sys.argv) == 2:
        for f, (n, s) in sorted(a.items()):
            print(f'{f} {n} {s}')
        return
    b = load(sys.argv[2])
    ta = [0, 0]
    tb = [0, 0]
    common = sorted(set(a) & set(b))
    for f in common:
        ta[0] += a[f][0]; ta[1] += a[f][1]; tb[0] += b[f][0]; tb[1] += b[f][1]
        if a[f] != b[f]:
            print(f'{f}: {a[f][0]}/{a[f][1]} -> {b[f][0]}/{b[f][1]}')
    print(f'TOTAL insns {ta[0]} -> {tb[0]}  stackrefs {ta[1]} -> {tb[1]}  ({len(common)} functions)')


if __name__ == '__main__':
    main()
