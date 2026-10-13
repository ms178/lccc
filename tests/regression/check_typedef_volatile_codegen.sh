#!/usr/bin/env bash
# A typedef name is a synonym for its declarator text (C11 6.7.8), so an object
# declared through `typedef volatile int VI;` is volatile: each store to it must
# survive optimization. lccc used to drop them (the typedef's qualifier was
# lost), so `VI z; z = 1; z = 2;` compiled to no store at all.
#
# The check counts the stores that survive at -O2 in each probe function and
# requires lccc to match the oracle (the oracle is checked for the same counts).
# It covers the volatile object forms: a typedef'd scalar, a typedef'd pointer
# with a volatile object qualifier (`VIP`), a volatile pointee through a typedef
# (`VPI`), a volatile array typedef (`VAV`), a volatile pointer object
# (`int *volatile p`), a volatile pointee stored through a local pointer, and an
# array of typedef'd volatile pointers (`VIP a[2]`), locally and as a struct member.
# Each form was a miscompile: its stores were dropped at -O2.
set -euo pipefail
. tests/regression/lib/gcc_oracle.sh
require_gcc_oracle check_typedef_volatile_codegen || exit 1

CCC=${CCC:-target/fastbuild/lccc}
[[ -x $CCC ]] || { echo "check_typedef_volatile_codegen: FAIL: compiler not found at $CCC" >&2; exit 1; }
tmp=$(mktemp -d "${TMPDIR:-/tmp}/lccc-tv.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
gcc_oracle_positive_control "$CCC" "$tmp" || exit 1
gcc_oracle_positive_control "$GCC_ORACLE_BIN" "$tmp" || exit 1

src=$tmp/tv.c
cat > "$src" <<'C'
typedef volatile int VI;
VI gv;
void f_local(void) { VI z; z = 1; z = 2; }
void f_global(void) { gv = 1; gv = 2; }
void f_written_local(void) { volatile int z; z = 1; z = 2; }
typedef int *volatile VIP;
typedef volatile int *VPI;
typedef volatile int VAV[2];
void f_ptr_typedef(void) { VIP p; p = 0; p = 0; }
void f_pointee(void) { int a = 0; VPI p = &a; *p = 1; *p = 2; }
void f_array(void) { VAV a; a[0] = 1; a[0] = 2; }
void f_written_ptr(void) { int *volatile p; p = 0; p = 0; }
void f_written_pointee(void) { int a = 0; volatile int *p = &a; *p = 1; *p = 2; }
void f_param_leak(VI p, int y) { y = 1; y = 2; (void)p; }
void f_member_leak(void) { struct S { VIP m; int n; } s; s.n = 1; s.n = 2; (void)s; }
void f_decl_leak(void) { VIP p; int z; z = 1; z = 2; (void)p; }
void f_arr_ptr_typedef(void) { VIP a[2]; a[0] = 0; a[0] = 0; (void)a; }
struct SA { VIP m[2]; };
void f_struct_arr_ptr_typedef(void) { struct SA s; s.m[0] = 0; s.m[0] = 0; (void)s; }
C

# Store count per function in an assembly file: a store is a mov with an
# immediate source to a memory operand, inside the named function.
count_stores() {
    awk -v want="$2" '
        /^[A-Za-z_][A-Za-z0-9_]*:/ { fn = $0; sub(/:.*/, "", fn) }
        /^[ \t]+mov[lq]?[ \t]+\$/ && fn == want && /\(/ { n++ }
        END { print n + 0 }' "$1"
}

fail=0
"$GCC_ORACLE_BIN" -O2 -S -o "$tmp/gcc.s" "$src"
"$CCC" -O2 -S "$src" -o "$tmp/lccc.s"
# function:expected-count. The leak cases keep no stores: a parameter's or a
# member's qualifiers must not leak into a later declaration, so the oracle drops them.
for probe in f_local:2 f_global:2 f_written_local:2 f_ptr_typedef:2 f_pointee:2 f_array:2 \
    f_written_ptr:2 f_written_pointee:2 f_param_leak:0 f_member_leak:0 f_decl_leak:0 \
    f_arr_ptr_typedef:2 f_struct_arr_ptr_typedef:2; do
    fn=${probe%%:*}
    want=${probe##*:}
    g=$(count_stores "$tmp/gcc.s" "$fn")
    l=$(count_stores "$tmp/lccc.s" "$fn")
    if [[ $g != "$want" ]]; then
        echo "FAIL $fn: GCC $GCC_ORACLE_MAJOR keeps $g stores, the probe expects $want (fixture drift)" >&2
        fail=$((fail + 1))
    elif [[ $l != "$g" ]]; then
        echo "FAIL $fn: lccc keeps $l stores, GCC $GCC_ORACLE_MAJOR keeps $g" >&2
        sed -n "/^$fn:/,/ret/p" "$tmp/lccc.s" >&2
        fail=$((fail + 1))
    fi
done
if (( fail > 0 )); then
    echo "check_typedef_volatile_codegen: FAILED ($fail)" >&2
    exit 1
fi
echo "check_typedef_volatile_codegen: PASSED (13 functions, store counts match GCC $GCC_ORACLE_MAJOR)"
