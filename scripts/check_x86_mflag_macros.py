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
        [--bases '-march=x86-64' ...] [--pairs] [--triples]

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

# `__CRC32__` is not in the generated table: GCC announces it with SSE4.2 and
# keeps it through `-mgeneral-regs-only` when SSE4.2 was live (see
# `TRIPLE_ALPHABET`), so it is compared explicitly.
MACROS = sorted({m for _, m in GEN.FEATURE_MACROS} | {m for _, m in GEN.EXTRA_MACROS}
                | {m for _, m, _, _ in GEN.PSEUDO} | {'__CRC32__'})
# `general-regs-only` has no negation; `cx16`'s macro is not ISA state GCC
# announces on -m32. Legacy AVX-512 names GCC 16 rejects are not modelled.
OPTIONS = sorted({GEN.OPTION_NAMES[v] for v, _ in GEN.FEATURE_MACROS}
                 | {o for _, _, o, _ in GEN.PSEUDO} | set(GEN.EXTRA_OPTIONS)
                 | set(GEN.EXTRA_OPTION_NAMES.values()))
# Deliberate divergences, each with its reason. AES/PCLMUL/GFNI used to be one
# (GCC announces them with the SSE register file off while rejecting their
# intrinsics there); lccc now matches GCC's announcement and keeps the SSE2
# requirement in the intrinsic gate instead, so that fork is gone and a
# regression on it fails this script again.
#
# Deliberate divergences: none. Every macro the oracle compares must match
# gcc-16.2 `-dM -E` exactly. `__SHA512__`, `__SM3__` and `__SM4__` are compared
# under `-mno-xsave` as well: GCC announces them there (measured in every
# flag order), and lccc matches. Only the SHA-512 intrinsic is withdrawn under
# `-mno-xsave`, because GCC rejects it and lccc refuses it with a diagnostic.
#
# Refused with a diagnostic, as documented: lccc has no AVX10 code generation.
REFUSED = {'avx10.1', 'avx10.2'}
# Ordered pairs that pin the one cascade the single-option matrix cannot see:
# XSAVE is an *option-layer* bit, so `-mno-xsave` has to withdraw the
# extensions whose state it saves while `-mxsave`/an implying option re-arms
# them, and the later spelling always wins.  `--pairs` covers this exhaustively
# but is far too slow for CI, so the interesting orderings are enumerated here
# and run on every base by default.
CASCADES = [
    ('-mno-xsave', '-mavx'),
    ('-mavx', '-mno-xsave'),
    ('-mno-xsave', '-mfma'),
    ('-mfma', '-mno-xsave'),
    ('-mno-xsave', '-mavx2'),
    ('-mavx2', '-mno-xsave'),
    ('-mno-xsave', '-mamx-tile'),
    ('-mamx-tile', '-mno-xsave'),
    ('-mamx-tile', '-mxsave'),
    ('-mno-xsave', '-mamx-int8'),
    ('-mamx-int8', '-mno-xsave'),
    ('-mno-xsave', '-mamx-bf16'),
    ('-mno-xsave', '-mavxifma'),
    ('-mavxifma', '-mno-xsave'),
    ('-mno-xsave', '-mavxneconvert'),
    ('-mno-xsave', '-msha512'),
    ('-msha512', '-mno-xsave'),
    ('-mno-xsave', '-maes'),
    ('-maes', '-mno-xsave'),
    ('-mno-xsave', '-mgfni'),
    ('-mno-sse2', '-maes'),
    ('-maes', '-mno-sse2'),
    ('-mno-sse', '-mpclmul'),
    ('-mno-sse', '-mgfni'),
]
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
    # An extension no table CPU enables (`-musermsr`) is still modelled when
    # its own option defines the macro: probe each option alone too, so the
    # oracle compares it instead of skipping it.
    probes += [['-m64', f'-m{option}'] for option in sorted(GEN.EXTRA_OPTION_NAMES.values())]
    with ThreadPoolExecutor(jobs) as pool:
        dumps = pool.map(lambda flags: macros(lccc, flags) or set(), probes)
        return frozenset(set().union(*dumps))


def spellings(option):
    yield f'-m{option}'
    if option != 'general-regs-only':
        yield f'-mno-{option}'


# Ordered triples over the options that decide the SSE4.2 ladder's sticky
# state: the POPCNT and CRC32 carry of `-mgeneral-regs-only` (GCC's
# `OPT_mgeneral_regs_only` consults the SSE4.2 bit at that point), the
# `-march=` reset, the SSE/AVX withdrawals that cascade into it, and the
# explicit MMX spellings. Pairs cannot reach these: the carry depends on which
# grant came BEFORE the denial, and on whether a later withdrawal happened.
TRIPLE_ALPHABET = [
    '-mavx', '-msse4.2', '-mabm', '-mpopcnt', '-mgeneral-regs-only', '-mno-avx',
    '-mno-sse4.2', '-mno-sse2', '-mno-sse', '-mno-popcnt', '-mno-avx2', '-mavx2',
    '-mmmx', '-mno-mmx', '-march=x86-64', '-march=haswell', '-msse4.1',
    '-mno-sse4.1', '-mxsave', '-mno-xsave', '-mno-abm',
]


def cases(bases, pairs, triples=False):
    for base in bases:
        flags = base.split()
        for option in OPTIONS:
            for spelling in spellings(option):
                yield flags + [spelling]
        if triples:
            for combo in itertools.product(TRIPLE_ALPHABET, repeat=3):
                yield flags + list(combo)
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

    for base in bases:
        for cascade in CASCADES:
            yield base.split() + list(cascade)


def main():
    parser = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    parser.add_argument('lccc')
    parser.add_argument('--gcc', default=os.environ.get('GCC', 'gcc-16'))
    parser.add_argument('--bases', nargs='+',
                        default=['-march=x86-64', '-march=haswell', '-march=icelake-server',
                                 '-march=clearwaterforest', '-march=sapphirerapids', '-m32'])
    parser.add_argument('--pairs', action='store_true', help='also every ordered pair')
    parser.add_argument('--triples', action='store_true',
                        help='also every ordered triple over TRIPLE_ALPHABET (~9.3k per base)')
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
        extra = got - want
        missing = want - got
        return flags, (sorted(missing), sorted(extra)), not extra and not missing

    total = failed = 0
    with ThreadPoolExecutor(args.j) as pool:
        for flags, detail, ok in pool.map(compare, cases(args.bases, args.pairs, args.triples)):
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
