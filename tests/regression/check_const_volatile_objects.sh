#!/usr/bin/env bash
# A `const volatile` object is a volatile object at every read: C11 6.7.3p7
# requires each access of a volatile lvalue to be evaluated strictly by the
# abstract machine, so `cvi + cvi` reads the object twice. The `const` qualifier
# only forbids writes; it must not make the object a compile-time constant.
#
# Sema used to record a folded value for every const object with a constant
# initializer, volatile ones included, and lowering cached `const` locals the
# same way. Both folded the reads away (`cvi + cvi` became 2, `(int)cvi + 1`
# became 1, and `const volatile int lv = 1; lv + lv` became 2). Probes cover
# globals, a cast, a typedef-qualified object, a `const volatile` member of a
# `const` struct object, and a local.
#
# Non-volatile const objects must still fold: their reads are not accesses, so
# the fold is correct and the control probes pin zero memory references.
#
# The check counts, per function at -O2, the memory-operand references (loads
# and stores). `lea` is NOT one: it forms an address in a register and never
# reads or writes memory, so `leal 1(%rdi), %eax` must not count (GCC's own
# bodies contain no lea-form memory line on these probes, so the pins are
# unaffected by the exclusion). lccc must match GCC 16.2 on every probe, and
# each probe's GCC count is pinned, so a fixture drift is reported as drift,
# not as a lccc bug.
#
# One documented exception: `f_field_twice` reads the `const volatile` member
# `f` of a `const struct` global. GCC 16.2 folds that read to 2 (no memory
# reference), which C11 6.7.3p7 does not allow. lccc keeps the read, as the
# standard requires, and the pinned lccc count is 1. A GCC/standard divergence
# is reported here, not silently matched.
#
# At -O0 every local lives in the stack frame, so whole-function counts are
# not meaningful. The -O0 check counts references to the global symbol only.
set -euo pipefail
. tests/regression/lib/gcc_oracle.sh
require_gcc_oracle check_const_volatile_objects || exit 1

