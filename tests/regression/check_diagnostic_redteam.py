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
# Each combo asserts: our exit code, our warning label count, and the VERBATIM
# first diagnostic (message + location spelling) against exact GCC 16.2.0.
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
    # Pointer-to-integer casts: GCC 16.2 warns by default in EVERY dialect
    # (never as a dialect default error) and only when the integer target's
    # size differs from the pointer's; _Bool and enumerations never warn
    # (measured).
    ('pointer_to_int_cast', 'void f(int *p){int x=(int)p;(void)x;}', False, 'pointer-to-int-cast'),
    ('pointer_to_int_cast_unsigned', 'void f(int *p){unsigned x=(unsigned)p;(void)x;}', False, 'pointer-to-int-cast'),
    ('pointer_to_int_cast_short', 'void f(int *p){short x=(short)p;(void)x;}', False, 'pointer-to-int-cast'),
    ('pointer_to_int_cast_array', 'void f(void){int a[1]={0};int x=(int)a;(void)x;}', False, 'pointer-to-int-cast'),
    ('pointer_to_int_cast_wide', 'void f(int *p){long x=(long)p;(void)x;}', False, None),
    ('pointer_to_int_cast_bool', '_Bool f(int *p){return (_Bool)p;}', False, None),
    ('pointer_to_int_cast_int_literal', 'void f(void){int x=(int)0;(void)x;}', False, None),
    # C11 6.5.4p2: a pointer converts only to an integer type (measured:
    # -w does not silence the constraint).
    ('pointer_float_constraint', 'void f(int *p){double d=(double)p;(void)d;}', True, None),
    ('pointer_complex_constraint', 'void f(int *p){float _Complex c=(float _Complex)p;(void)c;}', True, None),
    ('float_pointer_constraint', 'void f(void){char* a;double b;a=(char*)b;}', True, None),
    # Scalar typedef base constness: `CI x` is a read-only object and
    # `CI a[1]` has read-only elements, chains included.
    ('typedef_const_object', 'typedef const int CI;void f(void){CI x=0;x=1;}', True, None),
    ('typedef_const_parameter', 'typedef const int CI;void f(CI p){p=0;}', True, None),
    ('typedef_const_array', 'typedef const int CI;void f(void){CI a[1]={0};*a=1;}', True, None),
    ('typedef_star_object', 'typedef int *const PC;void f(void){PC q=0;q=0;}', True, None),
    ('typedef_pointee_writable', 'typedef const int *IP;void f(void){IP p=0;p=0;}', False, None),
    # `const` on a pointer typedef qualifies the pointer itself, not its
    # pointee (Expat 2.9.0's `const XML_Parser rootParser` is one real use).
    # Also retain the aliased level when a later star creates a pointer to it.
    ('typedef_qual_parameter_base', 'typedef int *P;void f(const P p){*p=1;}', False, None),
    ('typedef_qual_parameter_object', 'typedef int *P;void f(const P p){p=0;}', True, None),
    ('typedef_qual_middle_base', 'typedef int *P;void f(const P *p){**p=1;}', False, None),
    ('typedef_qual_middle_object', 'typedef int *P;void f(const P *p){*p=0;}', True, None),
    ('typedef_qual_chain_base', 'typedef int *P;typedef const P CP;void f(CP p){*p=1;}', False, None),
    ('typedef_qual_chain_middle', 'typedef int *P;typedef const P CP;void f(CP *p){*p=0;}', True, None),
    ('typedef_qual_derived_chain', 'typedef int *P;typedef const P *Q;void f(Q p){*p=0;}', True, None),
    ('typedef_qual_struct_base', 'struct S{int x;};typedef struct S *P;void f(const P p){p->x=1;}', False, None),
    ('typedef_qual_nested_base', 'struct A{int x;};struct S{struct A a;};typedef struct S *P;void f(const P p){p->a.x=1;}', False, None),
    ('typedef_qual_field_base', 'typedef int *P;struct S{const P p;};void f(struct S*s){*s->p=1;}', False, None),
    ('typedef_qual_field_object', 'typedef int *P;struct S{const P p;};void f(struct S*s){s->p=0;}', True, None),
    ('typedef_qual_field_middle', 'typedef int *P;struct S{const P *p;};void f(struct S*s){*s->p=0;}', True, None),
    ('typedef_qual_field_middle_base', 'typedef int *P;struct S{const P *p;};void f(struct S*s){**s->p=1;}', False, None),
    ('typedef_qual_cast_base', 'typedef int *P;void f(int*q){*(const P)q=1;}', False, None),
    ('typedef_qual_cast_middle', 'typedef int *P;void f(int**q){*(const P *)q=0;}', True, None),
    ('typedef_qual_cast_middle_base', 'typedef int *P;void f(int**q){**(const P *)q=1;}', False, None),
    ('typedef_qual_literal_middle', 'typedef int *P;void f(void){*(const P *){0}=0;}', True, None),
    ('typedef_qual_literal_base', 'typedef int *P;void f(void){**(const P *){0}=1;}', False, None),
    ('typedef_qual_pointer_to_array', 'typedef int *(*P)[2];void f(const P *p){*p=0;}', True, None),
    ('typedef_qual_pointer_to_array_base', 'typedef int *(*P)[2];void f(const P p){p[0][0]=0;}', False, None),
    ('typedef_qual_return_middle', 'typedef int *P;const P *g(void);void f(void){*g()=0;}', True, None),
    ('typedef_qual_return_base', 'typedef int *P;const P *g(void);void f(void){**g()=1;}', False, None),
    ('typedef_qual_array_base', 'typedef int *A[2];void f(void){int x;const A a={&x};*a[0]=1;}', False, None),
    ('typedef_qual_array_element', 'typedef int *A[2];void f(void){int x;const A a={&x};a[0]=0;}', True, None),
    ('typedef_qual_double_middle', 'typedef int **P;void f(const P *p){*p=0;}', True, None),
    ('typedef_qual_double_inner', 'typedef int **P;void f(const P *p){**p=0;}', False, None),
    # An unqualified aliased pointer still occupies a level when a later
    # declarator supplies its own qualifiers.
    ('typedef_unqualified_level_padding', 'typedef int *P;void f(P *const *p){*p=0;}', True, None),
    ('typedef_unqualified_level_base', 'typedef int *P;void f(P *const *p){**p=0;}', False, None),
    ('typedef_const_field_alias', 'typedef int *P;typedef const P CP;struct S{CP p;};void f(struct S*s){s->p=0;}', True, None),
    ('typedef_const_field_alias_base', 'typedef int *P;typedef const P CP;struct S{CP p;};void f(struct S*s){*s->p=0;}', False, None),
    ('typedef_const_field_alias_star', 'typedef int *P;typedef const P CP;struct S{CP *p;};void f(struct S*s){s->p=0;}', False, None),
    ('typedef_const_field_alias_array', 'typedef int *P;typedef const P CP;struct S{CP *p[2];};void f(struct S*s){s->p[0]=0;}', False, None),
    # Array transparency in the read-only walk: dims, stars and the
    # element type decide the level (measured against GCC 16.2).
    ('multidim_array_deref', 'void f(void){const int a[1][1]={{0}};**a=1;}', True, None),
    ('array_of_pointers_deref', 'void f(void){const int *ap[1]={0};**ap=1;}', True, None),
    ('pointer_to_array_param', 'void f(const int a[1][1]){**a=1;}', True, None),
    ('const_pointer_element_array', 'void f(void){int *const a[1]={0};*a=1;}', True, None),
    ('const_pointee_element_array', 'void f(void){const int *ap[1]={0};*ap=0;}', False, None),
    ('typedef_pointee_element_array', 'typedef const int CI;void f(void){CI * const a[1]={0};*a=0;}', True, None),
    # GCC's verbatim location spellings: parameter wording, byte-scaled
    # offsets, completed source fragments, cast type names.
    ('const_parameter_wording', 'void f(int *const p){p=0;}', True, None),
    ('const_subscript_offset', 'void f(const int p[2]){p[1]=1;}', True, None),
    ('local_array_subscript', 'void f(void){const int a[1]={0};a[0]=1;}', True, None),
    ('const_volatile_array_deref', 'void f(void){volatile const int a[1]={0};*a=1;}', True, None),
    ('const_member_pointee_field', 'struct S{const int *p;};void f(struct S*s){*s->p=1;}', True, None),
    ('const_cast_star_space', 'void f(int *p){*(const int*)p=1;}', True, None),
    ('const_cast_typedef', 'typedef const int CI;void f(int *p){*(CI*)p=1;}', True, None),
    ('const_cast_two_star', 'void f(int *p){*(int * const*)p=1;}', True, None),
    ('deref_offset_const', 'void f(const int *p){*(p+1)=1;}', True, None),
    ('reversed_subscript_wording', 'void f(const int *p){p[0]=1;}', True, None),
    ('mutable_written_p0', 'void f(int *p){p[0]=1;}', False, None),
]


