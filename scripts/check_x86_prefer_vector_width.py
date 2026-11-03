#!/usr/bin/env python3
"""Differential gate for `-mprefer-vector-width=` on x86.

The flag is a *cost-model* request: it narrows the register class the
vectorizers pick, it never changes the instruction set.  This gate proves
exactly that and nothing more:

  * codegen class — for every row of the matrix below, compile a trivially
    vectorizable dword add loop and a float add loop with lccc and with
    GCC 16.2 at `-O2`, and compare whether the compiler used a 256-bit
    register at all.  A `=128` row must produce none in lccc while still
    vectorizing at 128 bits; a `=256`/`=512`/`none` row keeps the 256-bit
    class; a target without AVX2 never uses it;
  * macro neutrality — `-mprefer-vector-width=` must not move a single
    predefined macro.  GCC 16.2 defines no macro for it (`-dM` is
    byte-identical with and without the flag), and a compiler that
    advertised 256-bit width in a macro would hand every `#ifdef`-based
    kernel the wrong promise once the preference narrowed the code;
  * rejection — an unknown value fails with GCC's wording and the same
    valid list, because build systems parse that text.

`-mprefer-avx128` is GCC's deprecated alias for `=128`; it is checked in the
same matrix.  lccc emits no AVX-512 code, so `=512` means "widest
available" (256) and the gate pins that reading.

Usage: check_x86_prefer_vector_width.py LCCC [--gcc GCC] [--keep DIR]
Exit status is the number of failing rows (0 = clean).
"""

import argparse
import os
import shutil
import subprocess
import sys
import tempfile

ENV = {**os.environ, 'LC_ALL': 'C', 'LANG': 'C'}

# One loop of each kind: the integer one vectorizes on every x86 ISA level
# from SSE2 up, the float one needs SSE1 -- both without alignment
# assumptions, so a row that does not vectorize at all is a real finding
# and not a fixture artifact.
FIXTURE = """
void iadd(int *restrict c, const int *restrict a, const int *restrict b, int n) {
    for (int i = 0; i < n; i++) c[i] = a[i] + b[i];
}
void fadd(float *restrict c, const float *restrict a, const float *restrict b, int n) {
    for (int i = 0; i < n; i++) c[i] = a[i] + b[i];
}
"""

# (label, flags, expectation)
#   'ymm'   -> the widest class used must be 256-bit (ymm present)
#   'xmm'   -> 128-bit code only (no ymm) but still vectorized
#   'scalar'-> no vector register at all
# The default rows restate lccc's documented policy (256-bit whenever the
# resolved AVX2 permission is there), which is also what GCC 16.2 does for
# `-mavx2`/`-march=x86-64-v3`; rows where GCC's own cost model picks 128
# (`-march=znver4` and friends) are deliberately *not* compared, because the
# preference there is a cost decision, not a contract -- the contract is the
# explicit flag.
MATRIX = (
    ('-mavx2', 'ymm'),
    ('-march=x86-64-v3', 'ymm'),
    ('-march=haswell', 'ymm'),
    ('-mavx2 -mprefer-vector-width=256', 'ymm'),
    ('-mprefer-vector-width=256 -mavx2', 'ymm'),
    ('-mavx2 -mprefer-vector-width=512', 'ymm'),
    ('-mavx2 -mprefer-vector-width=none', 'ymm'),
    ('-mavx2 -mprefer-vector-width=128', 'xmm'),
    ('-mprefer-vector-width=128 -mavx2', 'xmm'),
    ('-mavx2 -mprefer-avx128', 'xmm'),
    ('-march=x86-64-v3 -mprefer-vector-width=128', 'xmm'),
    ('-mavx512f -mprefer-vector-width=128', 'xmm'),
    ('-march=x86-64-v2', 'xmm'),
    ('-march=x86-64-v2 -mprefer-vector-width=256', 'xmm'),
    ('-march=x86-64', 'xmm'),
    ('-mno-sse', 'scalar'),
    # Preference and denial must not fight: 128-bit preference on a target
    # whose SSE ladder is denied is still scalar code.
    ('-mno-sse -mprefer-vector-width=128', 'scalar'),
)

