#!/usr/bin/env python3
"""GCC 16.2 differential for warning specificity, group expansion and severity."""
from __future__ import annotations

import argparse
import re
import shlex
import subprocess
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
FIXTURES = {
    'cast_fn_0': 'enum E { E0 }; enum F { F0 }; struct S { int x; }; struct T { int x; };\ntypedef int (*Source)(int); typedef int (*Target)(double); Target convert(Source value) { return (Target)value; }\n',
    'cast_fn_1': 'enum E { E0 }; enum F { F0 }; struct S { int x; }; struct T { int x; };\ntypedef int (*Source)(int); typedef int (*Target)(unsigned); Target convert(Source value) { return (Target)value; }\n',
    'cast_fn_2': 'enum E { E0 }; enum F { F0 }; struct S { int x; }; struct T { int x; };\ntypedef long (*Source)(int); typedef long long (*Target)(int); Target convert(Source value) { return (Target)value; }\n',
    'cast_fn_3': 'enum E { E0 }; enum F { F0 }; struct S { int x; }; struct T { int x; };\ntypedef int (*Source)(void*); typedef int (*Target)(char*); Target convert(Source value) { return (Target)value; }\n',
    'cast_fn_4': 'enum E { E0 }; enum F { F0 }; struct S { int x; }; struct T { int x; };\ntypedef int (*Source)(int); typedef void (*Target)(void); Target convert(Source value) { return (Target)value; }\n',
    'cast_fn_5': 'enum E { E0 }; enum F { F0 }; struct S { int x; }; struct T { int x; };\ntypedef int (*Source)(void); typedef int (*Target)(int); Target convert(Source value) { return (Target)value; }\n',
    'cast_fn_6': 'enum E { E0 }; enum F { F0 }; struct S { int x; }; struct T { int x; };\ntypedef int (*Source)(int,...); typedef int (*Target)(int,double); Target convert(Source value) { return (Target)value; }\n',
    'cast_fn_7': 'enum E { E0 }; enum F { F0 }; struct S { int x; }; struct T { int x; };\ntypedef int (*Source)(int); typedef int (*Target)(int,...); Target convert(Source value) { return (Target)value; }\n',
    'cast_fn_8': 'enum E { E0 }; enum F { F0 }; struct S { int x; }; struct T { int x; };\ntypedef int (*Source)(); typedef int (*Target)(int); Target convert(Source value) { return (Target)value; }\n',
    'cast_fn_9': 'enum E { E0 }; enum F { F0 }; struct S { int x; }; struct T { int x; };\ntypedef int (*Source)(int); typedef int (*Target)(); Target convert(Source value) { return (Target)value; }\n',
    'cast_fn_10': 'enum E { E0 }; enum F { F0 }; struct S { int x; }; struct T { int x; };\ntypedef long (*Source)(); typedef int (*Target)(int); Target convert(Source value) { return (Target)value; }\n',
    'cast_fn_11': 'enum E { E0 }; enum F { F0 }; struct S { int x; }; struct T { int x; };\ntypedef float (*Source)(int); typedef double (*Target)(int); Target convert(Source value) { return (Target)value; }\n',
    'cast_fn_12': 'enum E { E0 }; enum F { F0 }; struct S { int x; }; struct T { int x; };\ntypedef int (*Source)(short); typedef int (*Target)(int); Target convert(Source value) { return (Target)value; }\n',
    'cast_fn_13': 'enum E { E0 }; enum F { F0 }; struct S { int x; }; struct T { int x; };\ntypedef int (*Source)(void (*)(int)); typedef int (*Target)(void*); Target convert(Source value) { return (Target)value; }\n',
    'cast_fn_14': 'enum E { E0 }; enum F { F0 }; struct S { int x; }; struct T { int x; };\ntypedef _Bool (*Source)(int); typedef unsigned char (*Target)(int); Target convert(Source value) { return (Target)value; }\n',
    'cast_fn_15': 'enum E { E0 }; enum F { F0 }; struct S { int x; }; struct T { int x; };\ntypedef void (*Source)(); typedef int (*Target)(int); Target convert(Source value) { return (Target)value; }\n',
    'cast_fn_16': 'enum E { E0 }; enum F { F0 }; struct S { int x; }; struct T { int x; };\ntypedef void (*Source)(void); typedef int (*Target)(int); Target convert(Source value) { return (Target)value; }\n',
    'cast_fn_17': 'enum E { E0 }; enum F { F0 }; struct S { int x; }; struct T { int x; };\ntypedef int (*Source)(int,...); typedef int (*Target)(double,int); Target convert(Source value) { return (Target)value; }\n',
    'cast_fn_18': 'enum E { E0 }; enum F { F0 }; struct S { int x; }; struct T { int x; };\ntypedef int (*Source)(int,int,...); typedef int (*Target)(int); Target convert(Source value) { return (Target)value; }\n',
    'cast_fn_19': 'enum E { E0 }; enum F { F0 }; struct S { int x; }; struct T { int x; };\ntypedef int* (*Source)(int); typedef void* (*Target)(int); Target convert(Source value) { return (Target)value; }\n',
    'cast_fn_20': 'enum E { E0 }; enum F { F0 }; struct S { int x; }; struct T { int x; };\ntypedef int (*Source)(int); typedef int (*Target)(int,int,...); Target convert(Source value) { return (Target)value; }\n',
    'cast_fn_21': 'enum E { E0 }; enum F { F0 }; struct S { int x; }; struct T { int x; };\ntypedef int (*Source)(int,...); typedef int (*Target)(int,int,...); Target convert(Source value) { return (Target)value; }\n',
    'cast_fn_22': 'enum E { E0 }; enum F { F0 }; struct S { int x; }; struct T { int x; };\ntypedef int (*Source)(int,int,...); typedef int (*Target)(int,...); Target convert(Source value) { return (Target)value; }\n',
    'cast_fn_23': 'enum E { E0 }; enum F { F0 }; struct S { int x; }; struct T { int x; };\ntypedef int (*Source)(enum E); typedef int (*Target)(unsigned); Target convert(Source value) { return (Target)value; }\n',
    'cast_fn_24': 'enum E { E0 }; enum F { F0 }; struct S { int x; }; struct T { int x; };\ntypedef int (*Source)(enum E); typedef int (*Target)(enum F); Target convert(Source value) { return (Target)value; }\n',
    'cast_fn_25': 'enum E { E0 }; enum F { F0 }; struct S { int x; }; struct T { int x; };\ntypedef struct S (*Source)(int); typedef struct T (*Target)(int); Target convert(Source value) { return (Target)value; }\n',
    'cast_fn_26': 'enum E { E0 }; enum F { F0 }; struct S { int x; }; struct T { int x; };\ntypedef int (*Source)(char); typedef int (*Target)(signed char); Target convert(Source value) { return (Target)value; }\n',
    'cast_fn_27': 'enum E { E0 }; enum F { F0 }; struct S { int x; }; struct T { int x; };\ntypedef double (*Source)(int); typedef long double (*Target)(int); Target convert(Source value) { return (Target)value; }\n',
    'cast_long_int': 'typedef long (*Source)(int); typedef int (*Target)(int); Target convert(Source value) { return (Target)value; }\n',
    'prototype_missing': 'int f(void) { return 0; }\n',
    'prototype_old': 'int f(); int f(void) { return 0; }\n',
    'prototype_present': 'int f(void); int f(void) { return 0; }\n',
    'prototype_static': 'static int f(void) { return 0; }\n',
    'prototype_inherited_static': 'static int f(); int f(void) { return 0; }\n',
    'prototype_inline': 'inline int f(void) { return 0; }\n',
    'prototype_extern_inline': 'extern inline int f(void) { return 0; }\n',
    'prototype_main': 'int main(void) { return 0; }\n',
    'prototype_builtin': 'void *malloc(unsigned long n) { return (void*)0; }\n',
    'prototype_nested': 'int outer(void); int outer(void) { int inner(void) { return 1; } return inner(); }\n',
    'prototype_block': 'void g(void); void g(void) { extern int f(void); } int f(void) { return 0; }\n',

    'undef': '#if MISSING\nint x;\n#endif\nint y;\n',
    'undef_macro': '#define ALIAS MISSING\n#if ALIAS\n#endif\nint y;\n',
    'undef_short': '#if 0 && QUIET_A\n#endif\n#if 1 || QUIET_B\n#endif\nint y;\n',
    'undef_ternary': '#if 1 ? 7 : QUIET\n#endif\nint y;\n',
    'undef_taken': '#if 0 ? QUIET : MISSING\n#endif\nint y;\n',
    'undef_restore': '#if (1 || QUIET) && MISSING\n#endif\nint y;\n',
    'undef_inactive': '#if 0\n#if QUIET_A\n#endif\n#elif 1\n#elif QUIET_B\n#endif\nint y;\n',
    'undef_true': '#if true\n#endif\nint y;\n',
    'undef_defined': '#if defined(MISSING)\n#endif\nint y;\n',
    'undef_wide_char': "#if L'a' != 97 || U'\\x1234' != 0x1234\n#error incorrect wide character value\n#endif\nint y;\n",
    'unused': 'int f(void) { int unused_local; return 0; }\n',
    'sign': 'int f(int s, unsigned u) { return s < u; }\n',
    'cpp': '#warning sentinel\nint x;\n',
    'implicit': 'int f(void) { return missing(); }\n',
    'implicit_int': 'f(void) { return 0; }\n',
    'return': 'int f(int n) { if (n) return 1; }\n',
    'bare': 'int f(void) { return; }\n',
    'conversion': 'int *f(void) { return 3; }\n',
}
# name, source, dialect, options, leaf label, expected classification
FIXTURES.update({
    'old_empty': 'int f() { return 0; }\n',
    'old_prior': 'int f(void); int f() { return 0; }\n',
    'old_static': 'static int f() { return 0; }\n',
    'old_kr': 'int f(x) int x; { return x; }\n',
    'old_prototype': 'int f(int x) { return x; }\n',
    'old_decl': 'int f();\n',
    'old_ignored': '#pragma GCC diagnostic ignored "-Wold-style-definition"\nint f() {return 0;}\n',
    'old_promoted': '#pragma GCC diagnostic error "-Wold-style-definition"\nint f() {return 0;}\n',
})

