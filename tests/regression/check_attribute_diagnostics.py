#!/usr/bin/env python3
"""GP-01: attribute diagnostics against a live gcc-16.2.

Three classes (measured on gcc-16.2.0, 2026-10-28):

* unknown name    -> `'name' attribute directive ignored [-Wattributes]`
                     (default-on, caret at the name token),
* misapplied name -> `'name' attribute ignored [-Wattributes]`, plus gcc's
                     two dedicated wordings (`naked`, `const`/`pure` on a
                     void-returning function),
* known-but-unimplemented names -> `'name' attribute ignored` under an
                     EXPLICIT `-Wattributes` request only; the pinned list
                     below is generated from the registry (gcc-16.2 minus
                     the dispatcher) and may only shrink.

Every probe runs both compilers; expectations are gcc's own first
diagnostic, so the gate fails on any wording drift.
"""
from __future__ import annotations
import os, re, shutil, sys, tempfile
from pathlib import Path

# Probes compare GCC's English text byte for byte; under a UTF-8 locale GCC
# quotes with U+2018/U+2019, so every compiler runs with LC_ALL=C.
ENV = {**os.environ, 'LC_ALL': 'C'}

ROOT = Path(__file__).resolve().parents[2]
LCCC = ROOT / 'target/fastbuild/lccc'

sys.path.insert(0, str(Path(__file__).resolve().parent))
import abi_attribute_matrix as abi_matrix  # noqa: E402

def find_gcc():
    cands = [os.environ.get('GCC', ''), 'gcc-16', 'gcc']
    for cand in cands:
        if not cand:
            continue
        p = shutil.which(cand)
        # `run_bounded` always runs in text mode, so this compares str to str.
        if p and '16.2' in abi_matrix.run_bounded(
                [p, '--version'], timeout=abi_matrix.COMPILE_TIMEOUT).stdout:
            return p
    return None

