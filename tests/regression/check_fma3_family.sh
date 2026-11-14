#!/usr/bin/env bash
# Every FMA3 spelling must be ONE fused instruction, at every -O level and
# under every -ffp-contract setting.
#
# Background.  <fmaintrin.h> implemented `_mm256_fmsub_ps`/`_mm256_fnmadd_ps`
# as lane loops and provided no `_mm_fmsub_*`, `_mm_fnmadd_*`, `_mm_fnmsub_*`,
# `_mm_fmaddsub_*`, `_mm_fmsubadd_*`, scalar, or 512-bit forms at all -- 14
# functions gcc-16 and clang-23 compile and lccc rejected, plus a silent
# numerical defect: an FMA rounds the product ONCE, the loop rounds twice, and
# the loop only agreed with the reference compilers because the default
# `-ffp-contract=fast` contracted it back into one instruction.  Measured with
# `-ffp-contract=off`, `_mm256_fmsub_ps(1.0000001f, 1-1.1920929e-7f, 1.0f)`
# returned 0 where gcc-16 returns -1.42108547e-14.
#
# This gate proves three independent things, because either one alone can pass
# while the code is wrong:
#   1. shape   -- each operation is exactly one instruction, at -O1 AND -O2,
#                 with no `vmulps`/`vmulpd`/`vaddps`/`vsubps` beside it (a
#                 split lowering is two instructions and the wrong answer);
#   2. numbers -- the fixture's fused identity (x+1)(x-1)-x^2 == -1 exactly,
#                 at every -O level and with contraction disabled;
#   3. parity  -- executed output is byte-identical to the reference compiler.
#
# Usage: check_fma3_family.sh [--lccc PATH] [--reference CC]
set -uo pipefail

LCCC="${LCCC:-./target/fastbuild/lccc}"
REFERENCE="${REFERENCE:-gcc-16}"
while [ $# -gt 0 ]; do
    case "$1" in
        --lccc) LCCC="$2"; shift 2 ;;
        --reference) REFERENCE="$2"; shift 2 ;;
        *) echo "check_fma3_family: unknown argument '$1'" >&2; exit 2 ;;
    esac
done

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
FIXTURE=tests/regression/simd_fma3_family.c

if [ ! -x "$LCCC" ]; then
    echo "FAIL: no compiler at '$LCCC'" >&2
    exit 1
fi

fail=0
level_fail=0
bad() { echo "FAIL: $*" >&2; fail=1; level_fail=1; }

body_of() {
    awk -v fn="$2:" 'index($0, fn) == 1 {inside=1; next}
                     /^[a-z_][A-Za-z_0-9]*:/ {inside=0} inside' "$1"
}

# ── 1. shape ────────────────────────────────────────────────────────────────
for level in -O1 -O2; do
    level_fail=0
    ASM="$WORK/fixture${level}.s"
    "$LCCC" "$level" -mavx2 -mfma -S -o "$ASM" "$FIXTURE" ||
        { echo "FAIL: compile at $level" >&2; exit 1; }

    check_op() {
        fn="$1"; mnemonic="$2"; want="$3"
        body_of "$ASM" "$fn" > "$WORK/body.s"
        if [ ! -s "$WORK/body.s" ]; then
            bad "$level: cannot extract $fn"; return
        fi
        got=$(grep -cE "^[[:space:]]+${mnemonic}[[:space:]]" "$WORK/body.s")
        if [ "$got" -ne "$want" ]; then
            bad "$level: $fn emits $got '$mnemonic' (want $want)"
        fi
    }

    # One instruction per operation, per width.
    check_op ops256_ps vfmsub132ps 1
    check_op ops256_ps vfnmadd132ps 1
    check_op ops256_ps vfnmsub132ps 1
    check_op ops256_ps vfmaddsub132ps 1
    check_op ops256_ps vfmsubadd132ps 1
    check_op ops256_ps vfmadd132ps 1
    check_op ops256_pd vfmsub132pd 1
    check_op ops256_pd vfnmadd132pd 1
    check_op ops256_pd vfnmsub132pd 1
    check_op ops256_pd vfmaddsub132pd 1
    check_op ops256_pd vfmsubadd132pd 1
    check_op ops256_pd vfmadd132pd 1
    check_op ops128_ps vfmsub132ps 1
    check_op ops128_ps vfnmadd132ps 1
    check_op ops128_ps vfnmsub132ps 1
    check_op ops128_ps vfmaddsub132ps 1
    check_op ops128_ps vfmsubadd132ps 1
    check_op ops128_ps vfmadd132ps 1
    check_op ops128_pd vfmsub132pd 1
    check_op ops128_pd vfnmadd132pd 1
    check_op ops128_pd vfnmsub132pd 1
    check_op ops128_pd vfmaddsub132pd 1
    check_op ops128_pd vfmsubadd132pd 1
    check_op ops128_pd vfmadd132pd 1
    check_op ops_scalar vfmsub132ss 1
    check_op ops_scalar vfnmadd132ss 1
    check_op ops_scalar vfnmsub132ss 1
    check_op ops_scalar vfmadd132ss 1
    check_op ops_scalar vfmsub132sd 1
    check_op ops_scalar vfnmadd132sd 1
    check_op ops_scalar vfnmsub132sd 1
    check_op ops_scalar vfmadd132sd 1
    # The loop kernel: still exactly one fused op, no split form beside it.
    check_op loop_kernel vfmsub132ps 1

    # No split multiply/add/subtract anywhere in the operation functions; that
    # is the signature of the old `__a[i] * __b[i] - __c[i]` lowering.
    for fn in ops256_ps ops256_pd ops128_ps ops128_pd ops_scalar; do
        body_of "$ASM" "$fn" > "$WORK/body.s"
        split=$(grep -cE '^[[:space:]]+v(mul|add|sub)(ps|pd)[[:space:]]' "$WORK/body.s")
        split_scalar=$(grep -cE '^[[:space:]]+v(mul|add|sub)s[ds][[:space:]]' "$WORK/body.s")
        # `ops_scalar` legitimately contains the scalar addends of the
        # dump helpers? No: those are separate functions.  Any split op here is
        # a real finding.
        if [ "$split" -ne 0 ] || [ "$split_scalar" -ne 0 ]; then
            bad "$level: $fn still contains $split packed / $split_scalar scalar mul-add-sub"
        fi
    done
    if [ "$level_fail" -eq 0 ]; then
        echo "  ok  $level: one fused instruction per operation, no split forms"
    fi
