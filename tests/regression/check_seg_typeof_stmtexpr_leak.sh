#!/usr/bin/env bash
# typeof() whose expression contains a nested declaration with a named
# address space must not leak that space onto the declared object
# (kernel arch/x86/events/amd/core.c boot blocker, 6.18.55: the per-CPU
# read macro `typeof(({ __typeof_unqual__(v) p; <__verify_pcpu_ptr do-block
# with `const void __percpu *`; ...>; p; })) __dummy2` used to inherit
# __seg_gs from the nested declaration, tripping "'__seg_gs' specified
# for auto variable" on code GCC 16.2 accepts).
#
# The parser keeps a base address-space qualifier in `parsing_address_space`
# across `a, b` declarators; a declaration nested inside the outer
# declaration's own type specifier left it set, and the outer declarator
# snapshot then inherited the nested declaration's space. The contract:
#
#   * every fixture here compiles clean under GCC 16.2 -> lccc must accept;
#   * a block-scope object with an EXPLICIT space qualifier must still be
#     rejected (the auto-variable diagnostic is not weakened);
#   * the space of a nested declaration must not reach the outer object's
#     storage space either (an explicit `__seg_gs int x;` at block scope
#     stays the only way to declare a segment object).
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
. tests/regression/lib/gcc_oracle.sh
require_gcc_oracle check_seg_typeof_stmtexpr_leak || exit 1

tmp=$(mktemp -d "${TMPDIR:-/tmp}/lccc-seg-typeof-leak.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
gcc_oracle_positive_control "$CCC" "$tmp" || exit 1
gcc_oracle_positive_control "$GCC_ORACLE_BIN" "$tmp" || exit 1

fail=0

accept() { # accept <id> <body...>
    local id=$1 body=$2
    local f="$tmp/$id.c"
    printf 'void f(void) {\n%s\n}\n' "$body" > "$f"
    accept_check "$id" "$f"
}
accept_tu() { # accept_tu <id> <whole translation unit>
    local id=$1 body=$2
    local f="$tmp/$id.c"
    printf '%s\n' "$body" > "$f"
    accept_check "$id" "$f"
}
accept_check() { # accept_check <id> <file>
    local id=$1 f=$2
    set +e
    "$CCC" -c -o "$tmp/l.o" "$f" 2>"$tmp/l.err"
    lrc=$?
    "$GCC_ORACLE_BIN" -w -std=gnu11 -c -o "$tmp/g.o" "$f" >/dev/null 2>&1
    grc=$?
    set -e
    if [ "$grc" -ne 0 ]; then
        echo "FAIL: $id: oracle rejects the fixture; fix the fixture" >&2
        fail=1
        return
    fi
    if [ "$lrc" -ne 0 ]; then
        echo "FAIL: $id: lccc rejects, GCC 16 accepts" >&2
        head -2 "$tmp/l.err" >&2
        fail=1
        return
    fi
}

reject() { # reject <id> <body...>
    local id=$1 body=$2
    local f="$tmp/$id.c"
    printf 'void f(void) {\n%s\n}\n' "$body" > "$f"
    if "$CCC" -c -o "$tmp/l.o" "$f" >/dev/null 2>&1; then
        echo "FAIL: $id: lccc accepts a block-scope segment object" >&2
        fail=1
        return
    fi
}

# The kernel shape: nested __seg_gs pointer declaration inside the typeof'd
# statement expression (the __verify_pcpu_ptr do-block).
accept nested_decl \
  'typeof(({ const void __seg_gs *q = 0; (void)q; 0; })) x = 0; (void)x;'
accept nested_decl_cast \
  'typeof(({ const void __seg_gs *q = 0; (void)q; 0; })) x = 0; (void)x; (void)0;'
# The full percpu-read macro: __typeof_unqual__ result var + verify block +
# a dereference assignment, all inside the typeof'd statement expression.
# percpu_read_shape's segment object must live at file scope (GCC rejects
# block-scope segment objects), so this one is written as a whole TU.
accept_tu percpu_read_shape \
  'unsigned long __seg_gs v;
void f(void) {
   unsigned long __dummy = 0;
   typeof(({ __typeof_unqual__(v) p;
             do { const void __seg_gs *__vpp = (const void __seg_gs *)((void *)0); (void)__vpp; } while (0);
             p = *(volatile typeof(v) *)(&v); p; })) __dummy2;
   (void)(&__dummy == (unsigned long *)&__dummy2);
}'
# Several nested declarations; only the last is AS-qualified.
accept nested_last \
  'typeof(({ int a = 0; const void __seg_fs *q = 0; (void)a; (void)q; 0; })) x = 0; (void)x;'
# The leak must not flip the other direction either: a clean typeof is clean.
accept nested_clean \
  'typeof(({ const void *q = 0; (void)q; 0; })) x = 0; (void)x;'
# Same-translation-unit pollution: a qualified pointer declaration followed
# by an unrelated clean declaration in the same block.
accept sequence_clean \
  'const unsigned long __seg_gs *p = 0; int z = 0; (void)p; (void)z;'

# Negative controls: explicit block-scope segment objects stay rejected.
reject explicit_object 'int __seg_gs obj = 0;'
reject explicit_object_fs 'int __seg_fs obj2 = 0;'

if [ "$fail" -ne 0 ]; then
    echo "check_seg_typeof_stmtexpr_leak: FAILED"
    exit 1
fi
echo "check_seg_typeof_stmtexpr_leak: PASSED (6 accepted shapes, 2 negative controls, oracle GCC $GCC_ORACLE_MAJOR)"
