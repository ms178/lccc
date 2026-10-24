#!/usr/bin/env bash
# ============================================================================
# check_fp_select_minmax.sh — exact FP-select min/max semantics and shape.
#
# A Select over a strict relational FP compare, whose arms are the compare's
# operands verbatim, is one MIN/MAX instruction. The true arm is the first
# source and the false arm is the second: SECOND OPERAND WINS on NaN and ties
# is exactly the ordered C select. The instruction follows arm orientation:
#
#   a > b ? a : b -> max(dst=a, src=b)    a > b ? b : a -> min(dst=b, src=a)
#   a < b ? a : b -> min(dst=a, src=b)    a < b ? b : a -> max(dst=b, src=a)
#
# Non-strict ge/le forms are not folded: their tie arm is opposite MIN/MAX's
# second-source rule. This gate covers the COMPLETE 16-case matrix:
#   {gt,lt,ge,le} x {natural,swapped arms} x {f32,f64}.
#
# Validation has two independent halves:
#   1. bit-exact runtime differential against GCC 16.2 and Clang 23.1.3 at
#      -O0/-O1/-O2/-O3/-Os, in SSE and AVX modes, over infinities, ordinary
#      values, +/-0, QNaN and SNaN in both operand positions. Constant selects
#      and register-pressure/alias shapes are included alongside the matrix.
#      ICX 2026.2 is added as a third oracle when provisioned.
#   2. assembly shape for all 16 forms in both ISA modes. The eight strict
#      forms have exactly one MIN/MAX; the eight non-strict forms have none.
#      AVX must use VEX mnemonics directly, avoiding legacy-SSE transitions.
# ============================================================================
set -euo pipefail

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
ccc=${CCC:-$repo/target/fastbuild/lccc}
gcc=${GCC:-gcc-16}
clang=${CLANG:-clang-23}
icx=${ICX:-icx}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

fail() {
    echo "check_fp_select_minmax: FAIL: $*" >&2
    echo "::error title=FP select min/max::$*" >&2
    exit 1
}
[ -x "$ccc" ] || { echo "SKIP: $ccc not built" >&2; exit 0; }
icx_only=${FP_MINMAX_ICX_ONLY:-0}
if [ "$icx_only" = 1 ]; then
    "$icx" --version 2>/dev/null | grep -c '2026\.2' >/dev/null \
        || fail "FP_MINMAX_ICX_ONLY requires an ICX 2026.2 oracle"
else
    "$gcc" --version 2>/dev/null | grep -c '16\.2' >/dev/null \
        || { echo "SKIP: gcc-16.2 oracle not on PATH" >&2; exit 0; }
    "$clang" --version 2>/dev/null | grep -c '23\.1\.3' >/dev/null \
        || { echo "SKIP: clang-23.1.3 oracle not on PATH" >&2; exit 0; }
fi

cat >"$work/matrix.c" <<'EOF'
#include <stdio.h>
#include <string.h>
typedef unsigned long long u64;
#define DEF(name, type, op, t, f) \
    __attribute__((noinline)) type name(type a, type b) { return a op b ? t : f; }
DEF(d_gt,    double, >, a, b)
DEF(d_gt_sw, double, >, b, a)
DEF(d_lt,    double, <, a, b)
DEF(d_lt_sw, double, <, b, a)
DEF(d_ge,    double, >=, a, b)
DEF(d_ge_sw, double, >=, b, a)
DEF(d_le,    double, <=, a, b)
DEF(d_le_sw, double, <=, b, a)
DEF(f_gt,    float, >, a, b)
DEF(f_gt_sw, float, >, b, a)
DEF(f_lt,    float, <, a, b)
DEF(f_lt_sw, float, <, b, a)
DEF(f_ge,    float, >=, a, b)
DEF(f_ge_sw, float, >=, b, a)
DEF(f_le,    float, <=, a, b)
DEF(f_le_sw, float, <=, b, a)

