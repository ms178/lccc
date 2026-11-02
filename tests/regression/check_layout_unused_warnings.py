#!/usr/bin/env python3
"""Non-kernel GCC differential checks for layout and file-const diagnostics.

Compare actual code-generation AND syntax-only modes: GCC deliberately defers
file-scope unused warnings until code generation. Never accept option tolerance
as implementation. Header level, specificity, promotion, and pragma scopes are
observable contracts, not merely positive smoke tests.
"""
import argparse
from collections import Counter
import os
from pathlib import Path
import re
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
CASES = {
    'packed': 'struct A {int x;} __attribute__((aligned(16)));\nstruct B {char c; struct A a;} __attribute__((packed));\n',
    'packed_array': 'struct A {int x;} __attribute__((aligned(16)));\nstruct B {char c; struct A a[2];} __attribute__((packed));\n',
    'packed_union': 'struct A {int x;} __attribute__((aligned(16)));\nunion B {char c; struct A a;} __attribute__((packed));\n',
    'packed_field': 'struct A {int x;} __attribute__((aligned(16)));\nstruct B {char c; struct A a __attribute__((packed));};\n',
    'packed_outer_aligned': 'struct A {int x;} __attribute__((aligned(16)));\nstruct B {char c; struct A a;} __attribute__((packed,aligned(16)));\n',
    'packed_pragma': 'struct A {int x;} __attribute__((aligned(16)));\n#pragma pack(push,1)\nstruct B {char c; struct A a;};\n#pragma pack(pop)\n',
    'ordinary': 'struct A {int x;}; struct B {char c; struct A a;} __attribute__((packed));\n',
    'aligned_typedef': 'typedef int I __attribute__((aligned(16))); struct B {char c; I a;} __attribute__((packed));\n',
    'vector': 'typedef int V __attribute__((vector_size(16))); struct B {char c; V a;} __attribute__((packed));\n',
    'resolved_twice': 'struct A {int x;} __attribute__((aligned(16)));\nstruct B {char c; struct A a;} __attribute__((packed));\nstruct B b; int f(void) {return sizeof(struct B)+sizeof(b);}\n',
    'static_const': 'static const int x=1;\n',
    'external_const': 'const int x=1;\n',
    'block_const': 'int f(void) {const int x=1;return 0;}\n',
    'block_static_const': 'int f(void) {static const int x=1;return 0;}\n',
    'static_mutable': 'static int x=1;\n',
    'used_const': 'static const int x=1; int f(void) {return x;}\n',
    'maybe_unused': 'static const int x __attribute__((unused))=1;\n',
    'used_attribute': 'static const int x __attribute__((used))=1;\n',
    'header_const': '#include "fixture.h"\n',
    'renamed_main': '#line 1 "renamed.h"\nstatic const int x=1;\n',
    'scoped_const': '#pragma GCC diagnostic push\n#pragma GCC diagnostic ignored "-Wunused-const-variable"\nstatic const int x=1;\n#pragma GCC diagnostic pop\nstatic const int y=1;\n',
}
for label, body in {
    'set_only': 'int x; x=1; return 0;',
    'used_assignment': 'int x; return x=1;',
    'increment': 'int x=0; x++; return 0;',
    'preincrement': 'int x=0; ++x; return 0;',
    'compound': 'int x=0; x+=2; return 0;',
    'explicit_rhs_read': 'int x=0; x=x+1; return 0;',
    'array_set': 'int a[2]; a[0]=1; return 0;',
    'array_index_read': 'int a[2]={0}; a[a[0]]=1; return 0;',
    'member_set': 'struct {int x;} a; a.x=1; return 0;',
    'pointer_read': 'int x; int *p=&x; *p=1; return x;',
    'volatile_set': 'volatile int x; x=1; return 0;',
    'unused_initializer': 'int x=1; return 0;',
    'address_used': 'int x; int *p=&x; return p!=0;',
    'void_cast': 'int x=0; (void)(x=1); return 0;',
    'logical_use': 'int x=0; if (1&&(x=1)) {} return 0;',
    'sizeof_use': 'int x=0; (void)sizeof x; return 0;',
    'comma': 'int x; int y; x=1,y=2; return 0;',
    'shadow': 'int x=0; {int x; x=1;} return x;',
    'statement_expr_used': 'int x; return ({x=1;});',
    'statement_expr_unused': 'int x; ({x=1;}); return 0;',
    'loop_increment': 'int x=0; for (;0;x++) {} return 0;',
}.items():
    CASES[label] = 'int f(void) {' + body + '}\n'

