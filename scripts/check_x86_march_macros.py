#!/usr/bin/env python3
"""Differential: lccc's predefined macros for every GCC 16.2 x86 CPU name
against GCC's own (`-dM -E`), on -m64 (x86-64 CPUs) and -m32 (all CPUs but
the AVX-512 ones, which lccc refuses on i686).

Usage: check_x86_march_macros.py LCCC [GCC]   (GCC defaults to gcc-16)

Per name, three exact comparisons:
  * ISA macros (the uppercase macros that vary across GCC's CPUs:
    `__AVX2__`, `__SSE__`, ...) that lccc models, i.e. defines for at least
    one CPU. The rest
    (`__XSAVE__`, `__ADX__`, ...) are listed once as unmodeled: lccc never
    defines them, so it never claims an instruction it cannot emit;
  * CPU-name macros (`__haswell__`, `__core_avx2`, `__tune_haswell__`);
  * `-mtune=NAME` tuning macros, where GCC accepts the name for -mtune=.

Documented differences, excluded and printed: on -m32, lccc always defines
`__i686`/`__i686__` (the i686-linux-gnu-gcc spelling upstream chose), and
its `__SSE_MATH__`/`__SSE2_MATH__`/`__FLT_EVAL_METHOD__` state its own
scalar FP codegen (F32 in SSE with SSE2, F64 on x87 with declared-type
rounding), not GCC's x87 default. `-march=diamondrapids`/`novalake` are an
explicit lccc error (unimplemented AVX10/APX lowering).
"""
import os
import re
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
TABLE = os.path.join(ROOT, 'src', 'backend', 'x86', 'arch_table.rs')
ENV = {**os.environ, 'LC_ALL': 'C', 'LANG': 'C'}
REFUSED = {'diamondrapids', 'novalake'}
ISA = re.compile(r'__[A-Z][A-Z0-9_]*__')
CPU = re.compile(r'__[a-z][a-z0-9_]*')
FORKS = {
    '-m32': {'__i686', '__i686__', '__SSE_MATH__', '__SSE2_MATH__'},
    '-m64': set(),
}
NOT_ISA = {'__SSE_MATH__', '__SSE2_MATH__', '__MMX_WITH_SSE__', '__SEG_FS', '__SEG_GS'}


def rows():
    out = []
    for line in open(TABLE):
        m = re.match(r'\s*row\("([^"]+)",\s+([YN]),', line)
        if m:
            out.append((m[1], m[2] == 'Y'))
    if len(out) < 100:
        sys.exit(f'{TABLE}: only {len(out)} rows parsed')
    return out


def dump(cc, args):
    proc = subprocess.run([cc, *args, '-dM', '-E', '-x', 'c', os.devnull],
                          capture_output=True, text=True, env=ENV)
    if proc.returncode != 0:
        return None
    return {line.split()[1] for line in proc.stdout.splitlines() if line.startswith('#define ')}


def main(argv):
    if len(argv) not in (1, 2):
        sys.exit(__doc__)
    lccc, gcc = argv[0], argv[1] if len(argv) == 2 else 'gcc-16'
    table = rows()
    cases = [('-m64', n) for n, x64 in table if x64] + [('-m32', n) for n, _ in table]
    gcc_dumps, lccc_dumps = {}, {}
    for mode, name in cases:
        gcc_dumps[mode, name] = dump(gcc, [mode, f'-march={name}'])
        lccc_dumps[mode, name] = dump(lccc, [mode, f'-march={name}'])
    # ISA macros: the uppercase ones that vary across GCC's CPUs of a mode;
    # lccc models those it defines for at least one CPU.
    gcc_isa = set()
    for mode in ('-m64', '-m32'):
        dumps = [d for k, d in gcc_dumps.items() if k[0] == mode and d]
        common = set.intersection(*dumps)
        gcc_isa |= {m for d in dumps for m in d - common if ISA.fullmatch(m)}
    gcc_isa = {m for m in gcc_isa - NOT_ISA if not m.startswith('__STDC_')}
    lccc_defined = {m for d in lccc_dumps.values() if d for m in d}
    modeled = gcc_isa & lccc_defined
    # CPU-name macros: GCC's lowercase macros that vary across its CPUs.
    cpu_names = set()
    for mode in ('-m64', '-m32'):
        dumps = [{m for m in d if CPU.fullmatch(m)} for k, d in gcc_dumps.items() if k[0] == mode and d]
        cpu_names |= set.union(*dumps) - set.intersection(*dumps)
    fails = passes = 0
    for mode, name in cases:
        g, l = gcc_dumps[mode, name], lccc_dumps[mode, name]
        label = f'{mode} -march={name}'
        if g is None:
            print(f'FAIL {label}: GCC rejects a table name')
            fails += 1
            continue
        if l is None:
            if name in REFUSED:
                print(f'PASS {label}: refused (AVX10/APX), as documented')
                passes += 1
            else:
                print(f'FAIL {label}: lccc rejects it')
                fails += 1
            continue
        problems = []
        for kind, keep in (('ISA', lambda m: m in modeled), ('CPU', lambda m: m in cpu_names)):
            gs = {m for m in g if keep(m)} - FORKS[mode]
            ls = {m for m in l if keep(m)} - FORKS[mode]
            if gs != ls:
                problems.append(f'{kind} lccc-only {sorted(ls - gs)} gcc-only {sorted(gs - ls)}')
        gt = dump(gcc, [mode, f'-mtune={name}'])
        if gt is not None:
            lt = dump(lccc, [mode, f'-mtune={name}']) or set()
            gs = {m for m in gt if m.startswith('__tune_')}
            ls = {m for m in lt if m.startswith('__tune_')}
            if gs != ls:
                problems.append(f'-mtune lccc-only {sorted(ls - gs)} gcc-only {sorted(gs - ls)}')
        if problems:
            print(f'FAIL {label}: ' + '; '.join(problems))
            fails += 1
        else:
            passes += 1
    print(f'unmodeled GCC ISA macros (never defined by lccc): {" ".join(sorted(gcc_isa - modeled))}')
    print(f'{passes} PASS, {fails} FAIL of {len(cases)} (CPU name, mode) cases')
    sys.exit(1 if fails else 0)


if __name__ == '__main__':
    main(sys.argv[1:])