/* Extra register-home/alias pressure: the select result must not overwrite
   the false arm before that arm has been secured. */
__attribute__((noinline)) double d_alias(double a, double b, double c, double d) {
    double r = a > b ? a : b;
    return c == d ? c : r;
}
__attribute__((noinline)) float f_alias(float a, float b, float c, float d) {
    float r = a < b ? b : a;
    return c == d ? c : r;
}

/* Compile-time constants, including both signed-zero arm orientations. */
__attribute__((noinline)) double cd_zero(void) { return 0.0 > -0.0 ? 0.0 : -0.0; }
__attribute__((noinline)) double cd_zero_sw(void) { return 0.0 > -0.0 ? -0.0 : 0.0; }
__attribute__((noinline)) float cf_zero(void) { return 0.0f > -0.0f ? 0.0f : -0.0f; }
__attribute__((noinline)) float cf_zero_sw(void) { return 0.0f > -0.0f ? -0.0f : 0.0f; }

static u64 H;
static void mix(u64 x) { H ^= x; H *= 1099511628211ULL; }
static void mixd(double x) { u64 u; memcpy(&u, &x, 8); mix(u); }
static void mixf(float x) { unsigned u; memcpy(&u, &x, 4); mix((u64)u); }
#define DALL(a,b) do { \
    mixd(d_gt(a,b)); mixd(d_gt_sw(a,b)); mixd(d_lt(a,b)); mixd(d_lt_sw(a,b)); \
    mixd(d_ge(a,b)); mixd(d_ge_sw(a,b)); mixd(d_le(a,b)); mixd(d_le_sw(a,b)); \
    mixd(d_alias(a,b,0.75,-1.25)); \
} while (0)
#define FALL(a,b) do { \
    mixf(f_gt(a,b)); mixf(f_gt_sw(a,b)); mixf(f_lt(a,b)); mixf(f_lt_sw(a,b)); \
    mixf(f_ge(a,b)); mixf(f_ge_sw(a,b)); mixf(f_le(a,b)); mixf(f_le_sw(a,b)); \
    mixf(f_alias(a,b,0.75f,-1.25f)); \
} while (0)
static double frombits(u64 u) { double x; memcpy(&x, &u, 8); return x; }
static float frombitsf(unsigned u) { float x; memcpy(&x, &u, 4); return x; }
int main(void) {
    const u64 dbits[] = {
        0x3ff8000000000000ULL, 0xc002000000000000ULL,
        0x0000000000000000ULL, 0x8000000000000000ULL,
        0x4008000000000000ULL, 0xc008000000000000ULL,
        0x7ff0000000000000ULL, 0xfff0000000000000ULL,
        0x7ff8000000000000ULL, 0x7ff0000000000001ULL,
        0xfff8000000000001ULL, 0x0000000000000001ULL
    };
    const unsigned fbits[] = {
        0x3fc00000u, 0xc0100000u, 0x00000000u, 0x80000000u,
        0x40400000u, 0xc0400000u, 0x7f800000u, 0xff800000u,
        0x7fc00000u, 0x7f800001u, 0xffc00001u, 0x00000001u
    };
    H = 1469598103934665603ULL;
    for (unsigned i = 0; i < sizeof(dbits)/sizeof(dbits[0]); ++i)
        for (unsigned j = 0; j < sizeof(dbits)/sizeof(dbits[0]); ++j)
            DALL(frombits(dbits[i]), frombits(dbits[j]));
    mixd(cd_zero()); mixd(cd_zero_sw());
    printf("d=%016llx ", H);
    H = 1469598103934665603ULL;
    for (unsigned i = 0; i < sizeof(fbits)/sizeof(fbits[0]); ++i)
        for (unsigned j = 0; j < sizeof(fbits)/sizeof(fbits[0]); ++j)
            FALL(frombitsf(fbits[i]), frombitsf(fbits[j]));
    mixf(cf_zero()); mixf(cf_zero_sw());
    printf("f=%016llx\n", H);
    return 0;
}
EOF
have_icx=0
if [ "$icx_only" = 1 ]; then
    have_icx=1