FLAGS = [[], ['-Wall'], ['-Wextra'], ['-Wunused-variable'],
         ['-Wunused-const-variable'], ['-Wunused-const-variable=0'],
         ['-Wunused-const-variable=1'], ['-Wunused-const-variable=2'],
         ['-Werror=unused-const-variable'], ['-Werror=unused-const-variable=1'],
         ['-Wall', '-Wno-unused-const-variable'],
         ['-Wno-unused-const-variable', '-Wall'],
         ['-Wunused-const-variable=2', '-Wunused-variable'],
         ['-Wunused-variable', '-Wunused-const-variable=2'],
         ['-Werror=unused-variable'],
         ['-Werror=unused-const-variable', '-Wno-error=unused-const-variable'],
         ['-Wpacked-not-aligned'], ['-Werror=packed-not-aligned'],
         ['-Wpacked-not-aligned', '-Wno-packed-not-aligned'],
         ['-Wno-packed-not-aligned', '-Wall'],
         ['-Wall', '-Wno-error=packed-not-aligned', '-Werror']]
FLAGS += [[f'-Wunused-but-set-variable={level}'] for level in range(4)]
FLAGS += [['-Wunused-but-set-variable'], ['-Werror=unused-but-set-variable'],
    ['-Wall', '-Wno-unused-but-set-variable'],
    ['-Werror=unused-but-set-variable', '-Wno-error=unused-but-set-variable']]
FLAGS += [['-Werror=unused-const-variable', '-Wno-error=unused-variable'],
    ['-Wno-error=unused-const-variable', '-Werror=unused-variable'],
    ['-Wunused-const-variable', '-Wno-unused-variable'],
    ['-Wno-unused-variable', '-Wunused-const-variable'],
    ['-Wno-error=unused-const-variable=2'],
    ['-Werror', '-Wunused-const-variable=1', '-Wno-error=unused-const-variable=2']]
PATTERN = re.compile(r'(warning|error):[^\n]*\[-W(?:error=)?([^\]]+)\]')


def signature(proc):
    if 'not implemented' in proc.stderr or 'unrecognized command-line' in proc.stderr:
        raise AssertionError(proc.stderr)
    return bool(proc.returncode), Counter((severity, name.rstrip('='))
        for severity, name in PATTERN.findall(proc.stderr))


def check_rejected_negative_levels(lccc, gcc):
    commands = [c if isinstance(c, list) else [str(c)] for c in (gcc, lccc)]
    count = 0
    for family in ['format', 'unused-const-variable', 'unused-but-set-variable']:
        flag = f'-Wno-{family}=1'
        for strict in ['0', '1']:
            for command in commands:
                result = subprocess.run([*command, '-fsyntax-only', '-x', 'c', '-', flag],
                    input='int x;', text=True, capture_output=True, timeout=30,
                    env=dict(os.environ, LC_ALL='C', LCCC_STRICT_OPTIONS=strict))
                if result.returncode == 0 or flag not in result.stderr or 'error:' not in result.stderr:
                    raise AssertionError(f'{command} accepted malformed {flag}: {result.stderr}')
            count += 1
    print(f'negative numeric warning options: PASS ({count} GCC differential checks)')


def run(lccc, gcc, *, cases=None, flags=None, title="layout/file-const diagnostics"):
    cases = CASES if cases is None else cases
    flags_to_test = FLAGS if flags is None else flags
    env = dict(os.environ, LC_ALL='C', LCCC_STRICT_OPTIONS='1')
    lccc = lccc if isinstance(lccc, list) else [str(lccc)]
    gcc = gcc if isinstance(gcc, list) else [str(gcc)]
    checked = 0
    with tempfile.TemporaryDirectory(prefix='lccc-layout-unused-') as tmp:
        root = Path(tmp)
        (root / 'fixture.h').write_text('static const int header_const=1;\n')
        source = root / 'fixture.c'
        for label, text in cases.items():
            source.write_text(text)
            for mode in ['-S', '-fsyntax-only']:
                for flags in flags_to_test:
                    results = [subprocess.run([*compiler, mode, '-std=gnu17',
                        '-fdiagnostics-color=never', *flags, str(source), '-o', str(root/'out.s')],
                        text=True, capture_output=True, env=env, timeout=30)
                        for compiler in [gcc, lccc]]
                    expected, actual = map(signature, results)
                    if expected != actual:
                        raise AssertionError(f'{label} {mode} {flags}: GCC {expected}, LCCC {actual}\n'
                            f'GCC:\n{results[0].stderr}\nLCCC:\n{results[1].stderr}')
                    checked += 1
    print(f'{title}: PASS ({checked} non-kernel GCC differential checks)')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--lccc', default=str(ROOT/'target/fastbuild/lccc'))
    parser.add_argument('--gcc', default='gcc-16')
    args = parser.parse_args()
    run(args.lccc, args.gcc)
