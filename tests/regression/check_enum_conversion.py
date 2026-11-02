#!/usr/bin/env python3
"""GCC differential enum provenance/conversion checks, never kernel inputs."""
import argparse
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
from check_layout_unused_warnings import run

PREFIX = 'enum A { A0 }; enum B { B0 };\n'
CASES = {name: PREFIX + body for name, body in {
    'statement_constant': 'void f(void){enum B b=({A0;}); (void)b;}',
    'statement_variable': 'void f(enum A a){enum B b=({a;}); (void)b;}',
    'statement_local': 'void f(void){enum B b=({enum A a=A0; a;}); (void)b;}',
    'assignment_value': 'void f(void){enum B b; enum A a; b=(a=A0); (void)b;}',
    'constant': 'enum B b=A0;',
    'same_constant': 'enum A a=A0;',
    'integer': 'enum B b=0;',
    'return': 'enum B f(enum A a){return a;}',
    'initializer': 'void f(enum A a){enum B b=a; (void)b;}',
    'assignment': 'void f(enum B b){b=A0; (void)b;}',
    'argument': 'void g(enum B); void f(void){g(A0);}',
    'cast': 'void f(void){enum B b=(enum B)A0; (void)b;}',
    'cast_other': 'void f(void){enum B b=(enum A)0; (void)b;}',
    'promoted': 'void f(void){enum B b=+A0; (void)b;}',
    'arithmetic': 'void f(void){enum B b=A0+0; (void)b;}',
    'comma': 'void f(void){enum B b=((void)0,A0); (void)b;}',
    'conditional': 'enum B b=(1?A0:B0);',
    'struct': 'struct S {enum B b;}; struct S s={A0};',
    'array': 'enum B b[]={A0, B0};',
    'nested': 'struct S {enum B b;}; struct S s[1]={{A0}};',
    'designated': 'struct S {enum B b;}; struct S s={.b=A0};',
    'typedef': 'typedef enum B T; T b=A0;',
    'shadow_object': 'void f(void){int A0=0; enum B b=A0; (void)b;}',
    'shadow_constant': 'void f(void){{enum {A0=1}; enum B b=A0; (void)b;} enum B b=A0; (void)b;}',
    'pragma': '#pragma GCC diagnostic push\n#pragma GCC diagnostic ignored "-Wenum-conversion"\nenum B b=A0;\n#pragma GCC diagnostic pop\nenum B c=A0;',
}.items()}
FLAGS = [[], ['-Wall'], ['-Wextra'], ['-Wenum-conversion'],
    ['-Werror=enum-conversion'], ['-Wextra', '-Werror'],
    ['-Wextra', '-Wno-enum-conversion'],
    ['-Wno-enum-conversion', '-Wextra'],
    ['-Wenum-conversion', '-Wno-extra'],
    ['-Werror', '-Wextra', '-Wno-error=enum-conversion']]


def check(lccc, gcc):
    run(lccc, gcc, cases=CASES, flags=FLAGS, title='enum conversion')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--lccc', default=str(Path(__file__).resolve().parents[2]/'target/fastbuild/lccc'))
    parser.add_argument('--gcc', default='gcc-16')
    args = parser.parse_args()
    check(args.lccc, args.gcc)
