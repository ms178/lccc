#!/usr/bin/env python3
"""Direct `-c` compiles must contain the same instructions as `-S` + assemble.

Only alignment padding may differ: direct compiles carry the private
`.lccc_tight_loop` marker, resolved by the integrated assembler from exact
encoded spans. The marker once poisoned every peephole analysis in functions
with a tight loop (FP liveness returned unknown on the opaque directive), so
production `-c` objects silently lost memory-operand folding and slot
forwarding that `-S` output kept: nbody's inner loop had ten folded operands
via `-S` and one via `-c`. Compare normalized instruction streams. Assembling
the `-S` text with GNU as also proves that text is portable assembly.
"""
import argparse
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[2]
SOURCES = [
    ROOT / 'tests/benchmark/programs/nbody.c',
    ROOT / 'tests/benchmark/programs/expat_xml_scan.c',
    # AVX2 counting reduction: its -S text once spelled `vextracti128` with a
    # YMM destination that only the integrated assembler accepted.
    ROOT / 'tests/benchmark/programs/zlib_ng_adler32.c',
    ROOT / 'tests/regression/frame_gep_direct.c',
]
FLAGS = ['-O2', '-march=x86-64-v3', '-fwrapv', '-ffp-contract=off']


def run(cmd):
    result = subprocess.run([str(c) for c in cmd], capture_output=True, text=True)
    if result.returncode:
        sys.exit(f'command failed: {" ".join(map(str, cmd))}\n{result.stderr}')
    return result.stdout


def instructions(obj):
    """Instruction text without addresses, branch-target addresses or NOPs."""
    out = []
    for line in run(['objdump', '-d', '--no-show-raw-insn', obj]).splitlines():
        match = re.match(r'\s+[0-9a-f]+:\s+(.*)', line)
        if not match:
            if re.match(r'[0-9a-f]+ <.*>:$', line):
                out.append(line.split()[1])
            continue
        text = re.sub(r'\s+', ' ', match.group(1)).strip()
        if re.match(r'(data16 |cs )*nop', text) or text == 'xchg %ax,%ax':
            continue
        text = re.sub(r'[0-9a-f]+ <([^>+]*)(\+0x[0-9a-f]+)?>', r'<\1>', text)
        # RIP-relative displacements to symbols shift with alignment padding.
        text = re.sub(r'-?0x[0-9a-f]+\(%rip\)(.*)# <', r'REL(%rip)\1# <', text)
        out.append(canonical_commutative(text))
    return out


COMMUTATIVE = re.compile(r'v(p(add|mull|and|or|xor|min|max|cmpeq)[a-z]*|(add|mul|and|or|xor|min|max)p[sd])$')


def canonical_commutative(text):
    """Order the two sources of a commutative VEX op: an encoder may swap them
    (to keep a high register in VEX.vvvv for the 2-byte prefix) without any
    semantic change."""
    mnemonic, _, operands = text.partition(' ')
    parts = operands.split(',')
    if (COMMUTATIVE.match(mnemonic) or mnemonic == 'vpmaddwd') and len(parts) == 3 and all(p.startswith('%') for p in parts):
        parts[:2] = sorted(parts[:2])
        return mnemonic + ' ' + ','.join(parts)
    return text


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--lccc', default=os.environ.get('CCC', str(ROOT / 'target/fastbuild/lccc')))
    parser.add_argument('--assembler', default=os.environ.get('AS', 'as'))
    args = parser.parse_args()
    failures = []
    with tempfile.TemporaryDirectory() as work:
        work = Path(work)
        for source in SOURCES:
            direct, asm, via_s = work / 'direct.o', work / 'out.s', work / 'via_s.o'
            run([args.lccc, *FLAGS, '-c', source, '-o', direct])
            run([args.lccc, *FLAGS, '-S', source, '-o', asm])
            run([args.assembler, asm, '-o', via_s])
            a, b = instructions(direct), instructions(via_s)
            if a != b:
                first = next((i for i, (x, y) in enumerate(zip(a, b)) if x != y), min(len(a), len(b)))
                failures.append(f'{source.name}: -c {len(a)} vs -S {len(b)} instructions; '
                                f'first difference at {first}: {a[first:first+1]} vs {b[first:first+1]}')
    if failures:
        sys.exit('FAIL: direct and assembly-mode code differ:\n' + '\n'.join(failures))
    print(f'check_direct_assembly_parity: PASS ({len(SOURCES)} sources, identical instruction streams)')


if __name__ == '__main__':
    main()