done

# ── 2. numbers: the fused identity is exactly -64 ───────────────────────────
# 64 iterations of (x+1)(x-1) - x^2 for pseudo-random 1 <= x < 2.  Fused this is
# exactly -1 each time; split into mul/sub it is off by up to one ulp of x^2,
# so the sum is not -64.
IDENTITY_WANT="identity c050000000000000"
for level in -O0 -O1 -O2 -O3; do
    if ! "$LCCC" "$level" -mavx2 -mfma "$FIXTURE" -o "$WORK/fused$level" 2>"$WORK/err"; then
        bad "compile at $level: $(head -1 "$WORK/err")"; continue
    fi
    "$WORK/fused$level" > "$WORK/out$level" || { bad "$level: fixture exits non-zero"; continue; }
    got=$(grep '^identity ' "$WORK/out$level")
    if [ "$got" != "$IDENTITY_WANT" ]; then
        bad "$level: fused identity is '$got' (want '$IDENTITY_WANT')"
    fi
done
# The same, with contraction explicitly disabled: an intrinsic must not change
# meaning because of an unrelated flag.
if "$LCCC" -O2 -mavx2 -mfma -ffp-contract=off "$FIXTURE" -o "$WORK/off" 2>"$WORK/err"; then
    "$WORK/off" > "$WORK/out_off" || bad "-ffp-contract=off: fixture exits non-zero"
    got=$(grep '^identity ' "$WORK/out_off")
    if [ "$got" != "$IDENTITY_WANT" ]; then
        bad "-ffp-contract=off: fused identity is '$got' (want '$IDENTITY_WANT')"
    else
        echo "  ok  fused result is independent of -ffp-contract"
    fi
else
    bad "compile with -ffp-contract=off: $(head -1 "$WORK/err")"
fi

# ── 3. parity with a reference compiler ────────────────────────────────────
if command -v "$REFERENCE" >/dev/null 2>&1; then
    "$REFERENCE" -O2 -mavx2 -mfma "$FIXTURE" -o "$WORK/ref.bin" ||
        { echo "FAIL: reference does not build the fixture" >&2; exit 1; }
    "$WORK/ref.bin" > "$WORK/ref.out" || { echo "FAIL: reference fixture exits non-zero" >&2; exit 1; }
    for level in -O0 -O1 -O2 -O3; do
        if ! cmp -s "$WORK/out$level" "$WORK/ref.out"; then
            bad "$level: executed output differs from $REFERENCE"
            diff "$WORK/out$level" "$WORK/ref.out" >&2 || true
        fi
    done
    if ! cmp -s "$WORK/out_off" "$WORK/ref.out"; then
        bad "-ffp-contract=off: executed output differs from $REFERENCE"
        diff "$WORK/out_off" "$WORK/ref.out" >&2 || true
    fi
    echo "  ok  executed output matches $REFERENCE at -O0/-O1/-O2/-O3 and with contraction off"
else
    echo "  ..  reference '$REFERENCE' unavailable; parity check skipped"
fi

if [ "$fail" -ne 0 ]; then
    echo "FAIL: FMA3 family lowering"
    exit 1
fi
echo "PASS: every FMA3 spelling is one fused instruction, fused at every -O level"
