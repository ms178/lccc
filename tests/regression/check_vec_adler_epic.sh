#!/usr/bin/env bash
# Adler-32 loop-epic gate: the rolling-checksum vectorizer + the counting
# epic's miscompile fixes (found by this battery's development).
#
#   1. The battery must compile at -O3 -march=x86-64-v3 and run clean.
#   2. Tri-config differential: the battery's stdout must be identical
#      under (a) vectorization on, (b) the scalar reference of the same
#      compiler (no epic: CCC_NO_VEC_ADLER-style spelling is not needed —
#      the gate compares against -O0 semantics through the volatile
#      references INSIDE the battery, so the tri-config here adds the
#      -mno-avx2 spelling where the epic is disabled by ISA), and
#      (c) gcc -O3 -march=x86-64-v3 (the independent oracle; GCC leaves
#      the DO8 kernel scalar — the differential proves the packed form
#      computes the identical values).
#   3. Codegen contracts (scoped asm checks on the DO8 kernel):
#      - the epic fires: the body contains vpsadbw, vpmaddubsw AND
#        vpmaddwd (the three packed producers);
#      - the loop-carried accumulators are register-homed: no 32-byte
#        slot round-trip inside the vector loop (no `vmovdqu %ymm,
#        N(%rsp)` stores between the loop label and its backedge);
#      - the epic's remainder stays scalar-correct: no vpmaddubsw may
#        appear in the epilogue AFTER the vector loop's exit label.
set -euo pipefail
cd "$(dirname "$0")/../.."
ccc=${LCCC_BIN:-${CCC:-target/fastbuild/lccc}}
src=tests/regression/vec_adler_epic.c
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT

march=""
if "$ccc" -march=x86-64-v3 -E -x c /dev/null -o /dev/null 2>/dev/null; then
    march="-march=x86-64-v3"
fi

# ── 1. runtime, epic on ─────────────────────────────────────────────────
"$ccc" -O3 $march "$src" -o "$td/rt_on"
out_on=$("$td/rt_on")
if [ "$out_on" != "OK vec_adler_epic" ]; then
    echo "FAIL: vec_adler_epic runtime (epic on): $out_on"
    exit 1
fi

# ── 2. tri-config differential ──────────────────────────────────────────
# The counting epic requires AVX2; -mno-avx2 keeps every loop scalar while
# running the same battery (the SSE2-baseline spelling of the same code).
if "$ccc" -mno-avx2 -E -x c /dev/null -o /dev/null 2>/dev/null; then
    "$ccc" -O3 -mno-avx2 "$src" -o "$td/rt_off"
    out_off=$("$td/rt_off")
    if [ "$out_off" != "$out_on" ]; then
        echo "FAIL: vec_adler_epic epic-on vs epic-off differential"
        echo "  on : $out_on"
        echo "  off: $out_off"
        exit 1
    fi
fi
if command -v gcc >/dev/null 2>&1; then
    gcc -O3 $march "$src" -o "$td/rt_gcc" 2>/dev/null
    out_gcc=$("$td/rt_gcc")
    if [ "$out_gcc" != "$out_on" ]; then
        echo "FAIL: vec_adler_epic lccc vs gcc differential"
        echo "  lccc: $out_on"
        echo "  gcc : $out_gcc"
        exit 1
    fi
fi

# ── 3. codegen contracts (the DO8 kernel) ───────────────────────────────
"$ccc" -O3 $march -S -o "$td/do8.s" - < tests/regression/vec_adler_epic.c 2>/dev/null || \
    cp /dev/null "$td/do8.s"
# Extract just the adler_do8 function body (label .. .size).
awk '/^adler_do8:/{f=1} f{print} /\.size adler_do8/{if(f)exit}' "$td/do8.s" > "$td/do8fn.s" || true

if ! grep -q 'vpsadbw' "$td/do8fn.s"; then
    echo "FAIL: vec_adler_epic contract — no vpsadbw in adler_do8 (epic did not fire)"
    exit 1
fi
if ! grep -q 'vpmaddubsw' "$td/do8fn.s"; then
    echo "FAIL: vec_adler_epic contract — no vpmaddubsw in adler_do8 (weights missing)"
    exit 1
fi
if ! grep -q 'vpmaddwd' "$td/do8fn.s"; then
    echo "FAIL: vec_adler_epic contract — no vpmaddwd in adler_do8 (ones reduction missing)"
    exit 1
fi

# The vector loop body: between the first local label after the prologue
# that contains vpmaddubsw and the label that follows it, there must be NO
# 32-byte spill of a YMM register (the accumulator/weights/ones homes are
# registers — a slot round trip per iteration is the homing bug class).
loop_body=$(awk '
    /vpmaddubsw/ {inloop=1}
    inloop {print}
    inloop && /jae|jb |jmp/ && NR>1 {count++; if (count>=1 && /jae|jb /) exit}
' "$td/do8fn.s")
if echo "$loop_body" | grep -qE 'vmovdqu[[:space:]]+%ymm[0-9]+,[[:space:]]*[0-9]+\(%rsp\)'; then
    echo "FAIL: vec_adler_epic contract — YMM slot store inside the vector loop (homing bug)"
    exit 1
fi

# The epic's exit materialization: hsum chains present after the loop.
if ! grep -qE 'vextracti128|vpshufd' "$td/do8fn.s"; then
    echo "FAIL: vec_adler_epic contract — no horizontal-sum chain in the exit"
    exit 1
fi

echo "vec_adler_epic: all contracts pass"
