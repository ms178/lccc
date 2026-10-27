#!/usr/bin/env python3
"""Independent negative/positive diagnostic contracts, across target drivers.

Expected severity and classification were probed against GCC 16.2.0. An exact
oracle, when supplied, is checked too; timeouts and crashes are never rejections.
"""
import argparse
import os
from pathlib import Path
import re
import subprocess
import tempfile

# name, source, hard read-only error, warning class (None for legal controls).
CASES = [
    ('typeof_record_field', 'struct S{typeof((struct{int x;}){1}) a;};void f(struct S*p){p->a.x=2;}', False, None),
    ('typeof_empty_record_field', 'struct S{typeof((struct{}){}) a;};', False, None),
    ('typeof_union_field', 'struct S{typeof((union{int x;}){1}) a;};void f(struct S*p){p->a.x=2;}', False, None),
    ('typeof_empty_union_field', 'struct S{typeof((union{}){}) a;};', False, None),
    ('recursive_named_field', 'struct S{struct S a;};void f(struct S*p){(void)p;}', True, None),
    ('incomplete_array_field', 'struct S;struct T{struct S a[2];};', True, None),
    ('legal_recursive_pointer', 'struct S{struct S *p;};void f(struct S*p){p->p=p;}', False, None),
    ('recursive_tag_is_not_member', 'struct S{struct S;};void f(struct S*p){p->z=1;}', True, None),
    ('mutable_outer_parameter', 'void f(int *const *p){p=0;}', False, None),
    ('const_outer_parameter', 'void f(int **const p){p=0;}', True, None),
    ('const_member', 'struct S{const int x;};void f(struct S*p){p->x=1;}', True, None),
    ('mutable_member_after_const', 'struct S{const int x;int y;};void f(struct S*p){p->y=1;}', False, None),
    ('const_member_pointer', 'struct S{int *const p;};void f(struct S*s){s->p=0;}', True, None),
    ('mutable_member_pointee', 'struct S{int *const p;};void f(struct S*s){*s->p=1;}', False, None),
    ('const_member_pointee', 'struct S{const int *p;};void f(struct S*s){*s->p=1;}', True, None),
    ('mutable_member_pointer', 'struct S{const int *p;};void f(struct S*s){s->p=0;}', False, None),
    ('const_anonymous_member', 'struct S{struct{const int x;};};void f(struct S*s){s->x=1;}', True, None),
    ('const_first_sibling', 'void f(void){int *const p=0,*q=0;p=0;(void)q;}', True, None),
    ('mutable_first_sibling', 'void f(void){int *p=0,*const q=0;p=0;(void)q;}', False, None),
    ('const_cast_initializer', 'void f(void){int *const p=(int*)0;p=0;}', True, None),
    ('const_reversed_subscript', 'void f(const int *p){0[p]=1;}', True, None),
    ('mutable_reversed_subscript', 'void f(int *p){0[p]=1;}', False, None),
    ('const_depth2', 'void f(const int **p){**p=1;}', True, None),
    ('const_depth3', 'void f(const int ***p){***p=1;}', True, None),
    ('const_array_deref', 'void f(void){const int a[1]={0};*a=1;}', True, None),
    ('const_cast', 'void f(int *p){*(const int*)p=1;}', True, None),
    ('mutable_middle', 'void f(const int **p){*p=0;(*p)++;}', False, None),
    ('mutable_parameter_array', 'void f(const int p[1]){p=0;}', False, None),
    ('const_parameter_element', 'void f(const int p[1]){p[0]=1;}', True, None),
    ('pointer_bool', '_Bool f(int *p){return p;}', False, None),
    ('integer_pointer_init', 'void f(void){int *p=1;(void)p;}', False, 'int-conversion'),
    ('pointer_integer_init', 'void f(int *p){int x=p;(void)x;}', False, 'int-conversion'),
    ('pointer_integer_assign', 'void f(int *p){int x;x=p;}', False, 'int-conversion'),
    ('pointer_integer_return', 'int f(int *p){return p;}', False, 'int-conversion'),
    ('pointer_integer_argument', 'void g(int);void f(int *p){g(p);}', False, 'int-conversion'),
    ('void_return_value', 'void f(void){return 1;}', False, 'return-mismatch'),
    ('missing_return_value', 'int f(void){return;}', False, 'return-mismatch'),
    ('implicit_int', 'f(void){return 0;}', False, 'implicit-int'),
    # Per-level pointer qualifiers. `CType` erases qualifiers, so before the
    # per-level record these middle-level consts were invisible and every
    # `*p = 0` below was silently accepted. Measured against gcc 16.2.0 and
    # clang 23.1.4: exactly the cases marked True are constraint violations.
    ('const_middle_level', 'void f(int *const *p){*p=0;}', True, None),
    ('mutable_middle_pointee', 'void f(int *const *p){**p=1;}', False, None),
    ('mutable_middle_object', 'void f(int *const *p){p=0;}', False, None),
    ('const_middle_element', 'void f(int *const *p){p[0]=0;}', True, None),
    ('const_middle_offset', 'void f(int *const *p){*(p+0)=0;}', True, None),
    ('const_middle_increment', 'void f(int *const *p){(*p)++;}', True, None),
    ('const_middle_compound', 'void f(int *const *p){*p+=0;}', True, None),
    ('const_third_level', 'void f(int **const *p){*p=0;}', True, None),
    ('const_third_level_inner', 'void f(int *const **p){**p=0;}', True, None),
    ('mutable_third_level', 'void f(int *const **p){*p=0;}', False, None),
    ('const_middle_local', 'void f(void){int *const *p=0;*p=0;}', True, None),
    ('const_middle_field', 'struct S{int *const *p;};void f(struct S*s){*s->p=0;}', True, None),
    ('mutable_field_pointee', 'struct S{int *const *p;};void f(struct S*s){**s->p=1;}', False, None),
    ('const_middle_typedef', 'typedef int *const cp;void f(cp *p){*p=0;}', True, None),
    ('const_typedef_object', 'typedef int *const cp;void f(void){cp x=0;x=0;}', True, None),
    ('mutable_typedef_pointer', 'typedef int *const cp;void f(cp p){*p=0;}', False, None),
    ('const_typedef_parameter', 'typedef int *const cp;void f(cp p){p=0;}', True, None),
    ('mutable_typedef_pointee', 'typedef int *const cp;void f(cp *p){**p=0;}', False, None),
    ('const_typedef_field', 'typedef int *const cp;struct S{cp *f;};void g(struct S*s){*s->f=0;}', True, None),
    ('mutable_typedef_field_pointee', 'typedef int *const cp;struct S{cp *f;};void g(struct S*s){**s->f=0;}', False, None),
    ('const_typedef_chain', 'typedef int *const cp;typedef cp *cpp;void f(cpp p){*p=0;}', True, None),
    ('const_middle_prefix_offset', 'void f(int *const *p){*(1+p)=0;}', True, None),
    # The array layer is transparent: the element type decides the level.
    ('const_array_element_pointee', 'void f(void){const int *a[2]={0};*a[0]=1;}', True, None),
    ('mutable_array_element_pointee', 'void f(void){int *a[2]={0};*a[0]=1;}', False, None),
    ('const_array_of_const_pointers', 'void f(void){int *const a[2]={0};a[0]=0;}', True, None),
    ('const_function_pointer_object', 'void f(void){int (*const fp)(void)=0;fp=0;}', True, None),
    ('mutable_function_pointer_object', 'void f(void){int (*fp)(void)=0;fp=0;}', False, None),
    # A cast type name keeps the qualifiers it spells; `CType` erases them, so
    # the cast node carries the levels (`*(int *const *)p = 0`).
    ('const_cast_middle_level', 'void f(void*p){*(int *const *)p=0;}', True, None),
    ('mutable_cast_middle_pointee', 'void f(void*p){**(int *const *)p=0;}', False, None),
    ('const_cast_three_levels', 'void f(void*p){*(int *const *const *)p=0;}', True, None),
    ('const_cast_inner_level', 'void f(void*p){**(int *const **)p=0;}', True, None),
    ('mutable_cast_plain', 'void f(void*p){*(int **)p=0;}', False, None),
    # A function's return type and a compound literal keep their own levels.
    ('const_function_return_level', 'int *const *g(void);void f(void){*g()=0;}', True, None),
    ('mutable_function_return_level', 'int **g(void);void f(void){*g()=0;}', False, None),
    ('const_function_return_alias', 'typedef int *const cp;cp *g(void);void f(void){*g()=0;}', True, None),
    ('mutable_function_return_pointee', 'int *const *g(void);void f(void){**g()=0;}', False, None),
    ('const_compound_literal_level', 'void f(void){*((int *const *){0})=0;}', True, None),
    ('mutable_compound_literal_level', 'void f(void){*((int **){0})=0;}', False, None),
]