CCC=${CCC:-target/fastbuild/lccc}
[[ -x $CCC ]] || { echo "check_const_volatile_objects: FAIL: compiler not found at $CCC" >&2; exit 1; }
tmp=$(mktemp -d "${TMPDIR:-/tmp}/lccc-cvo.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
gcc_oracle_positive_control "$CCC" "$tmp" || exit 1
gcc_oracle_positive_control "$GCC_ORACLE_BIN" "$tmp" || exit 1

src=$tmp/cvo.c
cat > "$src" <<'C'
const volatile int cvi = 1;
const volatile double cvd = 1.0;
typedef const volatile int CVI;
CVI tci = 1;
typedef volatile int VI;
const VI tvi = 1;
struct cs { const volatile int f; };
const struct cs csv = { 1 };
const double kd = 1.0;
const int ci = 2;
int f_cvi_twice(void) { return cvi + cvi; }
int f_cvd_cast(void) { return (int)cvd; }
int f_cvi_plus(void) { return (int)cvi + 1; }
int f_typedef_twice(void) { return tci + tci; }
int f_typedef_vol_const(void) { return tvi + tvi; }
int f_field_twice(void) { return csv.f + csv.f; }
int f_local(void) { const volatile int lv = 1; return lv + lv; }
int f_fold_double(void) { return (int)kd; }
int f_fold_int(void) { return ci + ci; }
int f_fold_ptr_ok(void) { const int *p = &ci; return *p; }
C

# Memory-operand references: every instruction that reads or writes memory.
# The mnemonic may contain digits (`cvttsd2si`), and directives are skipped.
count_mem_refs() {
    awk -v want="$2" '
        /^[A-Za-z_][A-Za-z0-9_]*:/ { fn = $0; sub(/:.*/, "", fn) }
        /^[ \t]+[a-z][a-z0-9]*[ \t]/ && !/^[ \t]+\./ && !/^[ \t]+lea[bwlq]?[ \t]/ && fn == want && /\(%/ { n++ }
        END { print n + 0 }' "$1"
}
# References to one global symbol inside one function (for -O0).
count_sym_refs() {
    awk -v want="$2" -v sym="$3" '
        /^[A-Za-z_][A-Za-z0-9_]*:/ { fn = $0; sub(/:.*/, "", fn) }
        fn == want && index($0, sym "(%rip)") > 0 { n++ }
        END { print n + 0 }' "$1"
}

fail=0
"$GCC_ORACLE_BIN" -O2 -S -o "$tmp/gcc.s" "$src"
"$CCC" -O2 -S "$src" -o "$tmp/lccc.s"

# function:pinned-GCC-count. Non-volatile controls pin 0 (folded).
for probe in f_cvi_twice:2 f_cvd_cast:1 f_cvi_plus:1 f_typedef_twice:2 \
    f_typedef_vol_const:2 f_local:3 f_fold_double:0 f_fold_int:0 f_fold_ptr_ok:0; do
    fn=${probe%%:*}
    want=${probe##*:}
    g=$(count_mem_refs "$tmp/gcc.s" "$fn")
    l=$(count_mem_refs "$tmp/lccc.s" "$fn")
    if [[ $g != "$want" ]]; then
        echo "FAIL $fn: GCC 16.2 memory-reference count is $g, the probe pins $want (fixture drift)" >&2
        fail=$((fail + 1))
    elif [[ $l != "$g" ]]; then
        echo "FAIL $fn: lccc memory-reference count is $l, GCC 16.2 keeps $g" >&2
        sed -n "/^$fn:/,/\\.size $fn,/p" "$tmp/lccc.s" >&2
        fail=$((fail + 1))
    fi
done

# Documented GCC/standard divergence: GCC folds the volatile member, lccc must
# keep both reads as C11 6.7.3p7 requires. The pinned GCC count shows the fold.
g=$(count_mem_refs "$tmp/gcc.s" f_field_twice)
l=$(count_mem_refs "$tmp/lccc.s" f_field_twice)
if [[ $g != 0 ]]; then
    echo "FAIL f_field_twice: GCC 16.2 count is $g; the documented divergence pins 0 (fixture or GCC drift)" >&2
    fail=$((fail + 1))
elif [[ $l != 1 ]]; then
    echo "FAIL f_field_twice: lccc memory-reference count is $l; the standard requires both volatile reads (pinned 1 load, the second read is reused from the first)" >&2
    sed -n "/^f_field_twice:/,/\\.size f_field_twice,/p" "$tmp/lccc.s" >&2
    fail=$((fail + 1))
fi

# -O0: symbol-specific reference counts. Each volatile read is one reference.
"$GCC_ORACLE_BIN" -O0 -S -o "$tmp/gcc0.s" "$src"
"$CCC" -O0 -S "$src" -o "$tmp/lccc0.s"
for probe in f_cvi_twice:cvi:2 f_cvi_plus:cvi:1 f_cvd_cast:cvd:1 f_typedef_twice:tci:2; do
    fn=$(echo "$probe" | cut -d: -f1)
    sym=$(echo "$probe" | cut -d: -f2)
    want=$(echo "$probe" | cut -d: -f3)
    g=$(count_sym_refs "$tmp/gcc0.s" "$fn" "$sym")
    l=$(count_sym_refs "$tmp/lccc0.s" "$fn" "$sym")
    if [[ $g != "$want" ]]; then
        echo "FAIL $fn at -O0: GCC 16.2 references $sym $g times, the probe pins $want (fixture drift)" >&2
        fail=$((fail + 1))
    elif [[ $l != "$g" ]]; then
        echo "FAIL $fn at -O0: lccc references $sym $l times, GCC 16.2 references it $g times" >&2
        fail=$((fail + 1))
    fi
done

if (( fail > 0 )); then
    echo "check_const_volatile_objects: FAILED ($fail)" >&2
    exit 1
fi
echo "check_const_volatile_objects: PASSED (10 functions at -O2 and 4 symbol checks at -O0; volatile reads match GCC 16.2, non-volatile const controls still fold)"
