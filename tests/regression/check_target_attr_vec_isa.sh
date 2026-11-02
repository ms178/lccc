#!/usr/bin/env bash
# Per-function `__attribute__((target(...)))` must choose the vectorizer's
# register width.  The TU's `-march` must not be the only thing that can.
#
# `slp_vectorizer::family_for` gates every double-width arm on AVX2
# ((I16,16), (I8,32), (I32,8), (I64,4), (F32,8), (F64,4) are all `if avx2`;
# the widest ungated arms are half that), so the ISA the pass sees is exactly
# what separates one 256-bit op from two 128-bit ones.  Three middle-end
# entry points used to read the TU baseline instead of the function's own
# fold:
#
#   slp        (slp_vectorizer::run_bb_slp_with_contract)
#   slp_late   (the same pass, second invocation)
#   vec_arx    (vec_arx::vec_arx_function)
#
# while six sibling vector entry points already ran inside
# `with_function_x86_vec_isa`.  `vec_ssa`'s promotion table was worse: it was
# built once per module from the TU baseline, so an AVX2 function's ymm
# families were absent from it and its vector objects were left in memory.
#
# The corpus pairs each attributed function with a byte-identical unattributed
# twin, so the ONLY thing that can separate their codegen is the per-function
# ISA fold.  Asserted per function:
#
#   1. runtime correctness, and the two twins agreeing bit for bit;
#   2. the `target("avx2")` twin emits ymm;
#   3. the unattributed twin emits NO ymm - the ISA gate still holds, so this
#      is not a global AVX2 enable in disguise;
#   4. the unattributed twin still emits xmm, which is what stops (3) passing
#      vacuously because the pass did not fire at all.
#
# Compiled with no -march and no -mavx2: a baseline (SSE2) TU is the whole
# point.
set -euo pipefail
cd "$(dirname "$0")/../.."

if [ "$(uname -m)" != "x86_64" ] || [ "$(uname -s)" != "Linux" ]; then
    echo "SKIP: target-attribute vector ISA contract is x86-64/Linux only"
    exit 0
fi

ccc=${LCCC_BIN:-${CCC:-target/fastbuild/lccc}}
src=tests/regression/target_attr_vec_isa.c
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT

# ── 1. runtime ──────────────────────────────────────────────────────────
"$ccc" -O2 "$src" -o "$td/rt"
out=$("$td/rt")
if [ "$out" != "target_attr_vec_isa: all pass (0 fails)" ]; then
    echo "FAIL: target_attr_vec_isa runtime output: $out"
    exit 1
fi

# ── 2-4. per-function codegen contract, baseline TU ────────────────────
"$ccc" -O2 -S "$src" -o "$td/rt.s"

scoped() {  # scoped <func> — the function's asm slice (label to next fn)
    awk -v f="$1" '
        $0 ~ "^"f":" { infn = 1; next }
        infn && /^[a-zA-Z_][a-zA-Z0-9_]*:/ { exit }
        infn { print }
    ' "$td/rt.s"
}

check_pair() {  # check_pair <attributed> <plain> <shape>
    local wide="$1" plain="$2" shape="$3"
    local n_ymm_wide n_ymm_plain n_xmm_plain

    n_ymm_wide=$(scoped "$wide" | grep -c "ymm" || true)
    if [ "$n_ymm_wide" -lt 1 ]; then
        echo "FAIL: $wide did not widen to 256-bit under target(\"avx2\")" \
             "($shape: $n_ymm_wide ymm refs; the per-function ISA fold is" \
             "not reaching the vectorizer)"
        exit 1
    fi

    n_ymm_plain=$(scoped "$plain" | grep -c "ymm" || true)
    if [ "$n_ymm_plain" -gt 0 ]; then
        echo "FAIL: $plain emitted ymm without asking for AVX2" \
             "($shape: $n_ymm_plain ymm refs; the ISA gate leaked, or this" \
             "is a global enable rather than a per-function one)"
        exit 1
    fi

    n_xmm_plain=$(scoped "$plain" | grep -c "xmm" || true)
    if [ "$n_xmm_plain" -lt 1 ]; then
        echo "FAIL: $plain did not vectorize at all ($shape: no xmm), so the" \
             "absence of ymm above proves nothing - widen the corpus shape"
        exit 1
    fi

    echo "  ok: $shape  $wide=$n_ymm_wide ymm / $plain=$n_xmm_plain xmm, 0 ymm"
}

echo "target-attribute vector ISA contract (baseline TU, -O2):"
check_pair tgt_h20 base_h20 "20-lane u16 (I16x16 vs I16x8)"
check_pair tgt_b34 base_b34 "34-lane u8 (I8x32 vs I8x16)"

echo "OK: per-function target(\"avx2\") widens BB-SLP to ymm; unattributed twins stay xmm"