def run(cc, source, std, flags):
    result = subprocess.run([cc, '-std=' + std, '-fsyntax-only', *flags, str(source)],
                            capture_output=True, text=True, timeout=20)
    if result.returncode not in (0, 1) or re.search(
            r'panicked at|internal compiler error|unrecognized command-line', result.stderr):
        raise AssertionError((cc, result.returncode, result.stderr))
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--ccc', default=os.environ.get('CCC', 'target/fastbuild/lccc'))
    parser.add_argument('--gcc', default=os.environ.get('GCC', 'gcc-16'))
    args = parser.parse_args()
    cc = str(Path(args.ccc).resolve())
    oracle = None
    try:
        version = subprocess.check_output([args.gcc, '-dumpfullversion'], text=True).strip()
        if version == '16.2.0':
            oracle = args.gcc
        else:
            print(f'Exact GCC comparison unavailable: found {version}; static contracts still run')
    except FileNotFoundError:
        print('Exact GCC comparison unavailable; static contracts still run')
    count = 0
    with tempfile.TemporaryDirectory(prefix='diagnostic-redteam-') as tmp:
        for name, source, hard, kind in CASES:
            path = Path(tmp) / (name + '.c')
            path.write_text(source)
            for std in ('gnu90', 'gnu17', 'gnu23'):
                options = [[], ['-w']]
                if kind:
                    options += [['-Wno-error=' + kind], ['-Wno-' + kind],
                                ['-fpermissive'], ['-fpermissive', '-Werror'],
                                ['-fpermissive', '-w', '-Werror'],
                                ['-fpermissive', '-Werror=' + kind],
                                ['-Werror=' + kind, '-fpermissive'],
                                ['-fpermissive', '-fno-permissive']]
                if kind in ('implicit-int', 'return-mismatch'):
                    options += [['-Wall'], ['-Wreturn-type'], ['-Wall', '-Werror']]
                for flags in options:
                    enabled = bool(kind and '-Wno-' + kind not in flags)
                    permissive_flag = next((f for f in reversed(flags) if f in ('-fpermissive', '-fno-permissive')), '')
                    permissive = permissive_flag == '-fpermissive'
                    legacy = kind in ('implicit-int', 'return-mismatch') and std == 'gnu90' and (
                        '-Wall' in flags or '-Wreturn-type' in flags)
                    explicitly_enabled = kind and '-Werror=' + kind in flags
                    warn_live = enabled and (std != 'gnu90' or kind == 'int-conversion'
                                             or name == 'void_return_value' or legacy or (explicitly_enabled and kind != 'return-mismatch'))
                    default_error = warn_live and (std != 'gnu90' or permissive_flag == '-fno-permissive') and not permissive and not any(
                        flag.startswith('-Wno-error=') for flag in flags)
                    promoted = warn_live and ('-Werror' in flags or explicitly_enabled) and '-w' not in flags
                    expected = int(hard or default_error or promoted)
                    result = run(cc, path, std, flags)
                    assert result.returncode == expected, (name, std, flags, expected, result.stderr)
                    if oracle:
                        ref = run(oracle, path, std, flags)
                        assert ref.returncode == expected, ('oracle drift', name, std, flags, ref.stderr)
                    if name.startswith('const_depth'):
                        spelling = '**p' if name == 'const_depth2' else '***p'
                        assert f"read-only location '{spelling}'" in result.stderr, result.stderr
                    # Each kind-aware conversion is diagnosed once, not once in
                    # the funnel and again in the old per-context checks.
                    label_kind = 'return-type' if legacy and kind == 'implicit-int' else kind
                    labels = re.findall(r'\[-W(?:error=)?' + str(label_kind) + r'\]', result.stderr) if kind else []
                    wanted_labels = int(warn_live and ('-w' not in flags or default_error))
                    assert len(labels) == wanted_labels, (name, std, flags, labels, result.stderr)
                    count += 1
    print(f'PASS {count} diagnostic red-team contracts' + (' with exact GCC 16.2.0' if oracle else ''))


if __name__ == '__main__':
    main()