# name -> (source, first-diagnostic-substring, requires_explicit_Wattributes)
PROBES = [
    ('frobnicate', 'void __attribute__((frobnicate)) int y;'.replace('void ', ''),
     "'frobnicate' attribute directive ignored", False),
    ('unknown-fn', 'void __attribute__((totally_bogus_attr)) f(void){}',
     "'totally_bogus_attr' attribute directive ignored", False),
    ('packed-int', 'int __attribute__((packed)) x;',
     "'packed' attribute ignored", False),
    ('noreturn-int', 'int __attribute__((noreturn)) x;',
     "'noreturn' attribute ignored", False),
    ('constructor-int', 'int __attribute__((constructor)) x;',
     "'constructor' attribute ignored", False),
    ('destructor-int', 'int __attribute__((destructor)) x;',
     "'destructor' attribute ignored", False),
    ('noinline-int', 'int __attribute__((noinline)) x;',
     "'noinline' attribute ignored", False),
    ('cold-int', 'int __attribute__((cold)) x;',
     "'cold' attribute ignored", False),
    ('target-int', 'int __attribute__((target("sse2"))) x;',
     "'target' attribute ignored", False),
    ('naked-int', 'int __attribute__((naked)) x;',
     "'naked' attribute only applies to functions", False),
    ('noplt-int', 'int __attribute__((noplt)) x;',
     "'noplt' attribute is only applicable on functions", False),
    ('noplt-int-post', 'int x __attribute__((noplt));',
     "'noplt' attribute is only applicable on functions", False),
    ('const-int', 'int __attribute__((const)) x;',
     "'const' attribute ignored", False),
    ('pure-int', 'int __attribute__((pure)) x;',
     "'pure' attribute ignored", False),
    ('const-void', 'void __attribute__((const)) v(void){}',
     "'const' attribute on function returning 'void'", False),
    ('pure-void', 'void __attribute__((pure)) v(void){}',
     "'pure' attribute on function returning 'void'", False),
    # accepted forms: gcc silent, lccc silent (inventory stays behind -W)
    ('ok-struct-packed', 'struct __attribute__((packed)) S{int a;};', None, False),
    ('ok-fn-noreturn', 'void __attribute__((noreturn)) h(void){}', None, False),
    ('ok-fn-noplt', 'void __attribute__((noplt)) n(void){}', None, False),
    ('ok-int-aligned', 'int __attribute__((aligned(8))) x;', None, False),
    ('ok-fn-cold', 'void __attribute__((cold)) c(void){}', None, False),
    ('ok-format-default', '__attribute__((format(printf,1,2))) void g(const char*,...);', None, False),
    # Preserve upstream's GCC 16.2 canonical-name and glibc pointer-return
    # controls while extending the inventory and optimization-level survey.
    ('ok-gnu-inline-ptr-ret',
     'extern __inline __attribute__ ((__gnu_inline__)) void *fa(int);', None, False),
    ('ok-gnu-inline-nth',
     'extern __inline __attribute__ ((__gnu_inline__)) void *\n'
     '__attribute__ ((__nothrow__ , __leaf__)) fc(int);', None, False),
    ('naked-dunder-object',
     'extern __attribute__ ((__naked__)) int (*f1)(void);',
     "'naked' attribute only applies to functions", False),
    ('unknown-dunder',
     'extern __attribute__ ((__made_up__)) int (*f2)(void);',
     "'made_up' attribute directive ignored", False),
    ('gnu-inline-object',
     'extern __attribute__ ((__gnu_inline__)) int (*fpo)(void);',
     "'gnu_inline' attribute ignored", False),
    ('const-void-dunder', 'void __attribute__((__const__)) v(void){}',
     "'const' attribute on function returning 'void'", False),
    ('pure-void-dunder', 'void __attribute__((__pure__)) v(void){}',
     "'pure' attribute on function returning 'void'", False),
    ('ok-underscored-nothrow-leaf', 'void __attribute__((__nothrow__, __leaf__)) f(void){}', None, False),
    ('ok-nothrow', 'void __attribute__((nothrow)) f(void){}', None, False),
    ('ok-leaf', 'void __attribute__((leaf)) f(void){}', None, False),
    # Function identity is determined by the final derived declarator, not by
    # requiring a singleton Function list. Return pointers (including function
    # pointers) must not turn valid attributes into default-on warnings.
    ('ok-gnu-inline-pointer-return',
     'extern inline __attribute__((gnu_inline)) void *f(void){return 0;}', None, False),
    ('ok-noinline-pointer-return', 'int __attribute__((noinline)) *f(void);', None, False),
    ('ok-const-void-pointer-return', 'void __attribute__((const)) *f(void);', None, False),
    ('ok-pure-deep-pointer-return', 'void __attribute__((pure)) **f(void);', None, False),
    ('ok-pure-function-pointer-return',
     'void __attribute__((pure)) (*f(void))(int);', None, False),
    ('ok-const-function-pointer-return',
     'void __attribute__((const)) (*f(void))(int);', None, False),
    ('ok-pure-typedef-pointer-return',
     'typedef void *P; P __attribute__((pure)) f(void);', None, False),
    ('ok-local-pointer-return',
     'void g(void) { void __attribute__((pure)) *f(void); (void)f; }', None, False),
    ('ignored-noreturn-pointer-object', 'int __attribute__((noreturn)) *p;',
     "'noreturn' attribute ignored", False),
    ('ignored-noinline-function-pointer-object', 'int __attribute__((noinline)) (*p)(void);',
     "'noinline' attribute ignored", False),
    ('ignored-cleanup-on-pointer-return-function', 'int __attribute__((cleanup(c))) *f(void);',
     "'cleanup' attribute ignored", False),
    # `noreturn` on a declarator that names a function TYPE belongs to that
    # type; gcc-16.2 is silent for a function-pointer object and a typedef
    # (measured), so the object-target verdict must not fire.
    ('ok-noreturn-function-pointer-object',
     'extern int (*fp)(void) __attribute__((noreturn));', None, False),
    ('ok-noreturn-typedef-function-pointer',
     'typedef void (*handler)(int) __attribute__((noreturn));', None, False),
    # `cleanup` argument checking: the count is an error with an `expected 1,
    # found N` note, a non-identifier argument is an error, and a well-formed
    # attribute on an automatic object is silent.
    ('cleanup-arity-zero', 'void __attribute__((cleanup)) f(void) {}',
     "wrong number of arguments specified for 'cleanup' attribute", False),
    ('cleanup-arity-two',
     'void h(void *p) { (void)p; } void __attribute__((cleanup(h, h))) f(void) {}',
     "wrong number of arguments specified for 'cleanup' attribute", False),
    ('cleanup-argument-shape',
     'void f(void) { __attribute__((cleanup(1))) int x = 0; (void)x; }',
     "cleanup argument not an identifier", False),
    ('ok-cleanup-well-formed',
     'void h(void *p) { (void)p; } void f(void) { '
     '__attribute__((cleanup(h))) int x = 0; (void)x; }', None, False),
]

