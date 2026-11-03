#!/usr/bin/env python3
"""Fixed-frame diagnostic contracts on all four backends (non-kernel GCC oracle).

Frame byte counts are backend-owned, not expected to equal GCC's allocator.
Compare diagnostic classifications for intentionally separated small/large cases.
"""
import argparse
import os
from pathlib import Path
import re
import subprocess
import tempfile
import time

LARGE = 'extern void sink(void*);void f(void){char a[4096];sink(a);}\n'
CASES = {
    'large': LARGE,
    'small': LARGE.replace('4096', '16'),
    'dynamic': 'extern void sink(void*);void f(unsigned n){char a[n];sink(a);}\n',
    'two': LARGE + 'void g(void){char b[8192];sink(b);}\n',
    'pragma-ignore': '#pragma GCC diagnostic ignored "-Wframe-larger-than="\n' + LARGE,
    'pragma-numeric-ignore': '#pragma GCC diagnostic ignored "-Wframe-larger-than=2048"\n' + LARGE,
    'pragma-pop': '#pragma GCC diagnostic push\n#pragma GCC diagnostic ignored "-Wframe-larger-than="\n' + LARGE + '#pragma GCC diagnostic pop\nvoid g(void){char b[8192];sink(b);}\n',
    'pragma-demote': '#pragma GCC diagnostic warning "-Wframe-larger-than=2048"\n' + LARGE,
    'pragma-threshold': '#pragma GCC diagnostic error "-Wframe-larger-than=1"\n' + LARGE.replace('4096', '16'),
}
FLAGS = [[], ['-Wframe-larger-than=2048'], ['-Wframe-larger-than=2KiB'], ['-Wframe-larger-than=999999999999999999999999999'],
         ['-Werror=frame-larger-than=2048'],
         ['-Werror=frame-larger-than=2048', '-Wno-error=frame-larger-than=2048'],
         ['-Wframe-larger-than=2048', '-Wno-frame-larger-than'],
         ['-w', '-Werror=frame-larger-than=2048']]


def result(command, source, flags, mode, output):
    p = subprocess.run(command + [mode, '-fdiagnostics-color=never', '-o', str(output)] + flags + [str(source)],
                       capture_output=True, text=True, timeout=30)
    if 'tolerated but not implemented' in p.stderr:
        raise AssertionError(p.stderr)
    kinds = sorted(re.findall(r'(warning|error):[^\n]*\[-W(?:error=)?frame-larger-than=\]', p.stderr))
    return (p.returncode != 0, kinds), p.stderr


def main():
    started = time.monotonic()
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--lccc', default='target/fastbuild/lccc')
    parser.add_argument('--gcc', default=os.environ.get('GCC', 'gcc-16'))
    args = parser.parse_args()
    checks = 0
    with tempfile.TemporaryDirectory(prefix='lccc-frame-size-') as td:
        source, output = Path(td)/'fixture.c', Path(td)/'fixture.s'
        for name, text in CASES.items():
            source.write_text(text)
            for opt in ('-O0', '-O2', '-Os'):
                for mode in ('-S', '-fsyntax-only'):
                    for flags in FLAGS:
                        expected, oracle = result([args.gcc], source, [opt] + flags, mode, output)
                        for suffix in ('', '-i686', '-arm', '-riscv'):
                            got, err = result([args.lccc + suffix], source, [opt] + flags, mode, output)
                            if got != expected:
                                raise AssertionError(f'{name} {suffix} {opt} {mode} {flags}: {got} != GCC {expected}\n{err}\nGCC:\n{oracle}')
                            checks += 1
        for family in ('frame-larger-than', 'vla-larger-than'):
            for level in ('warning', 'error'):
                source.write_text(f'#pragma GCC diagnostic {level} "-W{family}="\n' + LARGE)
                for command in ([args.gcc], [args.lccc]):
                    p = subprocess.run(command + ['-fsyntax-only', str(source)], capture_output=True, text=True)
                    if p.returncode == 0:
                        raise AssertionError(f'missing pragma threshold accepted: {command} {family} {level}')
                    checks += 1
        source.write_text(LARGE)
        for flag in ('-Wframe-larger-than=-1', '-Wframe-larger-than=', '-Wframe-larger-than=1K',
                     '-Wframe-larger-than=bogus', '-Wno-frame-larger-than=2'):
            for command in ([args.gcc], [args.lccc]):
                p = subprocess.run(command + ['-S', flag, '-o', str(output), str(source)], capture_output=True, text=True)
                if p.returncode == 0:
                    raise AssertionError(f'malformed threshold accepted: {command} {flag}')
                checks += 1
        # Zero is a byte threshold, NOT the disable level used by -Wformat=0.
        got, err = result([args.lccc], source, ['-Wframe-larger-than=0'], '-S', output)
        if got != (False, ['warning']):
            raise AssertionError(f'zero threshold did not warn: {got}\n{err}')
        checks += 1
    print(f'frame-size diagnostics: PASS ({checks} checks; four backend-owned frame reservations)')
    print(f'Gate elapsed: {time.monotonic() - started:.2f}s')


if __name__ == '__main__':
    main()
