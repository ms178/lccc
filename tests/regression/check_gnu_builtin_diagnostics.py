#!/usr/bin/env python3
"""Require GCC-compatible raw bit-count and mode rejections, without ICEs."""
import argparse
from pathlib import Path
import subprocess
import tempfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--lccc', default='target/fastbuild/lccc')
    parser.add_argument('--gcc', default='gcc-16')
    args = parser.parse_args()
    cases = []
    for mode in ('-m32', '-m64'):
        for operation in ('pslldqi128', 'psrldqi128', 'pslldqi256', 'psrldqi256', 'palignr128'):
            size = 32 if operation.endswith('256') else 16
            operands = 'a, a, ' if operation == 'palignr128' else 'a, '
            for count in (-8, 7, 2048):
                source = (f'typedef long long V __attribute__((vector_size({size})));\n'
                          f'V f(V a) {{ return __builtin_ia32_{operation}({operands}{count}); }}\n')
                cases.append((mode, operation + str(count), source))
    declarations = ('typedef int V __attribute__((vector_size(16)));\n'
                    'typedef int S __attribute__((vector_size(8)));\n'
                    'typedef int W __attribute__((vector_size(32)));\n')
    for mode in ('-m32', '-m64'):
        for name, call in (
            ('scalar_in_vector_position', '__builtin_ia32_paddd128(1, (V){0})'),
            ('short_vector', '__builtin_ia32_paddd128((S){0}, (V){0})'),
            ('wide_vector', '__builtin_ia32_paddd128((W){0}, (V){0})'),
            ('vector_in_count_position', '__builtin_ia32_pslldi128((V){0}, (V){0})'),
            ('widening_requires_narrow_source', '__builtin_ia32_pmovsxbw256((W){0})'),
        ):
            cases.append((mode, name, declarations + f'void f(void) {{ {call}; }}\n'))
    for name, operands in (
        ('bzhi_di', '1, 2'), ('crc32di', '1, 2'),
        ('cvtsd2si64', '(D){0}'), ('cvtss2si64', '(F){0}'),
        ('movnti64', '(long long *)0, 0'),
    ):
        source = ('typedef double D __attribute__((vector_size(16)));\n'
                  'typedef float F __attribute__((vector_size(16)));\n'
                  f'void f(void) {{ __builtin_ia32_{name}({operands}); }}\n')
        cases.append(('-m32', name, source))
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        for mode, name, source in cases:
            path = root / 'probe.c'
            path.write_text(source)
            for compiler in (args.gcc, args.lccc):
                result = subprocess.run(
                    [compiler, mode, '-O2', '-mavx2', '-msse4.2', '-mbmi2',
                     '-Werror=implicit-function-declaration', '-S', str(path), '-o', str(root / 'out.s')],
                    capture_output=True, text=True, timeout=30)
                if result.returncode == 0 or 'error:' not in result.stderr:
                    raise SystemExit(f'{compiler}: {mode} {name}: missing rejection:\n{result.stderr}')
                if result.returncode < 0 or any(text in result.stderr for text in
                                               ('internal compiler error', 'panicked at', 'RUST_BACKTRACE')):
                    raise SystemExit(f'{compiler}: {mode} {name}: crashed:\n{result.stderr}')
    print(f'GNU builtin diagnostics: PASS ({len(cases)} rejections per compiler; both x86 modes)')


if __name__ == '__main__':
    main()
