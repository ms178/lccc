#!/usr/bin/env python3
"""Masks that only feed x86-64 shift counts must not be materialized.

`tests/regression/shift_count_demand.c` holds shift counts masked in C
(zstd's `BIT_lookBits` shape and variants). x86 shifts mask the count in
hardware (6 bits for 64-bit forms, 5 bits for 32-bit forms), so the five
`drop` functions must contain no `and` on a count, while the three `keep`
functions must keep theirs: a 64-bit shift reads bit 5 that `& 31` clears,
and a stored or added masked count is observable. `look_bits` and
`high_bits` must also lose the constant subtract (`~nb` / `-n`).

Negative control: `CCC_NO_SHIFT_COUNT_DEMAND=1` must bring the masks back
(the detector sees the defect and the switch reaches the pass). Program
output must equal GCC's with and without the rewrite.
"""
import argparse
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / 'tests/regression/shift_count_demand.c'
SWITCH = 'CCC_NO_SHIFT_COUNT_DEMAND'
DROP = ('look_bits', 'high_bits', 'sar_wide_mask', 'shl32', 'non_congruent')
KEEP = ('keep_narrow_mask', 'keep_escaping', 'keep_arith')
NO_SUBTRACT = ('look_bits', 'high_bits')
MASK = re.compile(r'^\s*and[lq]\s+\$(?:31|63|127),\s*%\w+\s*$')
# A constant subtract materialized for the count: `movl $63, r` / `$64`.
SUB_CONST = re.compile(r'^\s*mov[lq]\s+\$(?:63|64),\s*%\w+\s*$')


def bodies(asm):
    """Map function name -> instruction lines (labels and directives removed)."""
    out, current = {}, None
    for line in asm.splitlines():
        label = re.fullmatch(r'([A-Za-z_]\w*):', line.strip())
        if label and not line.startswith(('\t', ' ')) and not label.group(1).startswith('.'):
            current = label.group(1)
            out[current] = []
            continue
        if current is None:
            continue
        text = line.strip()
        if text.startswith('.size') or text.startswith('.cfi_endproc'):
            current = None
            continue
        if text and not text.startswith(('.', '#')) and not text.endswith(':'):
            out[current].append(text)
    return out


def masks(body):
    return [line for line in body if MASK.match(line)]


def sub_consts(body):
    return [line for line in body if SUB_CONST.match(line)]


def selftest():
    asm = ('look_bits:\n\t.cfi_startproc\n\tandl $63, %esi\n\tmovl $63, %eax\n\tret\n'
           '\t.cfi_endproc\n\t.size look_bits, .-look_bits\n'
           'high_bits:\n.LBB1:\n\tnegl %esi\n\tshrxq %rsi, %rdi, %rax\n\tret\n\t.size high_bits, .-high_bits\n')
    parsed = bodies(asm)
    assert masks(parsed['look_bits']) == ['andl $63, %esi']
    assert sub_consts(parsed['look_bits']) == ['movl $63, %eax']
    assert not masks(parsed['high_bits']) and not sub_consts(parsed['high_bits'])
    assert not MASK.match('andl $63, (%rdi)') and not MASK.match('andq $-64, %rsp')


def switched(disabled):
    env = {k: v for k, v in os.environ.items() if k != SWITCH}
    if disabled:
        env[SWITCH] = '1'
    return env


def run(cmd, env=None):
    result = subprocess.run(cmd, capture_output=True, text=True, env=env)
    if result.returncode:
        sys.exit(f'command failed: {" ".join(map(str, cmd))}\n{result.stderr}')
    return result.stdout


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--lccc', default=os.environ.get('CCC', str(ROOT / 'target/fastbuild/lccc')))
    parser.add_argument('--gcc', default=os.environ.get('GCC', 'gcc'))
    args = parser.parse_args()
    selftest()
    failures = []
    with tempfile.TemporaryDirectory() as work:
        work = Path(work)
        compiled = {}
        for disabled in (False, True):
            asm = work / f'shift-{int(disabled)}.s'
            run([args.lccc, '-O2', '-march=x86-64-v3', '-S', SOURCE, '-o', asm], env=switched(disabled))
            compiled[disabled] = bodies(asm.read_text())
        on, off = compiled[False], compiled[True]
        for name in DROP + KEEP:
            if name not in on or name not in off:
                sys.exit(f'FAIL: function {name} missing from the assembly')
        for name in DROP:
            if masks(on[name]):
                failures.append(f'{name}: redundant count mask {masks(on[name])}')
            if not masks(off[name]):
                failures.append(f'{name}: negative control ({SWITCH}=1) shows no mask, '
                                'so the fixture no longer exercises the rewrite')
        for name in NO_SUBTRACT:
            if sub_consts(on[name]):
                failures.append(f'{name}: constant subtract kept {sub_consts(on[name])}')
            if not sub_consts(off[name]):
                failures.append(f'{name}: negative control shows no constant subtract')
        for name in KEEP:
            if not masks(on[name]):
                failures.append(f'{name}: observable mask was removed')
        run([args.gcc, '-O2', SOURCE, '-o', work / 'gcc'])
        reference = run([work / 'gcc'])
        for disabled in (False, True):
            exe = work / f'lccc-{int(disabled)}'
            run([args.lccc, '-O2', '-march=x86-64-v3', SOURCE, '-o', exe], env=switched(disabled))
            mine = run([exe])
            if mine != reference:
                failures.append(f'output ({SWITCH}={int(disabled)}) {mine!r} != GCC {reference!r}')
        # Baseline ISA too: legacy `shl %cl` lowering instead of BMI2 shlx.
        exe = work / 'lccc-x86-64'
        run([args.lccc, '-O2', '-march=x86-64', SOURCE, '-o', exe], env=switched(False))
        mine = run([exe])
        if mine != reference:
            failures.append(f'output (-march=x86-64) {mine!r} != GCC {reference!r}')
    if failures:
        sys.exit('FAIL:\n  ' + '\n  '.join(failures))
    print(f'check_shift_count_demand: PASS ({len(DROP)} masks dropped, {len(KEEP)} kept, '
          'negative control fires, GCC output matches with and without the rewrite and on '
          'baseline x86-64)')


if __name__ == '__main__':
    main()
