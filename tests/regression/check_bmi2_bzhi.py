#!/usr/bin/env python3
"""BMI2 `_bzhi_u32`/`_bzhi_u64` lower to one `bzhi` and match GCC exactly.

`tests/regression/bmi2_bzhi.c` prints `bzhi` results for every combination of
edge-case sources and indices (bits 7:0 select, 0/31/32/63/64/255/256+, high
index bits set) through three paths: opaque operands (the backend
instruction), a constant index (the simplifier's `and`/copy/zero rewrite) and
two constants (the constant folder).

Contracts:
  * with `-march=x86-64-v3` the opaque helpers are a single `bzhil`/`bzhiq`
    each, with no compare, branch or conditional move left from the old
    software emulation;
  * output equals GCC 16's (`-march=x86-64-v3`), and with `-march=x86-64`
    (no BMI2: the header's portable fallback, no `bzhi` emitted);
  * calling `__builtin_ia32_bzhi_di` without BMI2 is a compile error naming
    the extension, as in GCC (negative control for the permission check).

The runtime half needs a BMI2 CPU; without one it reports SKIP for that half
explicitly while still enforcing the assembly and diagnostic contracts.
"""
import argparse
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / 'tests/regression/bmi2_bzhi.c'
DIRECT = ROOT / 'tests/regression/fixtures/bmi2-bzhi/bmi2_bzhi_direct.c'
HELPERS = {'vbzhi64': 'bzhiq', 'vbzhi32': 'bzhil'}
LEFTOVER = re.compile(r'^(?:cmp|test|j[a-z]+|cmov|set)[a-z]*\s')


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


def selftest():
    asm = ('vbzhi64:\n\t.cfi_startproc\n\tbzhiq %rsi, %rdi, %rax\n\tret\n\t.size vbzhi64, .-vbzhi64\n'
           'vbzhi32:\n.LBB1:\n\tandl $255, %esi\n\tcmpl $32, %esi\n\tjae .LBB2\n\tret\n')
    parsed = bodies(asm)
    assert parsed['vbzhi64'] == ['bzhiq %rsi, %rdi, %rax', 'ret']
    assert [l for l in parsed['vbzhi32'] if LEFTOVER.match(l)] == ['cmpl $32, %esi', 'jae .LBB2']
    assert not LEFTOVER.match('bzhiq %rsi, %rdi, %rax') and not LEFTOVER.match('ret')


def has_bmi2():
    try:
        flags = Path('/proc/cpuinfo').read_text()
    except OSError:
        return False
    return re.search(r'^flags\s*:.*\bbmi2\b', flags, re.M) is not None


def run(cmd, check=True):
    result = subprocess.run(cmd, capture_output=True, text=True)
    if check and result.returncode:
        sys.exit(f'command failed: {" ".join(map(str, cmd))}\n{result.stderr}')
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--lccc', default=os.environ.get('CCC', str(ROOT / 'target/fastbuild/lccc')))
    parser.add_argument('--gcc', default=os.environ.get('GCC', 'gcc'))
    args = parser.parse_args()
    selftest()
    failures = []
    with tempfile.TemporaryDirectory() as work:
        work = Path(work)
        for march, want_bzhi in (('x86-64-v3', True), ('x86-64', False)):
            asm = work / f'{march}.s'
            run([args.lccc, '-O2', f'-march={march}', '-S', SOURCE, '-o', asm])
            parsed = bodies(asm.read_text())
            for name, mnem in HELPERS.items():
                body = parsed.get(name)
                if body is None:
                    sys.exit(f'FAIL: function {name} missing from the -march={march} assembly')
                count = sum(1 for line in body if line.startswith(mnem + ' '))
                if want_bzhi:
                    if count != 1:
                        failures.append(f'{name} (-march={march}): {count} `{mnem}`, want 1: {body}')
                    leftovers = [line for line in body if LEFTOVER.match(line)]
                    if leftovers:
                        failures.append(f'{name} (-march={march}): emulation left {leftovers}')
                elif any(line.startswith('bzhi') for line in body):
                    failures.append(f'{name} (-march={march}): `bzhi` without BMI2: {body}')
        direct = run([args.lccc, '-O2', '-march=x86-64', '-c', DIRECT, '-o', work / 'direct.o'],
                     check=False)
        if direct.returncode == 0 or 'bmi2' not in direct.stderr:
            failures.append('__builtin_ia32_bzhi_di without BMI2 was not rejected with a bmi2 '
                            f'diagnostic (rc={direct.returncode}): {direct.stderr.strip()!r}')
        runtime = 'SKIP (CPU has no BMI2)'
        if has_bmi2():
            run([args.gcc, '-O2', '-march=x86-64-v3', SOURCE, '-o', work / 'gcc'])
            reference = run([work / 'gcc']).stdout
            cases = reference.count('\n')
            for march in ('x86-64-v3', 'x86-64'):
                exe = work / f'lccc-{march}'
                run([args.lccc, '-O2', f'-march={march}', SOURCE, '-o', exe])
                mine = run([exe]).stdout
                if mine != reference:
                    diff = [f'{a!r} != {b!r}' for a, b in
                            zip(mine.splitlines(), reference.splitlines()) if a != b][:5]
                    failures.append(f'output (-march={march}) differs from GCC: {diff}')
            runtime = f'{cases} results equal GCC at -march=x86-64-v3 and -march=x86-64'
    if failures:
        sys.exit('FAIL:\n  ' + '\n  '.join(failures))
    print(f'check_bmi2_bzhi: PASS (one bzhi per helper with BMI2, none without; '
          f'builtin rejected without BMI2; runtime: {runtime})')


if __name__ == '__main__':
    main()
