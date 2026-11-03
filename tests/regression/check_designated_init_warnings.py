#!/usr/bin/env python3
"""Non-kernel GCC16 oracle for designated_init, including aggregate cursors.

Check counts/severity, never tolerate a flag or attribute as an implementation.
"""
import argparse
import os
from pathlib import Path
import re
import subprocess
import tempfile
import time

RECORD = 'struct __attribute__((designated_init)) D { int x,y; };\n'
CASES = {
    'positional': RECORD + 'struct D d={1,2};',
    'zero': RECORD + 'struct D d={0};',
    'empty': RECORD + 'struct D d={};',
    'named': RECORD + 'struct D d={.x=1,.y=2};',
    'mixed': RECORD + 'struct D d={.x=1,2};',
    'plain': 'struct D {int x,y;}; struct D d={1,2};',
    'typedef': 'typedef struct __attribute__((designated_init)) {int x,y;} D; D d={1,2};',
    'post-body': 'struct D {int x,y;} __attribute__((designated_init)); struct D d={1,2};',
    'nested-braced': RECORD + 'struct O {struct D d;int z;} o={{1,2},3};',
    'nested-flat': RECORD + 'struct O {struct D d;int z;} o={1,2,3};',
    'nested-named': RECORD + 'struct O {struct D d;int z;} o={.d={1,2},.z=3};',
    'array': RECORD + 'struct D d[2]={{1,2},{3,4}};',
    'range': RECORD + 'struct D d[10]={[1 ... 8]={1,2}};',
    'compound': RECORD + 'int f(void){return ((struct D){1,2}).x;}',
    'copy': RECORD + 'struct D f(struct D a){struct D b=a;return b;}',
    'scope': RECORD + 'struct D d={1,2}; int f(void){struct D {int x;};struct D d={1};return d.x;}',
    'outer-not-inner': 'struct __attribute__((designated_init)) O {struct I {int x,y;} i;}; struct I i={1,2}; struct O o={{1,2}};',
    'inner-not-outer': 'struct O {struct __attribute__((designated_init)) I {int x,y;} i;int z;}; struct O o={{1,2},3};',
    'empty-field': 'struct __attribute__((designated_init)) D {int a[2];};struct D d={{}};',
    'ignored-typedef': 'struct D {int x;};typedef struct D T __attribute__((designated_init));T x={1};',
    'pragma': RECORD + '#pragma GCC diagnostic push\n#pragma GCC diagnostic ignored "-Wdesignated-init"\nstruct D d={1,2};\n#pragma GCC diagnostic pop\nstruct D e={3,4};',
    'pragma-empty': 'struct __attribute__((designated_init)) D {int a[2];};\n#pragma GCC diagnostic ignored "-Wdesignated-init"\nstruct D d={{}};',
    'forward': 'struct __attribute__((designated_init)) D;struct D {int x;};struct D d={1};',
    'reference-attribute': 'struct D {int x;};struct __attribute__((designated_init)) D d={1};',
    'invalid-member': 'struct D {int x __attribute__((designated_init));};',
    'invalid-typedef-member': 'typedef int I;struct D {I x __attribute__((designated_init));};',
    'ignored-record-member': 'struct D {int x;};struct O {struct D d __attribute__((designated_init));};struct D d={1};',
    'invalid-union': 'union __attribute__((designated_init)) U {int x;};',
    'invalid-scalar': 'int x __attribute__((designated_init));',
}
FLAGS = [[], ['-Wno-designated-init'], ['-Werror=designated-init'],
         ['-Werror=designated-init', '-Wno-error=designated-init'],
         ['-w', '-Werror=designated-init'], ['-Werror', '-Wno-designated-init']]


def signature(command, source, flags):
    p = subprocess.run(command + ['-std=gnu11', '-fsyntax-only', '-fdiagnostics-color=never'] + flags + [str(source)],
                       capture_output=True, text=True, timeout=30)
    family = re.findall(r'(warning|error):[^\n]*\[-W(?:error=)?designated-init\]', p.stderr)
    if 'tolerated but not implemented' in p.stderr or "attribute 'designated_init' is parsed without" in p.stderr:
        raise AssertionError(p.stderr)
    return (p.returncode != 0, sorted(family)), p.stderr


def main():
    started = time.monotonic()
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--lccc', default='target/fastbuild/lccc')
    parser.add_argument('--gcc', default=os.environ.get('GCC', 'gcc-16'))
    args = parser.parse_args()
    count = 0
    with tempfile.TemporaryDirectory(prefix='lccc-designated-init-') as td:
        source = Path(td) / 'fixture.c'
        for name, text in CASES.items():
            source.write_text(text + '\n')
            for flags in FLAGS:
                expected, gcc_err = signature([args.gcc], source, flags)
                for target, suffix in (('x86_64', ''), ('i686', '-i686'), ('aarch64', '-arm'), ('riscv64', '-riscv')):
                    got, err = signature([args.lccc + suffix], source, flags)
                    if got != expected:
                        raise AssertionError(f'{name} {target} {flags}: {got} != GCC {expected}\nLCCC:\n{err}\nGCC:\n{gcc_err}')
                    count += 1
    print(f'designated-init: PASS ({count} four-target GCC differential checks)')
    print(f'Gate elapsed: {time.monotonic() - started:.2f}s')


if __name__ == '__main__':
    main()