CASES = (
    ('old_empty', 'old_empty', 'gnu18', ('-Wold-style-definition',), 'old-style-definition', 'W'),
    ('old_prior', 'old_prior', 'gnu18', ('-Wold-style-definition',), 'old-style-definition', 'W'),
    ('old_static', 'old_static', 'gnu18', ('-Wold-style-definition',), 'old-style-definition', 'W'),
    ('old_kr', 'old_kr', 'gnu18', ('-Wold-style-definition',), 'old-style-definition', 'W'),
    ('old_prototype', 'old_prototype', 'gnu18', ('-Wold-style-definition',), 'old-style-definition', '-'),
    ('old_decl', 'old_decl', 'gnu18', ('-Wold-style-definition',), 'old-style-definition', '-'),
    ('old_ignored', 'old_ignored', 'gnu18', ('-Wold-style-definition',), 'old-style-definition', '-'),
    ('old_promoted', 'old_promoted', 'gnu18', ('-Wold-style-definition',), 'old-style-definition', 'E'),
    ('old-style-default', 'old_empty', 'gnu18', (), 'old-style-definition', '-'),
    ('old-style-wall', 'old_empty', 'gnu18', ('-Wall',), 'old-style-definition', '-'),
    ('old-style-extra', 'old_empty', 'gnu18', ('-Wextra',), 'old-style-definition', '-'),
    ('old-style-c23-empty', 'old_empty', 'gnu23', ('-Wold-style-definition',), 'old-style-definition', '-'),
    ('old-style-error', 'old_empty', 'gnu18', ('-Werror=old-style-definition',), 'old-style-definition', 'E'),
    ('old-style-demote', 'old_empty', 'gnu18', ('-Werror=old-style-definition', '-Wno-error=old-style-definition'), 'old-style-definition', 'W'),
    ('old-style-off', 'old_empty', 'gnu18', ('-Wold-style-definition', '-Wno-old-style-definition'), 'old-style-definition', '-'),
    ('old-style-quiet', 'old_empty', 'gnu18', ('-w', '-Werror=old-style-definition'), 'old-style-definition', '-'),
    ('cast-function-0', 'cast_fn_0', 'gnu18', ('-Wcast-function-type',), 'cast-function-type', 'W'),
    ('cast-function-1', 'cast_fn_1', 'gnu18', ('-Wcast-function-type',), 'cast-function-type', '-'),
    ('cast-function-2', 'cast_fn_2', 'gnu18', ('-Wcast-function-type',), 'cast-function-type', '-'),
    ('cast-function-3', 'cast_fn_3', 'gnu18', ('-Wcast-function-type',), 'cast-function-type', '-'),
    ('cast-function-4', 'cast_fn_4', 'gnu18', ('-Wcast-function-type',), 'cast-function-type', '-'),
    ('cast-function-5', 'cast_fn_5', 'gnu18', ('-Wcast-function-type',), 'cast-function-type', 'W'),
    ('cast-function-6', 'cast_fn_6', 'gnu18', ('-Wcast-function-type',), 'cast-function-type', '-'),
    ('cast-function-7', 'cast_fn_7', 'gnu18', ('-Wcast-function-type',), 'cast-function-type', '-'),
    ('cast-function-8', 'cast_fn_8', 'gnu18', ('-Wcast-function-type',), 'cast-function-type', '-'),
    ('cast-function-9', 'cast_fn_9', 'gnu18', ('-Wcast-function-type',), 'cast-function-type', '-'),
    ('cast-function-10', 'cast_fn_10', 'gnu18', ('-Wcast-function-type',), 'cast-function-type', 'W'),
    ('cast-function-11', 'cast_fn_11', 'gnu18', ('-Wcast-function-type',), 'cast-function-type', 'W'),
    ('cast-function-12', 'cast_fn_12', 'gnu18', ('-Wcast-function-type',), 'cast-function-type', 'W'),
    ('cast-function-13', 'cast_fn_13', 'gnu18', ('-Wcast-function-type',), 'cast-function-type', '-'),
    ('cast-function-14', 'cast_fn_14', 'gnu18', ('-Wcast-function-type',), 'cast-function-type', 'W'),
    ('cast-function-15', 'cast_fn_15', 'gnu18', ('-Wcast-function-type',), 'cast-function-type', 'W'),
    ('cast-function-16', 'cast_fn_16', 'gnu18', ('-Wcast-function-type',), 'cast-function-type', '-'),
    ('cast-function-17', 'cast_fn_17', 'gnu18', ('-Wcast-function-type',), 'cast-function-type', 'W'),
    ('cast-function-18', 'cast_fn_18', 'gnu18', ('-Wcast-function-type',), 'cast-function-type', 'W'),
    ('cast-function-19', 'cast_fn_19', 'gnu18', ('-Wcast-function-type',), 'cast-function-type', '-'),
    ('cast-function-20', 'cast_fn_20', 'gnu18', ('-Wcast-function-type',), 'cast-function-type', 'W'),
    ('cast-function-21', 'cast_fn_21', 'gnu18', ('-Wcast-function-type',), 'cast-function-type', '-'),
    ('cast-function-22', 'cast_fn_22', 'gnu18', ('-Wcast-function-type',), 'cast-function-type', '-'),
    ('cast-function-23', 'cast_fn_23', 'gnu18', ('-Wcast-function-type',), 'cast-function-type', '-'),
    ('cast-function-24', 'cast_fn_24', 'gnu18', ('-Wcast-function-type',), 'cast-function-type', '-'),
    ('cast-function-25', 'cast_fn_25', 'gnu18', ('-Wcast-function-type',), 'cast-function-type', 'W'),
    ('cast-function-26', 'cast_fn_26', 'gnu18', ('-Wcast-function-type',), 'cast-function-type', '-'),
    ('cast-function-27', 'cast_fn_27', 'gnu18', ('-Wcast-function-type',), 'cast-function-type', 'W'),
    ('cast-width-lp64', 'cast_long_int', 'gnu18', ('-m64', '-Wcast-function-type'), 'cast-function-type', 'W'),
    ('cast-width-ilp32', 'cast_long_int', 'gnu18', ('-m32', '-Wcast-function-type'), 'cast-function-type', '-'),
    ('cast-function-default', 'cast_fn_0', 'gnu18', (), 'cast-function-type', '-'),
    ('cast-function-wall', 'cast_fn_0', 'gnu18', ('-Wall',), 'cast-function-type', '-'),
    ('cast-function-extra', 'cast_fn_0', 'gnu18', ('-Wextra',), 'cast-function-type', 'W'),
    ('cast-function-error', 'cast_fn_0', 'gnu18', ('-Werror=cast-function-type',), 'cast-function-type', 'E'),
    ('cast-function-leaf-disabled', 'cast_fn_0', 'gnu18', ('-Wextra', '-Wno-cast-function-type'), 'cast-function-type', '-'),
    ('cast-function-demoted', 'cast_fn_0', 'gnu18', ('-Werror=cast-function-type', '-Wno-error=cast-function-type'), 'cast-function-type', 'W'),
    ('cast-function-global-error', 'cast_fn_0', 'gnu18', ('-Wextra', '-Werror'), 'cast-function-type', 'E'),
    ('missing-prototypes-default', 'prototype_missing', 'gnu18', (), 'missing-prototypes', '-'),
    ('missing-prototypes-groups', 'prototype_missing', 'gnu18', ('-Wall', '-Wextra'), 'missing-prototypes', '-'),
    ('missing-prototypes-on', 'prototype_missing', 'gnu18', ('-Wmissing-prototypes',), 'missing-prototypes', 'W'),
    ('missing-prototypes-error', 'prototype_missing', 'gnu18', ('-Werror=missing-prototypes',), 'missing-prototypes', 'E'),
    ('missing-prototypes-off', 'prototype_missing', 'gnu18', ('-Wmissing-prototypes', '-Wno-missing-prototypes'), 'missing-prototypes', '-'),
    ('missing-prototypes-demote', 'prototype_missing', 'gnu18', ('-Werror=missing-prototypes', '-Wno-error=missing-prototypes'), 'missing-prototypes', 'W'),
    ('missing-prototypes-declared', 'prototype_present', 'gnu18', ('-Wmissing-prototypes',), 'missing-prototypes', '-'),
    ('missing-prototypes-old', 'prototype_old', 'gnu18', ('-Wmissing-prototypes',), 'missing-prototypes', 'W'),
    ('missing-prototypes-static', 'prototype_static', 'gnu18', ('-Wmissing-prototypes',), 'missing-prototypes', '-'),
    ('missing-prototypes-inherited-static', 'prototype_inherited_static', 'gnu18', ('-Wmissing-prototypes',), 'missing-prototypes', 'W'),
    ('missing-prototypes-inline', 'prototype_inline', 'gnu18', ('-Wmissing-prototypes',), 'missing-prototypes', '-'),
    ('missing-prototypes-extern-inline', 'prototype_extern_inline', 'gnu18', ('-Wmissing-prototypes',), 'missing-prototypes', '-'),
    ('missing-prototypes-main', 'prototype_main', 'gnu18', ('-Wmissing-prototypes',), 'missing-prototypes', '-'),
    ('missing-prototypes-builtin', 'prototype_builtin', 'gnu18', ('-Wmissing-prototypes',), 'missing-prototypes', 'W'),
    ('missing-prototypes-nested', 'prototype_nested', 'gnu18', ('-Wmissing-prototypes',), 'missing-prototypes', '-'),
    ('missing-prototypes-block', 'prototype_block', 'gnu18', ('-Wmissing-prototypes',), 'missing-prototypes', 'W'),
    ('missing-declarations-default', 'prototype_missing', 'gnu18', (), 'missing-declarations', '-'),
    ('missing-declarations-groups', 'prototype_missing', 'gnu18', ('-Wall', '-Wextra'), 'missing-declarations', '-'),
    ('missing-declarations-on', 'prototype_missing', 'gnu18', ('-Wmissing-declarations',), 'missing-declarations', 'W'),
    ('missing-declarations-error', 'prototype_missing', 'gnu18', ('-Werror=missing-declarations',), 'missing-declarations', 'E'),
    ('missing-declarations-off', 'prototype_missing', 'gnu18', ('-Wmissing-declarations', '-Wno-missing-declarations'), 'missing-declarations', '-'),
    ('missing-declarations-demote', 'prototype_missing', 'gnu18', ('-Werror=missing-declarations', '-Wno-error=missing-declarations'), 'missing-declarations', 'W'),
    ('missing-declarations-declared', 'prototype_present', 'gnu18', ('-Wmissing-declarations',), 'missing-declarations', '-'),
    ('missing-declarations-old', 'prototype_old', 'gnu18', ('-Wmissing-declarations',), 'missing-declarations', '-'),
    ('missing-declarations-static', 'prototype_static', 'gnu18', ('-Wmissing-declarations',), 'missing-declarations', '-'),
    ('missing-declarations-inherited-static', 'prototype_inherited_static', 'gnu18', ('-Wmissing-declarations',), 'missing-declarations', '-'),
    ('missing-declarations-inline', 'prototype_inline', 'gnu18', ('-Wmissing-declarations',), 'missing-declarations', '-'),
    ('missing-declarations-extern-inline', 'prototype_extern_inline', 'gnu18', ('-Wmissing-declarations',), 'missing-declarations', '-'),
    ('missing-declarations-main', 'prototype_main', 'gnu18', ('-Wmissing-declarations',), 'missing-declarations', '-'),
    ('missing-declarations-builtin', 'prototype_builtin', 'gnu18', ('-Wmissing-declarations',), 'missing-declarations', 'W'),
    ('missing-declarations-nested', 'prototype_nested', 'gnu18', ('-Wmissing-declarations',), 'missing-declarations', '-'),
    ('missing-declarations-block', 'prototype_block', 'gnu18', ('-Wmissing-declarations',), 'missing-declarations', 'W'),
    ('prototype-preferred', 'prototype_missing', 'gnu18', ('-Wmissing-prototypes', '-Wmissing-declarations'), 'missing-prototypes', 'W'),
    ('declaration-suppressed', 'prototype_missing', 'gnu18', ('-Wmissing-prototypes', '-Wmissing-declarations'), 'missing-declarations', '-'),
    ('undef-default-off', 'undef', 'gnu23', (), 'undef', '-'),
    ('undef-wall-does-not-enable', 'undef', 'gnu23', ('-Wall', '-Wextra'), 'undef', '-'),
    ('undef-enabled', 'undef', 'gnu23', ('-Wundef',), 'undef', 'W'),
    ('undef-error', 'undef', 'gnu23', ('-Werror=undef',), 'undef', 'E'),
    ('undef-disabled', 'undef', 'gnu23', ('-Wundef', '-Wno-undef'), 'undef', '-'),
    ('undef-global-error', 'undef', 'gnu23', ('-Wundef', '-Werror'), 'undef', 'E'),
    ('undef-demotion', 'undef', 'gnu23', ('-Werror=undef', '-Wno-error=undef'), 'undef', 'W'),
    ('undef-inhibit-all', 'undef', 'gnu23', ('-Werror=undef', '-w'), 'undef', '-'),
    ('undef-macro-expansion', 'undef_macro', 'gnu23', ('-Wundef',), 'undef', 'W'),
    ('undef-short-circuit', 'undef_short', 'gnu23', ('-Werror=undef',), 'undef', '-'),
    ('undef-ternary-inactive', 'undef_ternary', 'gnu23', ('-Werror=undef',), 'undef', '-'),
    ('undef-ternary-active', 'undef_taken', 'gnu23', ('-Wundef',), 'undef', 'W'),
    ('undef-quiet-restored', 'undef_restore', 'gnu23', ('-Wundef',), 'undef', 'W'),
    ('undef-inactive-directives', 'undef_inactive', 'gnu23', ('-Werror=undef',), 'undef', '-'),
    ('undef-c23-true', 'undef_true', 'gnu23', ('-Werror=undef',), 'undef', '-'),
    ('undef-pre-c23-true', 'undef_true', 'gnu11', ('-Wundef',), 'undef', 'W'),
    ('undef-defined-operand', 'undef_defined', 'gnu23', ('-Werror=undef',), 'undef', '-'),
    ('undef-wide-character-token', 'undef_wide_char', 'gnu23', ('-Werror=undef',), 'undef', '-'),
    ('default-unused', 'unused', 'gnu23', (), 'unused-variable', '-'),
    ('wall-unused', 'unused', 'gnu23', ('-Wall',), 'unused-variable', 'W'),
    ('leaf-negative-before-wall', 'unused', 'gnu23', ('-Wno-unused-variable', '-Wall'), 'unused-variable', '-'),
    ('leaf-negative-after-wall', 'unused', 'gnu23', ('-Wall', '-Wno-unused-variable'), 'unused-variable', '-'),
    ('group-wall-disable', 'unused', 'gnu23', ('-Wall', '-Wno-all'), 'unused-variable', '-'),
    ('group-wall-enable', 'unused', 'gnu23', ('-Wno-all', '-Wall'), 'unused-variable', 'W'),
    ('subgroup-off-before-wall', 'unused', 'gnu23', ('-Wno-unused', '-Wall'), 'unused-variable', '-'),
    ('subgroup-off-after-wall', 'unused', 'gnu23', ('-Wall', '-Wno-unused'), 'unused-variable', '-'),
    ('subgroup-on-before-wall-off', 'unused', 'gnu23', ('-Wunused', '-Wno-all'), 'unused-variable', 'W'),
    ('subgroup-on-after-wall-off', 'unused', 'gnu23', ('-Wno-all', '-Wunused'), 'unused-variable', 'W'),
    ('leaf-on-before-subgroup-off', 'unused', 'gnu23', ('-Wunused-variable', '-Wno-unused'), 'unused-variable', 'W'),
    ('leaf-on-after-subgroup-off', 'unused', 'gnu23', ('-Wno-unused', '-Wunused-variable'), 'unused-variable', 'W'),
    ('leaf-off-before-subgroup-on', 'unused', 'gnu23', ('-Wno-unused-variable', '-Wunused'), 'unused-variable', '-'),
    ('leaf-off-after-subgroup-on', 'unused', 'gnu23', ('-Wunused', '-Wno-unused-variable'), 'unused-variable', '-'),
    ('extra-same-level-disable', 'sign', 'gnu23', ('-Wextra', '-Wno-extra'), 'sign-compare', '-'),
    ('extra-same-level-enable', 'sign', 'gnu23', ('-Wno-extra', '-Wextra'), 'sign-compare', 'W'),
    ('sign-leaf-before-extra', 'sign', 'gnu23', ('-Wno-sign-compare', '-Wextra'), 'sign-compare', '-'),
    ('sign-leaf-after-extra', 'sign', 'gnu23', ('-Wextra', '-Wno-sign-compare'), 'sign-compare', '-'),
    ('default-cpp-survives-no-all', 'cpp', 'gnu23', ('-Wno-all',), 'cpp', 'W'),
    ('cpp-leaf-negative-before-wall', 'cpp', 'gnu23', ('-Wno-cpp', '-Wall'), 'cpp', '-'),
    ('inhibit-all', 'cpp', 'gnu23', ('-w', '-Wall'), 'cpp', '-'),
    ('default-return-type-off', 'return', 'gnu23', (), 'return-type', '-'),
    ('return-type-by-wall', 'return', 'gnu23', ('-Wall',), 'return-type', 'W'),
    ('return-type-off-before-wall', 'return', 'gnu23', ('-Wno-return-type', '-Wall'), 'return-type', '-'),
    ('default-implicit-c23', 'implicit', 'gnu23', (), 'implicit-function-declaration', 'E'),
    ('default-implicit-gnu90', 'implicit', 'gnu90', (), 'implicit-function-declaration', '-'),
    ('implicit-via-wall-gnu90', 'implicit', 'gnu90', ('-Wall',), 'implicit-function-declaration', 'W'),
    ('implicit-subgroup', 'implicit', 'gnu90', ('-Wimplicit',), 'implicit-function-declaration', 'W'),
    ('implicit-off-before-wall', 'implicit', 'gnu90', ('-Wno-implicit', '-Wall'), 'implicit-function-declaration', '-'),
    ('implicit-on-before-no-all', 'implicit', 'gnu90', ('-Wimplicit', '-Wno-all'), 'implicit-function-declaration', 'W'),
    ('implicit-c23-no-all', 'implicit', 'gnu23', ('-Wno-all',), 'implicit-function-declaration', '-'),
    ('implicit-int-subgroup', 'implicit_int', 'gnu90', ('-Wimplicit',), 'implicit-int', 'W'),
    ('implicit-int-wall-return-type', 'implicit_int', 'gnu90', ('-Wall', '-Wno-implicit-int'), 'return-type', 'W'),
    ('implicit-int-no-return-type', 'implicit_int', 'gnu90', ('-Wall', '-Wno-return-type'), 'implicit-int', 'W'),
    ('bare-gnu90-default', 'bare', 'gnu90', (), 'return-mismatch', '-'),
    ('bare-gnu90-wall', 'bare', 'gnu90', ('-Wall',), 'return-mismatch', 'W'),
    ('bare-gnu90-no-all', 'bare', 'gnu90', ('-Wno-all',), 'return-mismatch', 'W'),
    ('bare-gnu90-no-return-type', 'bare', 'gnu90', ('-Wno-return-type',), 'return-mismatch', 'W'),
    ('bare-gnu90-disabled-mismatch', 'bare', 'gnu90', ('-Wno-return-type', '-Wno-return-mismatch'), 'return-mismatch', '-'),
    ('error-leaf-disable', 'unused', 'gnu23', ('-Werror=unused-variable', '-Wno-unused-variable'), 'unused-variable', '-'),
    ('error-leaf-reenable', 'unused', 'gnu23', ('-Wno-unused-variable', '-Werror=unused-variable'), 'unused-variable', 'E'),
    ('error-group-respects-leaf-disable', 'unused', 'gnu23', ('-Wno-unused-variable', '-Werror=all'), 'unused-variable', '-'),
    ('error-leaf-beats-group-disable', 'unused', 'gnu23', ('-Werror=unused-variable', '-Wno-all'), 'unused-variable', 'E'),
    ('demote-after-group-error', 'unused', 'gnu23', ('-Werror=all', '-Wno-error=unused-variable'), 'unused-variable', 'W'),
    ('group-error-after-leaf-demote', 'unused', 'gnu23', ('-Wno-error=unused-variable', '-Werror=all'), 'unused-variable', 'E'),
    ('group-demote-not-leaf', 'unused', 'gnu23', ('-Werror=all', '-Wno-error=all'), 'unused-variable', 'E'),
    ('error-global-demote-before', 'unused', 'gnu23', ('-Wno-error=unused-variable', '-Werror', '-Wall'), 'unused-variable', 'W'),
    ('error-global-demote-after', 'unused', 'gnu23', ('-Werror', '-Wno-error=unused-variable', '-Wall'), 'unused-variable', 'W'),
    ('c99-default-error-no-all', 'conversion', 'gnu23', ('-Wno-all',), 'int-conversion', 'E'),
    ('c99-default-error-with-w', 'conversion', 'gnu23', ('-w',), 'int-conversion', 'E'),
    ('c99-default-demotion', 'conversion', 'gnu23', ('-Wno-error=int-conversion',), 'int-conversion', 'W'),
    ('permissive-implicit', 'implicit', 'gnu23', ('-fpermissive',), 'implicit-function-declaration', 'W'),
    ('permissive-leaf-error', 'implicit', 'gnu23', ('-fpermissive', '-Werror=implicit-function-declaration'), 'implicit-function-declaration', 'E'),
    ('permissive-group-error', 'implicit', 'gnu23', ('-fpermissive', '-Werror=implicit', '-Wno-error=implicit'), 'implicit-function-declaration', 'E'),
    ('group-error-leaf-demotion', 'implicit', 'gnu23', ('-Werror=implicit', '-Wno-error=implicit-function-declaration'), 'implicit-function-declaration', 'W'),
    ('group-error-group-demotion', 'implicit', 'gnu23', ('-Werror=implicit', '-Wno-error=implicit'), 'implicit-function-declaration', 'E'),
    ('c99-error-demotion-with-w', 'conversion', 'gnu23', ('-w', '-Wno-error=int-conversion'), 'int-conversion', '-'),
)