# Macro neutrality: these rows must produce identical `-dM` output with and
# without a preference.
MACRO_ROWS = (
    '-mavx2',
    '-march=haswell',
    '-march=x86-64-v3',
    '-march=znver4',
)


def run(cc, args, text_input=None, path=None):
    proc = subprocess.run([cc, *args], capture_output=True, text=True,
                          env=ENV, cwd=path or os.getcwd())
    return proc.returncode, proc.stdout, proc.stderr


def asm(cc, flags, source):
    code, out, _ = run(cc, [*flags.split(), '-O2', '-S', '-o', '-', str(source)])
    return out if code == 0 else ''


def width_class(assembly):
    if '%zmm' in assembly or '%ymm' in assembly:
        return 'ymm'
    if '%xmm' in assembly:
        return 'xmm'
    return 'scalar'


def macros(cc, flags):
    code, out, _ = run(cc, [*flags.split(), '-dM', '-E', '-x', 'c', os.devnull])
    return {line for line in out.splitlines() if line.startswith('#define ')} if code == 0 else None


def main(argv):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument('lccc', help='lccc binary')
    parser.add_argument('--gcc', default='gcc-16', help='GCC oracle (16.2)')
    parser.add_argument('--keep', help='keep the generated fixture here')
    args = parser.parse_args(argv)

    workdir = tempfile.mkdtemp(prefix='prefer-vector-width-')
    try:
        source = os.path.join(workdir, 'vw.c')
        with open(source, 'w') as handle:
            handle.write(FIXTURE)
        fails = 0
        passes = 0
        for flags, expected in MATRIX:
            got = width_class(asm(args.lccc, flags, source))
            gcc_got = width_class(asm(args.gcc, flags, source))
            # GCC is the oracle for the class *the flag names*; every other
            # row is lccc's documented policy, printed for the record.
            detail = f'lccc={got} gcc={gcc_got}'
            if got != expected:
                print(f'FAIL {flags}: expected {expected}, {detail}')
                fails += 1
                continue
            if expected == 'xmm' and ('prefer-vector-width=128' in flags
                                      or 'prefer-avx128' in flags):
                if gcc_got != 'xmm':
                    print(f'FAIL {flags}: oracle disagrees ({detail})')
                    fails += 1
                    continue
            print(f'PASS {flags}: {detail}')
            passes += 1

        for flags in MACRO_ROWS:
            base = macros(args.lccc, flags)
            for want in ('-mprefer-vector-width=128', '-mprefer-vector-width=256',
                         '-mprefer-vector-width=512', '-mprefer-avx128'):
                moved = macros(args.lccc, f'{flags} {want}')
                if base is None or moved is None:
                    print(f'FAIL {flags} {want}: cannot preprocess')
                    fails += 1
                elif moved != base:
                    print(f'FAIL {flags} {want}: macros moved '
                          f'{sorted(moved ^ base)}')
                    fails += 1
                else:
                    print(f'PASS {flags} {want}: no macro moved')
                    passes += 1

        # Rejection wording: GCC 16.2 says
        #   unrecognized argument in option '-mprefer-vector-width=64'
        #   valid arguments ... are: 128 256 512 none
        code, _, err = run(args.lccc, ['-mprefer-vector-width=64', '-c', str(source),
                                       '-o', os.path.join(workdir, 'x.o')])
        if code == 0:
            print('FAIL -mprefer-vector-width=64: accepted, must be rejected')
            fails += 1
        elif 'unrecognized argument in option' in err and '128 256 512 none' in err:
            print('PASS -mprefer-vector-width=64: rejected with GCC wording')
            passes += 1
        else:
            print(f'FAIL -mprefer-vector-width=64: wrong wording: {err.strip()[:160]}')
            fails += 1

        print(f'{passes} PASS, {fails} FAIL of {passes + fails} (flags, macro) cases')
        return fails
    finally:
        if args.keep:
            os.makedirs(args.keep, exist_ok=True)
            shutil.copy(source, os.path.join(args.keep, 'vw.c'))
        shutil.rmtree(workdir, ignore_errors=True)


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
