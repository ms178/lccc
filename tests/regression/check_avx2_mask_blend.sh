#!/usr/bin/env bash
# `_mm256_movemask_epi8` and `_mm256_blend_epi32` must be one instruction each.
#
# Both were scalar loops over the whole 32-byte vector: the movemask spilled to
# the stack and branched on all 32 bytes, the blend wrote eight lanes by hand.
# A byte-classification loop (JSON/XML scanning, UTF-8 validation, memchr
# overlays) spends its whole inner loop in `vpmovmskb`, so the loop was the hot
# code of every caller.
#
# Usage: check_avx2_mask_blend.sh [--lccc PATH] [--reference CC]
set -uo pipefail

LCCC="${LCCC:-./target/fastbuild/lccc}"
REFERENCE="${REFERENCE:-gcc-16}"
while [ $# -gt 0 ]; do
    case "$1" in
        --lccc) LCCC="$2"; shift 2 ;;
        --reference) REFERENCE="$2"; shift 2 ;;
        *) echo "check_avx2_mask_blend: unknown argument '$1'" >&2; exit 2 ;;
    esac
done

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
FIXTURE=tests/regression/simd_avx2_mask_blend.c

if [ ! -x "$LCCC" ]; then
    echo "FAIL: no compiler at '$LCCC'" >&2
    exit 1
fi

fail=0
bad() { echo "FAIL: $*" >&2; fail=1; }

body_of() {
    awk -v fn="$2:" 'index($0, fn) == 1 {inside=1; next}
                     /^[a-z_][A-Za-z_0-9]*:/ {inside=0} inside' "$1"
}

for level in -O1 -O2; do
    ASM="$WORK/f.s"
    "$LCCC" "$level" -mavx2 -S -o "$ASM" "$FIXTURE" || { echo "FAIL: compile at $level" >&2; exit 1; }
    check_op() {
        body_of "$ASM" "$1" > "$WORK/body.s"
        if [ ! -s "$WORK/body.s" ]; then bad "$level: cannot extract $1"; return; fi
        got=$(grep -cE "^[[:space:]]+$2[[:space:]]" "$WORK/body.s")
        if [ "$got" -ne "$3" ]; then bad "$level: $1 emits $got '$2' (want $3)"; fi
    }
    check_op mask_of vpmovmskb 1
    check_op blend_a5 vblendps 1
    check_op blend_00 vblendps 1
    check_op blend_ff vblendps 1
    check_op blend_55 vblendps 1
    # No per-byte/per-lane work beside them.
    for fn in mask_of blend_a5 blend_00 blend_ff blend_55; do
        body_of "$ASM" "$fn" > "$WORK/body.s"
        n=$(grep -cE '^[[:space:]]+(vmov(ss|sd|q|d)|movzbl|movsbl)[[:space:]]' "$WORK/body.s")
        if [ "$n" -ne 0 ]; then
            bad "$level: $fn still moves $n byte/lane(s) individually"
        fi
    done
done

for level in -O0 -O1 -O2 -O3; do
    "$LCCC" "$level" -mavx2 "$FIXTURE" -o "$WORK/bin" || { bad "$level: does not link"; continue; }
    "$WORK/bin" > "$WORK/out" || { bad "$level: exits non-zero"; continue; }
    if command -v "$REFERENCE" >/dev/null 2>&1; then
        "$REFERENCE" -O2 -mavx2 "$FIXTURE" -o "$WORK/ref.bin" || { echo "FAIL: reference cannot build the fixture" >&2; exit 1; }
        "$WORK/ref.bin" > "$WORK/ref.out"
        cmp -s "$WORK/out" "$WORK/ref.out" || { bad "$level: output differs from $REFERENCE"; diff "$WORK/out" "$WORK/ref.out" >&2 || true; }
    fi
done

if [ "$fail" -ne 0 ]; then
    echo "FAIL: AVX2 mask/blend lowering"
    exit 1
fi
echo "PASS: movemask and blend_epi32 are one instruction and match the reference"
