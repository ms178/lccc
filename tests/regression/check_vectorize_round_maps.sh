#!/usr/bin/env bash
set -euo pipefail
CCC=${CCC:-target/fastbuild/lccc}
GCC=${GCC:-gcc}
TD=$(mktemp -d)
trap 'rm -rf "$TD"' EXIT
SRC=tests/regression/vectorize_round_maps/vectorize_round_maps.c
fail() { echo "FAIL: $*" >&2; exit 1; }
body() { sed -n "/^$2:/,/^\.size $2/p" "$1"; }

"$CCC" -O3 -march=x86-64-v3 -S "$SRC" -o "$TD/v3.s"
for spec in \
    'floor_f64 vroundpd.*\$9' 'floor_f32 vroundps.*\$9' \
    'ceil_f64 vroundpd.*\$10' 'ceil_f32 vroundps.*\$10' \
    'trunc_f64 vroundpd.*\$11' 'trunc_f32 vroundps.*\$11' \
    'rint_f64 vroundpd.*\$4' 'rint_f32 vroundps.*\$4' \
    'nearby_f64 vroundpd.*\$12' 'nearby_f32 vroundps.*\$12' \
    'roundeven_f64 vroundpd.*\$8' 'roundeven_f32 vroundps.*\$8'; do
    set -- $spec
    body "$TD/v3.s" "$1" | grep -Ec "$2" >/dev/null || fail "$1 lacks its packed rounding mode"
done
for spec in 'sqrt_f64 vsqrtpd' 'sqrt_f32 vsqrtps'; do
    set -- $spec
    body "$TD/v3.s" "$1" | grep -Ec "$2" >/dev/null || fail "$1 lacks packed square root"
done
body "$TD/v3.s" floor_f64 | grep -Ec 'vroundpd.*\(' >/dev/null || fail 'round failed to fold its source load'
body "$TD/v3.s" ceil_f32 | grep -Ec 'vroundps.*\(' >/dev/null || fail 'roundf failed to fold its source load'
body "$TD/v3.s" sqrt_f64 | grep -Ec 'vsqrtpd.*\(' >/dev/null || fail 'sqrt failed to fold its source load'
body "$TD/v3.s" sqrt_f32 | grep -Ec 'vsqrtps.*\(' >/dev/null || fail 'sqrtf failed to fold its source load'
# Dynamic signed-long bounds need a guarded byte limit, not a second hot-loop
# induction variable. Between the packed op and its backedge there must be only
# the byte-IV increment; an element-counter `addq $1` costs every vector trip.
for fn in floor_f64 floor_f32; do
    hot=$(body "$TD/v3.s" "$fn" | sed -n '/vroundp[sd].*(/,/jl /p' | sed -n '1,20p')
    [ "$(printf '%s\n' "$hot" | grep -Ec 'addq')" -eq 1 ] || fail "$fn kept two hot induction increments"
    printf '%s\n' "$hot" | grep -Ec 'addq[[:space:]]+\$32' >/dev/null || fail "$fn lacks its byte-IV increment"
done
# Register-source legs are independently required: a memory fold alone must
# not hide a backend that cannot carry a computed packed value into round/sqrt.
for spec in \
    'floor_add_f64 vaddpd' 'floor_add_f64 vroundpd.*%ymm' \
    'floor_add_f32 vaddps' 'floor_add_f32 vroundps.*%ymm' \
    'sqrt_add_f64 vaddpd' 'sqrt_add_f64 vsqrtpd.*%ymm' \
    'sqrt_add_f32 vaddps' 'sqrt_add_f32 vsqrtps.*%ymm'; do
    set -- $spec
    body "$TD/v3.s" "$1" | grep -Ec "$2" >/dev/null || fail "$1 lacks register-source packed chain: $2"
done
# Register-to-register unary chains must remain in the SIMD scratch register:
# a dead stack home between load, round/sqrt and store erased most of the win.
for fn in floor_f64 floor_f32 sqrt_f64 sqrt_f32 \
          floor_add_f64 floor_add_f32 sqrt_add_f64 sqrt_add_f32; do
    if body "$TD/v3.s" "$fn" | grep -Ec 'vmovdqu.*%rsp|vmovdqu.*\(%rsp\)' >/dev/null; then
        fail "$fn regressed to dead vector stack traffic"
    fi
done
# The negative controls are part of legality, not merely shape preferences.
CCC_NO_MAP_VEC=1 "$CCC" -O3 -march=x86-64-v3 -S "$SRC" -o "$TD/off.s"
if grep -Eq 'vroundp[sd]' "$TD/off.s"; then
    fail 'map kill switch did not restore scalar rounding'
fi
"$CCC" -O3 -march=x86-64 -S "$SRC" -o "$TD/sse2.s"
if grep -Eq '(^|[[:space:]])v?roundp[sd]' "$TD/sse2.s"; then
    fail 'SSE4.1 packed rounding leaked into the SSE2 baseline'
fi
"$CCC" -O3 -march=x86-64-v3 -mno-avx -S "$SRC" -o "$TD/sse41.s"
grep -Eq '(^|[[:space:]])roundpd' "$TD/sse41.s" || fail '128-bit F64 rounding fallback missing'
grep -Eq '(^|[[:space:]])roundps' "$TD/sse41.s" || fail '128-bit F32 rounding fallback missing'
if grep -Eq '(^|[[:space:]])vroundp[sd]' "$TD/sse41.s"; then
    fail 'VEX packed rounding leaked through -mno-avx'
fi

# Integrated assembly and full runtime parity over every tail length, signed
# zero, infinities, fractional boundaries and quiet-NaN payloads.
"$CCC" -O3 -march=x86-64-v3 -DTEST_MAIN "$SRC" -lm -o "$TD/lccc"
"$GCC" -O3 -march=x86-64-v3 -DTEST_MAIN "$SRC" -lm -o "$TD/gcc"
"$TD/lccc" >"$TD/lccc.out"
"$TD/gcc" >"$TD/gcc.out"
cmp "$TD/lccc.out" "$TD/gcc.out" || fail 'packed/scalar-tail results differ from GCC'

# i686 currently uses its exact x87 scalar implementation: the x86-64 map
# family must neither leak YMM ops nor silently drop a result on the 32-bit
# backend. Exercise every mode and tail there as a mandatory cross-leg.
"$CCC" -m32 -O3 -msse4.1 -S "$SRC" -o "$TD/i686.s"
if grep -Eq 'v?roundp[sd]|v?sqrtp[sd]' "$TD/i686.s"; then
    fail 'x86-64 packed map leaked into i686 lowering'
fi
"$CCC" -m32 -O3 -msse4.1 -DTEST_MAIN "$SRC" -lm -o "$TD/lccc32"
"$GCC" -m32 -O3 -msse4.1 -DTEST_MAIN "$SRC" -lm -o "$TD/gcc32"
"$TD/lccc32" >"$TD/lccc32.out"
"$TD/gcc32" >"$TD/gcc32.out"
cmp "$TD/lccc32.out" "$TD/gcc32.out" || fail 'i686 scalar rounding differs from GCC'

echo 'vector round maps: PASS'