# Generated from the registry: GCC 16.2 knows these, lccc's dispatcher has
# no handler for them. Shrink-only: removing a name must be paired with the
# dispatcher gaining a handler (the gate re-probes both directions).
UNIMPLEMENTED = sorted(set("""
access alloc_align alloc_size artificial assume assume_aligned btf_decl_tag btf_type_tag
cf_check copy deprecated designated_init expected_throw
fallthrough fd_arg fd_arg_read fd_arg_write flag_enum flatten format
format_arg hardbool hot indirect_branch indirect_return interrupt leaf malloc
may_alias ms_hook_prologue musttail no_address_safety_analysis
no_icf no_profile_instrument_function
no_reorder no_sanitize_address no_sanitize_coverage no_sanitize_thread no_split_stack
no_stack_limit nocf_check noclone nodirect_extern_access noipa nonnull nonnull_if_nonzero
nonstring nothrow null_terminated_string_arg objc_nullability objc_root_class
patchable_function_entry reproducible returns_nonnull returns_twice sentinel
signed_bool_precision simd strict_flex_array strub tainted_args
target_version transaction_callable transaction_may_cancel_outer transaction_pure
transaction_safe transaction_safe_dynamic transaction_unsafe unavailable unsequenced
vector_mask warn_if_not_aligned warn_unused warn_unused_result
""".split()))

# These system headers use GCC's underscored spelling of attributes (notably
# __nothrow__ and __leaf__). Any diagnostic, even a non-attribute one, breaks
# real projects that build their translation units with warnings as errors.
HEADER_PROBES = [
    ('stdio', '#include <stdio.h>\nint f(void) { return ferror(stdin); }'),
    ('stdlib-string', '#include <stdlib.h>\n#include <string.h>\n'
     'int f(void) { return atoi("0") + (int)strlen("x"); }'),
]


def registry_unimplemented() -> list[str]:
    """Check the pin against the actual Rust table, not just its diagnostics."""
    source = (ROOT / 'src/frontend/parser/attribute_registry.rs').read_text()
    def names(table):
        match = re.search(rf'const {table}: &\[&str\] = &\[(.*?)\n\];', source, re.S)
        if not match:
            raise ValueError(f'missing attribute registry table {table}')
        entries = re.findall(r'"([^"\n]+)"', match.group(1))
        if len(entries) != len(set(entries)):
            raise ValueError(f'duplicate names in {table}')
        return entries
    known = names('GCC_KNOWN')
    if known != sorted(known):
        raise ValueError('GCC_KNOWN must be sorted')
    # Convention attributes never appear in this inventory: their verdict is
    # decided by `abi_effect` before the classification funnel, and the ABI
    # half of this gate (abi_attribute_matrix) owns them.
    return sorted(set(known) - set(names('IMPLEMENTED')) - set(registry_abi_names()))


def registry_abi_names() -> list[str]:
    """Names carrying a calling-convention verdict (parsed from ABI_VERDICTS)."""
    return sorted({name for (name, _target) in abi_matrix.registry_verdicts()})


def has_multilib(gcc: str) -> bool:
    """Whether this gcc can target i686 at all (the -m32 half of the table).

    Bounded like every other probe: gcc hanging on a broken multilib install
    must fail the gate, not stall the job until its six-hour limit.
    """
    return abi_matrix.run_bounded(
        [gcc, '-m32', '-E', '-x', 'c', '-', '-o', os.devnull],
        timeout=abi_matrix.COMPILE_TIMEOUT, env=ENV,
        input='int x;\n').returncode == 0


def first_diag(stderr: str) -> str:
    for line in stderr.splitlines():
        if ': error: ' in line or ': warning: ' in line:
            return line.split(': ', 2)[-1] if line.count(': ') >= 2 else line
    return ''

def run(cc, src, flags):
    with tempfile.NamedTemporaryFile('w', suffix='.c', delete=False) as f:
        f.write(src + '\n')
        path = f.name
    try:
        r = abi_matrix.run_bounded([cc, '-fsyntax-only', '-std=gnu23', *flags, path],
                                   timeout=abi_matrix.COMPILE_TIMEOUT, env=ENV)
        return r.returncode, r.stderr
    finally:
        os.unlink(path)

