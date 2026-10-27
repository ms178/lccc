#!/usr/bin/env python3
"""Differential: lccc's x86 ISA macros against GCC 16.2's for `-m` options.

Every ISA option `scripts/gen_x86_arch_table.py` models is compiled alone
(`-mNAME`, `-mno-NAME`) and in ordered pairs with every other option
(`-mA -mno-B`, `-mno-A -mB`, `-mA -mB`) on several `-march=` bases, and
the feature macros (`__AVX2__`, `__POPCNT__`, ... and the macro-only
extensions) of `lccc -dM -E` must equal GCC's. Only the macros lccc
models are compared: those it defines for some `-march=` CPU. A macro
for an extension without lccc intrinsics (`__FXSR__`) would send
`#ifdef` code into calls lccc cannot compile. The pairs exercise GCC's
implication graph in both directions and the command-line cancellation
of a later spelling of the same option.

    LC_ALL=C python3 scripts/check_x86_mflag_macros.py LCCC [--gcc gcc-16]
        [--bases '-march=x86-64' ...] [--pairs]

Exit status 1 lists every mismatch as `flags: missing [...] extra [...]`.
"""
import argparse
import importlib.util
import itertools
import os
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location('gen', ROOT / 'scripts/gen_x86_arch_table.py')
GEN = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(GEN)

MACROS = sorted({m for _, m in GEN.FEATURE_MACROS} | {m for _, m in GEN.EXTRA_MACROS}
                | {m for _, m, _, _ in GEN.PSEUDO})
# `general-regs-only` has no negation; `cx16`'s macro is not ISA state GCC
# announces on -m32. Legacy AVX-512 names GCC 16 rejects are not modelled.
OPTIONS = sorted({GEN.OPTION_NAMES[v] for v, _ in GEN.FEATURE_MACROS}
                 | {o for _, _, o, _ in GEN.PSEUDO} | set(GEN.EXTRA_OPTIONS)
                 | set(GEN.EXTRA_OPTION_NAMES.values()))
# Deliberate divergences, each with its reason (flags -> macros lccc omits):
# GCC keeps announcing AES, PCLMUL and GFNI with the SSE register file off
# but rejects their intrinsics there (they need SSE2): lccc's macros only
# promise intrinsics that compile.
FORKS = {'__AES__', '__PCLMUL__', '__GFNI__'}
SSE_OFF = ('-mno-sse', '-mno-sse2', '-mgeneral-regs-only')
# Refused with a diagnostic, as documented: lccc has no AVX10 code generation.
REFUSED = {'avx10.1', 'avx10.2'}
ENV = dict(os.environ, LC_ALL='C')


def macros(compiler, flags, tracked=frozenset(MACROS)):
    proc = subprocess.run([compiler, *flags, '-dM', '-E', '-x', 'c', '/dev/null'],
                          capture_output=True, text=True, env=ENV)
    if proc.returncode != 0:
        return None
    defined = {line.split()[1] for line in proc.stdout.splitlines() if line.startswith('#define ')}
    return defined & tracked


def modelled(lccc, jobs):
    """The tracked macros lccc defines for at least one table CPU."""
    table = (ROOT / 'src/backend/x86/arch_table.rs').read_text()
    names = [line.split('"')[1] for line in table.splitlines() if line.lstrip().startswith('row("')]
    probes = [[mode, f'-march={name}'] for name in names for mode in ('-m64', '-m32')]
    with ThreadPoolExecutor(jobs) as pool:
        dumps = pool.map(lambda flags: macros(lccc, flags) or set(), probes)
        return frozenset(set().union(*dumps))


def spellings(option):
    yield f'-m{option}'
    if option != 'general-regs-only':
        yield f'-mno-{option}'


def cases(bases, pairs):
    for base in bases:
        flags = base.split()
        for option in OPTIONS:
            for spelling in spellings(option):
                yield flags + [spelling]
        if not pairs:
            continue
        for a, b in itertools.permutations(OPTIONS, 2):
            for first in spellings(a):
                for second in spellings(b):
                    if first.startswith('-mno-') and second.startswith('-mno-'):
                        continue
                    yield flags + [first, second]
        for option in OPTIONS:
            if option != 'general-regs-only':
                yield flags + [f'-m{option}', f'-mno-{option}']
                yield flags + [f'-mno-{option}', f'-m{option}']


def main():
    parser = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    parser.add_argument('lccc')
    parser.add_argument('--gcc', default=os.environ.get('GCC', 'gcc-16'))
    parser.add_argument('--bases', nargs='+',
                        default=['-march=x86-64', '-march=haswell', '-march=icelake-server',
                                 '-march=clearwaterforest', '-march=sapphirerapids', '-m32'])
    parser.add_argument('--pairs', action='store_true', help='also every ordered pair')
    parser.add_argument('-j', type=int, default=2)
    args = parser.parse_args()

    tracked = modelled(args.lccc, args.j)
    print(f'not modelled by lccc, not compared: {sorted(set(MACROS) - tracked)}')

    # An option for an extension lccc does not model may be refused.
    unmodelled = {GEN.EXTRA_OPTION_NAMES[v] for v, m in GEN.EXTRA_MACROS if m not in tracked}
    unmodelled |= {GEN.OPTION_NAMES[v] for v, m in GEN.FEATURE_MACROS if m not in tracked}

    def compare(flags):
        want, got = macros(args.gcc, flags, tracked), macros(args.lccc, flags, tracked)
        if want is None:
            return flags, 'gcc-rejects', got is None
        if got is None:
            refused = any(f.removeprefix('-m').removeprefix('no-') in unmodelled | REFUSED for f in flags[-2:])
            return flags, 'lccc-rejects', refused
        if any(f in SSE_OFF for f in flags):
            want, got = want - FORKS, got - FORKS
        return flags, (sorted(want - got), sorted(got - want)), want == got

    total = failed = 0
    with ThreadPoolExecutor(args.j) as pool:
        for flags, detail, ok in pool.map(compare, cases(args.bases, args.pairs)):
            total += 1
            if ok:
                continue
            failed += 1
            if isinstance(detail, str):
                print(f'FAIL {" ".join(flags)}: {detail}')
            else:
                missing, extra = detail
                print(f'FAIL {" ".join(flags)}: missing {missing} extra {extra}')
    print(f'{total - failed} PASS / {failed} FAIL ({total} cases)')
    sys.exit(1 if failed else 0)


if __name__ == '__main__':
    main()
