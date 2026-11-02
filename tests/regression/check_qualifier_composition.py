#!/usr/bin/env python3
"""Qualifier composition contracts across dialects and all supplied target drivers.

GCC 16.2 is the diagnostic-policy oracle. Each independent input checks exact
primary diagnostic classification and exit status; no ill-formed monolithic
input can hide a missing exit transition. Host Clang is an optional independent
accept/reject oracle for the portable cases (not a GCC wording oracle).
"""
import argparse
import re
import subprocess
import tempfile
from pathlib import Path

from check_qualifier_conversions import ENV, diagnose, oracle_path

# name, translation unit, expected option (None means legal), dialect default error
CASES = [
    ('address', 'void f(void){const int x=1;int*p=&x;}', 'discarded-qualifiers', False),
    ('address_add', 'void f(void){const int x=1;int*p=&x+0;}', 'discarded-qualifiers', False),
    ('array_add', 'void f(void){const int a[2]={0};int*p=a+1;}', 'discarded-qualifiers', False),
    ('reverse_array_add', 'void f(void){const int a[2]={0};int*p=1+a;}', 'discarded-qualifiers', False),
    ('address_conditional', 'void f(int c){const int x=1;int*p=c?&x:&x;}', 'discarded-qualifiers', False),
    ('array_conditional', 'void f(int c){const int a[2]={0};int*p=c?a:a;}', 'discarded-qualifiers', False),
    ('address_comma', 'void f(void){const int x=1;int*p=(0,&x);}', 'discarded-qualifiers', False),
    ('array_comma', 'void f(void){const int a[2]={0};int*p=(0,a);}', 'discarded-qualifiers', False),
    ('return_address_add', 'const int x=1;int*f(void){return &x+0;}', 'discarded-qualifiers', False),
    ('argument_address_add', 'void g(int*);void f(void){const int x=1;g(&x+0);}', 'discarded-qualifiers', False),
    ('assignment_array_add', 'void f(int*p){const int a[2]={0};p=a+1;}', 'discarded-qualifiers', False),
    ('member_address_add', 'struct S{const int x;};void f(struct S*s){int*p=&s->x+0;}', 'discarded-qualifiers', False),
    ('outer_member_add', 'struct S{int x;};void f(const struct S*s){int*p=&s->x+0;}', 'discarded-qualifiers', False),
    ('nested_array_add', 'void f(void){const int a[2][3]={0};int(*p)[3]=a+1;}', 'discarded-array-qualifiers', False),
    ('callback_argument', 'void g(void(*)(int*));void h(const int*);void f(void){g(h);}', 'incompatible-pointer-types', True),
    ('callback_compare', 'void f(void(*a)(int*),void(*b)(const int*)){(void)(a==b);}', 'compare-distinct-pointer-types', False),
    ('callback_return', 'void f(int*(*a)(void),const int*(*b)(void)){a=b;}', 'incompatible-pointer-types', True),
    ('nested_callback', 'void f(void(*a)(int**),void(*b)(const int**)){a=b;}', 'incompatible-pointer-types', True),
    ('choose_pointer', 'void f(void){const int x=1;int*p=__builtin_choose_expr(1,&x,(int*)0);}', 'discarded-qualifiers', False),
    ('choose_array', 'void f(void){const int a[2]={0};int*p=__builtin_choose_expr(1,a,a)+1;}', 'discarded-qualifiers', False),
    ('generic_pointer', 'void f(void){const int x=1;int*p=_Generic(0,int:&x);}', 'discarded-qualifiers', False),
    ('generic_array', 'void f(void){const int a[2]={0};int*p=_Generic(0,int:a)+1;}', 'discarded-qualifiers', False),
    ('legal_add', 'void f(void){int a[2]={0};int*p=a+1;}', None, False),
    ('legal_const_add', 'void f(void){const int a[2]={0};const int*p=a+1;}', None, False),
    ('legal_callback_names', 'void f(void(*a)(int*x),void(*b)(int*y)){a=b;}', None, False),
    ('legal_callback_top_const', 'void f(void(*a)(int*),void(*b)(int*const)){a=b;}', None, False),
    ('shadow_object', 'const int x=1;void f(void){int x=2;int*p=&x+0;}', None, False),
    ('restore_object', 'const int x=1;void f(void){{int x=2;}int*p=&x+0;}', 'discarded-qualifiers', False),
    ('shadow_typedef', 'typedef const int T;void f(void){typedef int T;T x=1;int*p=&x+0;}', None, False),
    ('restore_typedef', 'typedef const int T;void f(void){{typedef int T;}T x=1;int*p=&x+0;}', 'discarded-qualifiers', False),
]