def classification(stderr: str, leaf: str) -> str:
    pattern = re.compile(r':\s*(warning|error):[^\n]*\[-W(?:error=)?' + re.escape(leaf) + r'\]')
    hits = pattern.findall(stderr)
    if len(hits) > 1:
        raise RuntimeError(f'multiple {leaf} diagnoses: {stderr}')
    return {'warning': 'W', 'error': 'E'}.get(hits[0], '-') if hits else '-'


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--lccc', default='target/fastbuild/lccc')
    parser.add_argument('--gcc', default='gcc-16')
    args = parser.parse_args()
    gcc, lccc = shlex.split(args.gcc), shlex.split(args.lccc)
    if lccc and not Path(lccc[0]).is_absolute() and '/' in lccc[0]:
        lccc[0] = str((ROOT / lccc[0]).resolve())
    version = subprocess.run(gcc + ['-dumpfullversion'], text=True, capture_output=True, check=True, timeout=30).stdout.strip()
    if not re.match(r'^16\.2(?:\.|$)', version):
        raise RuntimeError(f'expected GCC 16.2, got {version!r}')
    failures=[]
    with tempfile.TemporaryDirectory(prefix='lccc-warning-precedence-') as tmp:
        src=Path(tmp)/'probe.c'
        obj=Path(tmp)/'probe.o'
        for label, fixture, dialect, flags, leaf, expected in CASES:
            modes = ('-c', '-E', '-fsyntax-only') if leaf == 'undef' else (('-c', '-fsyntax-only') if leaf.startswith('missing-') or leaf in ('cast-function-type', 'old-style-definition') else ('-c',))
            for mode in modes:
                src.write_text(FIXTURES[fixture], encoding='utf-8')
                results={}
                for name, compiler in [('gcc',gcc),('lccc',lccc)]:
                    proc=subprocess.run(compiler + [f'-std={dialect}', mode, '-o', str(obj), *flags, str(src)],
                                        cwd=ROOT, text=True, capture_output=True, timeout=30)
                    value=classification(proc.stderr,leaf)
                    # A diagnostic label must agree with success/failure. Missing
                    # expected errors or unrelated compile errors are not a pass.
                    valid=(proc.returncode != 0) == (value == 'E')
                    results[name]=(value,proc.returncode,proc.stderr,valid)
                if results['gcc'][0] != expected or not results['gcc'][3]:
                    failures.append(f'{label}: GCC expected {expected}, got {results["gcc"]}')
                elif results['lccc'][0] != expected or not results['lccc'][3]:
                    failures.append(f'{label}: LCCC expected {expected}, got {results["lccc"]}')
                else:
                    print(f'PASS {label}: {expected}')
    if failures:
        raise RuntimeError(f'{len(failures)}/{len(CASES)} cases failed:\n' + '\n'.join(failures))
    print(f'warning precedence: PASS ({len(CASES)} cases against GCC {version})')
    from check_diagnostic_pragmas import run as check_scopes
    check_scopes(lccc, gcc)
    from check_include_path_warnings import run as check_include_paths
    check_include_paths(lccc, gcc)
    from check_date_time_warnings import run as check_dates
    check_dates(lccc, gcc)
    return 0


if __name__ == '__main__':
    try:
        raise SystemExit(main())
    except (OSError, RuntimeError, subprocess.SubprocessError) as exc:
        raise SystemExit(f'warning precedence: ERROR: {exc}')
