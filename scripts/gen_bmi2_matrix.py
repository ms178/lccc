#!/usr/bin/env python3
"""PDEP/PEXT assembler matrix: exhaustive register triples and address axes.

Assembly-only: combinations writing ESP/RSP are intentionally never executed.
The two address axes independently cover every base/index/scale/displacement
and every source/destination pair for representative memory encoding classes.
This is not an exhaustive Cartesian product of all memory operand fields.
"""
import argparse
from itertools import product
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
REG32 = ['eax', 'ecx', 'edx', 'ebx', 'esp', 'ebp', 'esi', 'edi']


def matrix(bits):
    rows = [';;; pdep-pext-register-triples', '.text']
    address = REG32 if bits == 32 else ['rax', 'rcx', 'rdx', 'rbx', 'rsp', 'rbp', 'rsi', 'rdi'] + [f'r{i}' for i in range(8, 16)]
    widths = [('l', REG32 + ([] if bits == 32 else [f'r{i}d' for i in range(8, 16)]))]
    if bits == 64:
        widths.append(('q', address))
    for suffix, regs in widths:
        for instruction, mask, value, dest in product(('pdep', 'pext'), regs, regs, regs):
            rows.append(f'{instruction}{suffix} %{mask},%{value},%{dest}')
    rows.extend(['', ';;; pdep-pext-address-fields', '.text'])
    for suffix, regs in widths:
        for base, index, scale, disp in product(address, [None] + [reg for reg in address if reg not in ('esp', 'rsp')], (1, 2, 4, 8), (0, 1, -1, 127, 128, -128, -129, 0x12345678)):
            memory = f'{disp}(%{base}' + (f',%{index},{scale})' if index else ')')
            for instruction in ('pdep', 'pext'):
                rows.append(f'{instruction}{suffix} {memory},%{regs[1]},%{regs[2]}')
        for value, dest in product(regs, regs):
            for memory in (f'(%{address[0]})', f'128(%{address[4]})', f'-129(%{address[5]})', f'127(%{address[3]},%{address[-1]},4)'):
                for instruction in ('pdep', 'pext'):
                    rows.append(f'{instruction}{suffix} {memory},%{value},%{dest}')
    return '\n'.join(rows) + '\n'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    for bits, directory in ((32, 'i686'), (64, '')):
        path = ROOT / 'tests/asm-diff' / directory / 'bmi2-matrix.casefile'
        text = matrix(bits)
        if args.check:
            if path.read_text() != text:
                raise SystemExit(f'{path}: stale generated matrix')
        else:
            path.write_text(text)
        print(f'{bits}-bit: {sum(line.startswith(("pdep", "pext")) for line in text.splitlines())} instructions')


if __name__ == '__main__':
    main()