def main() -> int:
    if not LCCC.exists():
        print(f'SKIP: {LCCC} not built')
        return 1
    gcc = find_gcc()
    if not gcc:
        print('SKIP: gcc-16 not found')
        return 1
    failures = []
    actual_inventory = registry_unimplemented()
    if actual_inventory != UNIMPLEMENTED:
        failures.append(('inventory', 'registry/pin mismatch', UNIMPLEMENTED, actual_inventory))
    for name, src, expected, explicit in PROBES:
        flags = ['-Wattributes'] if explicit else []
        grext, gerr = run(gcc, src, flags)
        lex, lerr = run(str(LCCC), src, flags)
        gdiag, ldiag = first_diag(gerr), first_diag(lerr)
        if expected is None:
            if grext or lex or gerr or lerr:
                failures.append((name, 'expected silence and success', (grext, gerr), (lex, lerr)))
        else:
            if expected not in gdiag:
                failures.append((name, 'gcc wording drift', expected, gdiag))
            elif gdiag != ldiag:
                failures.append((name, 'text drift', gdiag, ldiag))
        if lex != grext and expected is not None:
            failures.append((name, 'exit drift', grext, lex))
    for name, src in HEADER_PROBES:
        for label, cc in (('gcc', gcc), ('lccc', str(LCCC))):
            # glibc selects additional extern-inline definitions when
            # __OPTIMIZE__ is defined, notably bsearch returning void *.
            for opt in ('-O0', '-O2', '-O3', '-Os'):
                rc, err = run(cc, src, [opt, '-Wall', '-Wextra', '-Werror'])
                if rc or err:
                    failures.append((name, f'{label} {opt} header probe must be silent', 0, (rc, err)))
    # Convention attributes: the table in the registry must agree with what
    # this gcc actually does (re-measured, not trusted), and lccc must honour
    # every cell — silence for `Default`, gcc's own sentence for the ignored
    # classes, a refusal for the conventions it cannot lower. The two it does
    # lower are proved end-to-end by a cross-compiler runtime sentinel.
    abi_failures, abi_notes = abi_matrix.check_table_against_gcc(
        gcc, abi_matrix.registry_verdicts(), has_multilib(gcc))
    # One resolved driver for both directions: the caller already knows the
    # artifact under test, and a second guess inside the matrix could name a
    # stale binary (`LCCC` unset) while this gate measured the right one.
    abi_lccc_failures, abi_lccc_notes = abi_matrix.check_lccc(
        abi_matrix.registry_verdicts(), LCCC)
    for note in abi_notes + abi_lccc_notes:
        print(f'  note: {note}')
    failures.extend(abi_failures)
    failures.extend(abi_lccc_failures)
    # Inventory: explicit -Wattributes reports each pinned name; gcc stays
    # silent on the valid-target probe (the delta is exactly the pin).
    for n in UNIMPLEMENTED:
        src = f'void __attribute__(({n})) f(void){{}}'
        grext, gerr = run(gcc, src, ['-Wattributes'])
        lex, lerr = run(str(LCCC), src, ['-Wattributes'])
        gdiag, ldiag = first_diag(gerr), first_diag(lerr)
        want = f"'{n}' attribute ignored"
        # The pin is only valid while gcc recognises the name: an unknown-name
        # classification on gcc's side means the registry lost the entry.
        if 'directive ignored' in gdiag:
            failures.append((n, 'gcc no longer knows the name', '', gdiag))
        if want not in ldiag:
            failures.append((n, 'inventory silent', want, ldiag))
    # -Wno-attributes suppresses everything.
    _, lerr = run(str(LCCC), '__attribute__((frobnicate)) int y;', ['-Wno-attributes'])
    if first_diag(lerr):
        failures.append(('no-flag', 'not suppressed', '', first_diag(lerr)))
    if failures:
        print(f'FAIL: {len(failures)} attribute diagnostic mismatches')
        for f in failures[:20]:
            print('  ', f)
        return 1
    print(f'PASS: attribute diagnostics match gcc-16.2 '
          f'({len(PROBES)} probes + {len(HEADER_PROBES)} headers x 4 optimization levels + '
          f'{len(UNIMPLEMENTED)} pinned inventory names + '
          f'{len(abi_matrix.registry_verdicts())} ABI convention cells)')
    return 0

if __name__ == '__main__':
    sys.exit(main())
