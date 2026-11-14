#!/usr/bin/env bash
# AVX 256-bit float bitwise operations must be ONE instruction each.
#
# `_mm256_and/or/xor/andnot_(ps|pd)` are lane-agnostic bitwise ops.  The header
# used to implement the `pd` forms as element-wise `union` loops (and did not
# provide the `ps` forms at all), which lowered to a per-lane GPR round trip:
#
#   vmovsd 40(%rsp), %xmm2      ; lane 0 in
#   vmovq  %xmm2, %rax
#   xorq   %rcx, %rax
#   vmovq  %rax, %xmm0
#   vmovsd %xmm0, 8(%rsp)       ; lane 0 out ... four times, plus the reload
#
# instead of the single `vxorpd %ymm1, %ymm0, %ymm0` the operation is.  This
# gate pins the emitted shape for every one of the eight float ops and for the
# integer `and`/`andnot` forms the float ones are built from, and executes the
# fixture against a reference compiler so a "fixed" shape that computes the
# wrong bytes fails rather than passing on instruction counts.
#
# Usage: check_avx256_float_bitwise.sh [--lccc PATH] [--reference CC]
set -uo pipefail

LCCC="${LCCC:-./target/fastbuild/lccc}"
REFERENCE="${REFERENCE:-gcc-16}"
while [ $# -gt 0 ]; do
    case "$1" in
        --lccc) LCCC="$2"; shift 2 ;;
        --reference) REFERENCE="$2"; shift 2 ;;
        *) echo "check_avx256_float_bitwise: unknown argument '$1'" >&2; exit 2 ;;
    esac
done

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
FIXTURE=tests/regression/simd_avx256_float_bitwise.c
ASM="$WORK/fixture.s"

if [ ! -x "$LCCC" ]; then
    echo "FAIL: no compiler at '$LCCC'" >&2
    exit 1
fi

fail=0
bad() { echo "FAIL: $*" >&2; fail=1; }

"$LCCC" -O2 -mavx2 -S -o "$ASM" "$FIXTURE" || { echo "FAIL: compile for -S" >&2; exit 1; }

# ── Shape: one instruction per operation inside each op function ────────────
check_op() {
    fn="$1"; mnemonic="$2"; want="$3"
    if ! awk -v fn="$fn:" 'index($0, fn) == 1 {inside=1; next} /^[a-z_][A-Za-z_0-9]*:/ {inside=0} inside' \
            "$ASM" > "$WORK/body.s"; then
        bad "cannot extract $fn"
        return
    fi
    got=$(grep -cE "^[[:space:]]+${mnemonic}[[:space:]]" "$WORK/body.s")
    if [ "$got" -ne "$want" ]; then
        bad "$fn emits $got '$mnemonic' (want $want)"
    else
        echo "  ok  $fn: $got x $mnemonic"
    fi
}

# Four operations per float function: and, or, xor, andnot.
check_op pd_ops vpand 1
check_op pd_ops vpor 1
check_op pd_ops vpxor 1
check_op pd_ops vpandn 1
check_op ps_ops vpand 1
check_op ps_ops vpor 1
check_op ps_ops vpxor 1
check_op ps_ops vpandn 1
check_op si_ops vpand 1
check_op si_ops vpor 1
check_op si_ops vpxor 1
check_op si_ops vpandn 1
# The loop carries one xor, one and, one or and one andnot.  The second
# `vpxor` is the accumulator's zeroing idiom (`vpxor %ymm,%ymm,%ymm`), which
# is the cheapest way to materialise the zero vector.
check_op loop_mix vpxor 2
check_op loop_mix vpand 1
check_op loop_mix vpor 1
check_op loop_mix vpandn 1

# ── Shape: no per-lane GPR round trip anywhere in the file ──────────────────
# `vmovq %xmmN, %r..` is the lane-extraction signature of the old lowering.  A
# legitimate 64-bit lane move writes to an XMM destination, so this pattern is
# unambiguous.
gpr_lanes=$(grep -cE 'vmovq[[:space:]]+%xmm[0-9]+, %(r|e)' "$ASM")
if [ "$gpr_lanes" -ne 0 ]; then
    bad "$gpr_lanes per-lane GPR extractions remain"
else
    echo "  ok  no per-lane GPR extraction in the file"
fi

# ── Execution: the fixture must agree with the reference compiler ───────────
"$LCCC" -O2 -mavx2 "$FIXTURE" -o "$WORK/lccc.bin" || bad "fixture does not link"
"$WORK/lccc.bin" > "$WORK/lccc.out" || bad "fixture exits non-zero"
if command -v "$REFERENCE" >/dev/null 2>&1; then
    "$REFERENCE" -O2 -mavx2 "$FIXTURE" -o "$WORK/ref.bin" || bad "reference does not build the fixture"
    "$WORK/ref.bin" > "$WORK/ref.out" || bad "reference fixture exits non-zero"
    if cmp -s "$WORK/lccc.out" "$WORK/ref.out"; then
        echo "  ok  executed output matches $REFERENCE"
    else
        bad "executed output differs from $REFERENCE"
        diff "$WORK/lccc.out" "$WORK/ref.out" >&2 || true
    fi
else
    echo "  ..  reference '$REFERENCE' unavailable; executed comparison skipped"
fi

if [ "$fail" -ne 0 ]; then
    echo "FAIL: AVX-256 float bitwise lowering"
    exit 1
fi
echo "PASS: every AVX-256 bitwise op lowers to one instruction and matches the reference"