def qualifier_matrix():
    """All 64 CVR pairs at the immediate and nested pointer boundaries."""
    spelling = lambda bits: ' '.join(word for mask, word in
                                    [(1, 'const'), (2, 'volatile'), (4, '__restrict__')]
                                    if bits & mask)
    for source in range(8):
        for target in range(8):
            sq, tq = spelling(source), spelling(target)
            drop = 'discarded-qualifiers' if source & ~target else None
            mismatch = 'incompatible-pointer-types' if source != target else None
            yield (f'matrix_immediate_{source}_{target}',
                   f'void f(int *{sq} *a,int *{tq} *b){{b=a;}}', drop, False)
            yield (f'matrix_nested_{source}_{target}',
                   f'void f(int *{sq} **a,int *{tq} **b){{b=a;}}', mismatch, True)



def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--lccc', action='append', required=True)
    parser.add_argument('--gcc', default='gcc-16')
    parser.add_argument('--clang', help='optional Clang 23.1.4 semantic oracle')
    parser.add_argument('--icx', help='authenticated ICX 2026.2 semantic oracle')
    parser.add_argument('--vendor-only', action='store_true',
                        help='explicit vendor lane: LCCC contracts plus a required vendor oracle')
    args = parser.parse_args()
    if args.vendor_only and not (args.clang or args.icx):
        parser.error('--vendor-only requires --clang or --icx')
    gcc = None if args.vendor_only else oracle_path(args.gcc, False)
    compilers = ([gcc] if gcc else []) + [str(Path(cc).resolve()) for cc in args.lccc]
    if args.clang:
        version = subprocess.check_output([args.clang, '-dumpversion'], env=ENV, text=True).strip()
        if version != '23.1.4':
            raise SystemExit(f'expected Clang 23.1.4, got {version}')
    if args.icx:
        version = subprocess.check_output([args.icx, '--version'], env=ENV, text=True,
                                          stderr=subprocess.STDOUT, timeout=20)
        if not re.search(r'Intel.*2026\.2(?:\.|\b)', version):
            raise SystemExit(f'expected Intel ICX 2026.2, got {version}')
    checks = 0
    cases = [*CASES, *qualifier_matrix()]
    with tempfile.TemporaryDirectory(prefix='qualifier-composition-') as tmp:
        source = Path(tmp) / 'case.c'
        for std in ('gnu90', 'gnu17', 'gnu23'):
            for name, code, option, default_error in cases:
                source.write_text(code + '\n')
                severity = 'error' if default_error and std != 'gnu90' else 'warning'
                expected = [] if option is None else [(severity, option)]
                for cc in compilers:
                    rows = diagnose(cc, source=source, std=std)
                    got = [(row['sev'], row['opt']) for row in rows]
                    if got != expected:
                        raise SystemExit(f'{name}/{std}/{cc}: got {got}, expected {expected}\n{code}')
                    checks += 1
                    if option:
                        quiet = diagnose(cc, '-Wno-' + option, source=source, std=std)
                        promoted = diagnose(cc, '-Werror=' + option, source=source, std=std)
                        if quiet or [(r['sev'], r['opt']) for r in promoted] != [('error', option)]:
                            raise SystemExit(f'{name}/{std}/{cc}: suppression/promotion contract failed')
                        checks += 2
                for vendor in ([args.clang] if args.clang and std == 'gnu17' and not name.startswith('matrix_') else []) + (
                        [args.icx] if args.icx and std == 'gnu17' and not name.startswith('matrix_') else []):
                    result = subprocess.run([vendor, '-std=gnu17', '-fsyntax-only', '-Werror',
                                             '-Wno-unused-value', str(source)], env=ENV,
                                            capture_output=True, text=True, timeout=30)
                    expected_rc = int(option is not None)
                    if result.returncode != expected_rc or re.search(
                            r'internal compiler error|PLEASE submit|panicked at', result.stderr):
                        raise SystemExit(f'{name}/{vendor}: exit {result.returncode}, expected {expected_rc}\n{result.stderr}')
                    checks += 1
    print(f'qualifier-composition: {len(cases)} cases, {checks} checks PASS')


if __name__ == '__main__':
    main()
