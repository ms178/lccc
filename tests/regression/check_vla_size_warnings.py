#!/usr/bin/env python3
"""Optimized VLA byte-bound warnings: GCC oracle, all four LCCC backends."""
import argparse
import os
from pathlib import Path
import re
import subprocess
import tempfile
import time

CASES = {
    'unbounded': 'void f(unsigned n){volatile char a[n];a[0]=0;}',
    'signed': 'void f(int n){volatile char a[n];a[0]=0;}',
    'guarded': 'void f(unsigned n){if(n>16)return;volatile char a[n];a[0]=0;}',
    'unsigned-view-positive-guard': 'void f(int n){if((unsigned)n>16)return;volatile char a[n];a[0]=0;}',
    'byte-sign-reinterpret': 'void f(unsigned char n){if((signed char)n < -50 || (signed char)n > 100)return;volatile char a[n];a[0]=0;}',
    'signed-positive-guard': 'void f(int n){if(n<1 || n>16)return;volatile char a[n];a[0]=0;}',
    'join-not-guard': 'extern void g(void);void f(unsigned n){if(n<16)g();volatile char a[n];a[0]=0;}',
    'mask': 'void f(unsigned n){volatile char a[n&15];a[0]=0;}',
    'remainder': 'void f(unsigned n){volatile char a[n%16];a[0]=0;}',
    'multiply': 'void f(unsigned n){if(n>16)return;volatile int a[n];a[0]=0;}',
    'wide-low-bits-guard': 'void f(unsigned long long n){if((unsigned)n>16)return;volatile char a[n];a[0]=0;}',
    'unsigned-wrap-guard': 'void f(unsigned n){if(n+1>16)return;volatile char a[n];a[0]=0;}',
    'signed-cast-guard': 'void f(unsigned n){if((int)n>16)return;volatile char a[n];a[0]=0;}',
    'wide-elements': 'void f(unsigned long long n){if(n>16)return;volatile int a[n];a[0]=0;}',
    'short': 'void f(unsigned short n){volatile char a[n];a[0]=0;}',
    'constant-guard': 'void f(unsigned n){if(n!=4096)return;volatile char a[n];a[0]=0;}',
    'constant': 'void f(void){int n=4096;volatile char a[n];a[0]=0;}',
    'fixed': 'void f(void){volatile char a[4096];a[0]=0;}',
    'alloca-not-vla': 'extern void use(void*);void f(unsigned n){use(__builtin_alloca(n));}',
    'two': 'void f(unsigned n,unsigned m){volatile char a[n],b[m];a[0]=0;b[0]=0;}',
    'pragma-enable-error': '#pragma GCC diagnostic error "-Wvla-larger-than=1"\nvoid f(unsigned n){volatile char a[n];a[0]=0;}',
    'pragma-demote': '#pragma GCC diagnostic warning "-Wvla-larger-than=2048"\nvoid f(unsigned n){volatile char a[n];a[0]=0;}',
    'pragma': '#pragma GCC diagnostic ignored "-Wvla-larger-than="\nvoid f(unsigned n){volatile char a[n];a[0]=0;}',
    # Distinct volatile effects prevent GCC IPA-ICF from deleting g's VLA.
    'pragma-pop': '#pragma GCC diagnostic push\n#pragma GCC diagnostic ignored "-Wvla-larger-than="\nvoid f(unsigned n){volatile char a[n];a[0]=0;}\n#pragma GCC diagnostic pop\nvoid g(unsigned n){volatile char a[n];a[0]=1;}',
}
FLAGS = [[], ['-Wvla-larger-than=1'], ['-Wvla-larger-than=2048'], ['-Wvla-larger-than=128'], ['-Wvla-larger-than=1MiB'],
         ['-Werror=vla-larger-than=2048'],
         ['-Werror=vla-larger-than=2048', '-Wno-error=vla-larger-than=2048'],
         ['-Wvla-larger-than=1', '-Wno-vla-larger-than'], ['-w', '-Werror=vla-larger-than=1'],
         ['-Wvla-larger-than=0'], ['-Werror=vla-larger-than=1', '-Wno-error=vla-larger-than='],
         ['-Og', '-O1', '-Wvla-larger-than=2048'], ['-O1', '-Og', '-Wvla-larger-than=2048']]


def result(command, source, flags, output):
    p = subprocess.run(command + ['-S', '-fdiagnostics-color=never', '-o', str(output)] + flags + [str(source)],
                       capture_output=True, text=True, timeout=30)
    if 'tolerated but not implemented' in p.stderr:
        raise AssertionError(p.stderr)
    kinds = sorted(re.findall(r'(warning|error):[^\n]*\[-W(?:error=)?vla-larger-than=\]', p.stderr))
    return (p.returncode != 0, kinds), p.stderr


def main():
    started = time.monotonic()
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--lccc', default='target/fastbuild/lccc')
    parser.add_argument('--gcc', default=os.environ.get('GCC', 'gcc-16'))
    args = parser.parse_args()
    checks = 0
    with tempfile.TemporaryDirectory(prefix='lccc-vla-size-') as td:
        source, output = Path(td)/'fixture.c', Path(td)/'fixture.s'
        for name, text in CASES.items():
            source.write_text(text + '\n')
            for opt in ('-O0', '-O1', '-Og', '-O2', '-Os', '-Oz'):
                for flags in FLAGS:
                    expected, oracle = result([args.gcc], source, [opt] + flags, output)
                    for suffix in ('', '-i686', '-arm', '-riscv'):
                        got, err = result([args.lccc + suffix], source, [opt] + flags, output)
                        if got != expected:
                            raise AssertionError(f'{name} {suffix} {opt} {flags}: {got} != GCC {expected}\n{err}\nGCC:\n{oracle}')
                        checks += 1
    print(f'VLA-size diagnostics: PASS ({checks} four-target GCC differential checks)')
    print(f'Gate elapsed: {time.monotonic() - started:.2f}s')


if __name__ == '__main__':
    main()