def first_diag(err):
    """The first `error:`/`warning:` payload, or None when silent."""
    for line in err.splitlines():
        m = re.search(r'(?:error|warning): (.*)', line)
        if m:
            return m.group(1)
    return None


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
                if kind == 'pointer-to-int-cast':
                    # The measured matrix: -fno-permissive never promotes
                    # this warning; -Werror / -Werror= do; -Wno- silences.
                    options += [['-fno-permissive'], ['-Werror'],
                                ['-Wno-pointer-to-int-cast']]
                for flags in options:
                    enabled = bool(kind and '-Wno-' + kind not in flags)
                    permissive_flag = next((f for f in reversed(flags) if f in ('-fpermissive', '-fno-permissive')), '')
                    permissive = permissive_flag == '-fpermissive'
                    legacy = kind in ('implicit-int', 'return-mismatch') and std == 'gnu90' and (
                        '-Wall' in flags or '-Wreturn-type' in flags)
                    explicitly_enabled = kind and '-Werror=' + kind in flags
                    warn_live = enabled and (std != 'gnu90' or kind in ('int-conversion', 'pointer-to-int-cast')
                                             or name == 'void_return_value' or legacy or (explicitly_enabled and kind != 'return-mismatch'))
                    default_error = warn_live and kind != 'pointer-to-int-cast' and (
                        std != 'gnu90' or permissive_flag == '-fno-permissive') and not permissive and not any(
                        flag.startswith('-Wno-error=') for flag in flags)
                    promoted = warn_live and ('-Werror' in flags or explicitly_enabled) and '-w' not in flags
                    expected = int(hard or default_error or promoted)
                    result = run(cc, path, std, flags)
                    assert result.returncode == expected, (name, std, flags, expected, result.stderr)
                    if oracle:
                        ref = run(oracle, path, std, flags)
                        assert ref.returncode == expected, ('oracle drift', name, std, flags, ref.stderr)
                        # Verbatim first-diagnostic parity: message and
                        # location spelling must match GCC's own text.
                        assert first_diag(ref.stderr) == first_diag(result.stderr), (
                            'text drift', name, std, flags,
                            first_diag(ref.stderr), first_diag(result.stderr))
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
