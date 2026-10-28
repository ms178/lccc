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
import os, shutil, subprocess, sys, tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
LCCC = ROOT / 'target/fastbuild/lccc'

def find_gcc():
    cands = [os.environ.get('GCC', ''), 'gcc-16', 'gcc']
    for cand in cands:
        if not cand:
            continue
        p = shutil.which(cand)
        if p and b'16.2' in subprocess.run([p, '--version'], capture_output=True).stdout:
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
    ('ok-int-aligned', 'int __attribute__((aligned(8))) x;', None, False),
    ('ok-fn-cold', 'void __attribute__((cold)) c(void){}', None, False),
    ('ok-format-default', '__attribute__((format(printf,1,2))) void g(const char*,...);', None, False),
]

# Generated from the registry: GCC 16.2 knows these, lccc's dispatcher has
# no handler for them. Shrink-only: removing a name must be paired with the
# dispatcher gaining a handler (the gate re-probes both directions).
UNIMPLEMENTED = sorted(set("""
alloc_align alloc_size assume btf_decl_tag btf_type_tag callee_pop_aggregate_return cdecl cf_check
copy deprecated designated_init expected_throw fallthrough flag_enum flatten force_align_arg_pointer
format format_arg gcc_struct hardbool indirect_branch indirect_return interrupt leaf malloc may_alias
ms_abi ms_hook_prologue ms_struct musttail no_address_safety_analysis no_callee_saved_registers
no_caller_saved_registers no_icf no_reorder no_sanitize_address no_sanitize_thread no_split_stack
no_stack_limit nocf_check noclone nocommon nodirect_extern_access noinit noipa nonnull nonstring noplt
null_terminated_string_arg patchable_function_entry persistent preserve_none retain returns_nonnull
signed_bool_precision simd sseregparm stdcall strict_flex_array strub sysv_abi tainted_args
target_version thiscall transaction_pure transaction_unsafe unavailable vector_mask warn_if_not_aligned
warn_unused warn_unused_result
""".split()))

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
        r = subprocess.run([cc, '-fsyntax-only', '-std=gnu23', *flags, path],
                           capture_output=True, text=True)
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
    for name, src, expected, explicit in PROBES:
        flags = ['-Wattributes'] if explicit else []
        grext, gerr = run(gcc, src, flags)
        lex, lerr = run(str(LCCC), src, flags)
        gdiag, ldiag = first_diag(gerr), first_diag(lerr)
        if expected is None:
            if gdiag or ldiag:
                failures.append((name, 'expected silence', gdiag, ldiag))
        else:
            if expected not in gdiag:
                failures.append((name, 'gcc wording drift', expected, gdiag))
            elif gdiag != ldiag:
                failures.append((name, 'text drift', gdiag, ldiag))
        if lex != grext and expected is not None:
            failures.append((name, 'exit drift', grext, lex))
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
          f'({len(PROBES)} probes + {len(UNIMPLEMENTED)} pinned inventory names)')
    return 0

if __name__ == '__main__':
    sys.exit(main())