elif "$icx" --version 2>/dev/null | grep -c '2026\.2' >/dev/null; then
    have_icx=1
else
    echo "note: ICX 2026.2 not provisioned; GCC and Clang differential legs remain mandatory"
fi

for opt in -O0 -O1 -O2 -O3 -Os; do
    for isa in -mno-avx -mavx; do
        if [ "$icx_only" = 1 ]; then
            "$icx" "$opt" "$isa" "$work/matrix.c" -o "$work/icx" \
                || fail "ICX $opt $isa build"
            expected=$("$work/icx")
            oracle=ICX
        else
            "$gcc" "$opt" "$isa" "$work/matrix.c" -o "$work/gcc" \
                || fail "GCC $opt $isa build"
            "$clang" "$opt" "$isa" "$work/matrix.c" -o "$work/clang" \
                || fail "Clang $opt $isa build"
            expected=$("$work/gcc")
            clang_out=$("$work/clang")
            [ "$clang_out" = "$expected" ] \
                || fail "Clang $opt $isa: '$clang_out' != GCC '$expected'"
            oracle=GCC/Clang
            if [ "$have_icx" = 1 ]; then
                "$icx" "$opt" "$isa" "$work/matrix.c" -o "$work/icx" \
                    || fail "ICX $opt $isa build"
                icx_out=$("$work/icx")
                [ "$icx_out" = "$expected" ] \
                    || fail "ICX $opt $isa: '$icx_out' != GCC '$expected'"
                oracle=GCC/Clang/ICX
            fi
        fi
        "$ccc" "$opt" "$isa" "$work/matrix.c" -o "$work/lccc" \
            || fail "lccc $opt $isa build"
        actual=$("$work/lccc")
        [ "$actual" = "$expected" ] \
            || fail "lccc $opt $isa: '$actual' != $oracle '$expected'"
        echo "PASS: $opt $isa full matrix vs $oracle ($actual)"
    done
done

# Assembly shape: eight strict folds and eight deliberate non-folds, at both
# ISA levels. Checking the complete matrix prevents a width/arm-specific
# mapper from being hidden by a single canonical example.
folds='d_gt:maxsd d_gt_sw:minsd d_lt:minsd d_lt_sw:maxsd f_gt:maxss f_gt_sw:minss f_lt:minss f_lt_sw:maxss'
nonfolds='d_ge d_ge_sw d_le d_le_sw f_ge f_ge_sw f_le f_le_sw'
for isa in -mno-avx -mavx; do
    "$ccc" -O2 "$isa" -S "$work/matrix.c" -o "$work/$isa.s" \
        || fail "shape build $isa"
    prefix=
    [ "$isa" = -mavx ] && prefix=v
    for item in $folds; do
        fn=${item%%:*}
        insn=$prefix${item##*:}
        body=$(sed -n "/^${fn}:/,/^\\.size/p" "$work/$isa.s")
        count=$(grep -cE "^[[:space:]]+${insn}[[:space:]]" <<<"$body" || true)
        [ "$count" -eq 1 ] || fail "$isa $fn: expected one $insn, found $count"
        if [ "$isa" = -mavx ] && grep -qE '^[[:space:]]+(max|min)(sd|ss)[[:space:]]' <<<"$body"; then
            fail "$isa $fn: legacy SSE MIN/MAX survived in AVX mode"
        fi
    done
    for fn in $nonfolds; do
        body=$(sed -n "/^${fn}:/,/^\\.size/p" "$work/$isa.s")
        if grep -qE '^[[:space:]]+v?(max|min)(sd|ss)[[:space:]]' <<<"$body"; then
            fail "$isa $fn: non-strict select folded with different tie semantics"
        fi
    done
    echo "PASS: $isa all 16 predicate/arm/width assembly forms"
done

echo "check_fp_select_minmax: ok"
