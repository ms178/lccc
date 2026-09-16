#!/usr/bin/env bash
# BB-SLP red-team gate: runtime correctness + per-function codegen contract
# for the soundness holes closed by the 2026-09-16 semantic audit.
#
#   1. The battery must compile at -O2 -march=x86-64-v3 and run clean.
#   2. rt_fwd (store→load forwarding inside a seed) must contain NO vector
#      store: rule (e) rejects the seed (the deferred vector load would
#      read the pre-store value).
#   3. rt_xseed's second seed must not reorder its loads across the first
#      seed's vector store — verified by runtime under aliasing args.
#   4. rt_precise (disjoint same-stream store/load byte ranges) MUST
#      vectorize: rule (e)'s range check must not over-reject.
#   5. rt_w16/rt_w8 (restrict-qualified 20-lane u16 / 34-lane u8 runs)
#      MUST pack at the widest family the run supports — under AVX2 that
#      is I16x16 / I8x32 for the leading 16/32 lanes (one 256-bit op,
#      strictly better than the two 128-bit seeds the pre-v3 fallback
#      produced) — not be dropped.
#   6. rt_phi must contain no vector store: cross-block phi uses reject.
#      rt_w16_norestr (no restrict) must also stay scalar: rule (e)'s
#      cross-stream hazard check — the shape GCC miscompiles under a
#      shifted alias.
set -euo pipefail
cd "$(dirname "$0")/../.."
ccc=${LCCC_BIN:-${CCC:-target/fastbuild/lccc}}
src=tests/regression/bb_slp_redteam.c
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT

march=$("$ccc" -march=x86-64-v3 -E -x c /dev/null -o /dev/null 2>/dev/null \
    && echo "-march=x86-64-v3" || echo "")

# ── 1. runtime ──────────────────────────────────────────────────────────
"$ccc" -O2 $march "$src" -o "$td/rt"
out=$("$td/rt")
if [ "$out" != "bb_slp_redteam: all pass (0 fails)" ]; then
    echo "FAIL: bb_slp_redteam runtime output: $out"
    exit 1
fi

# ── 2–6. per-function codegen contracts ────────────────────────────────
"$ccc" -O2 $march -S "$src" -o "$td/rt.s"

scoped() {  # scoped <func> — the function's asm slice (label to next fn)
    awk -v f="$1" '
        $0 ~ "^"f":" { infn = 1; next }
        infn && /^[a-zA-Z_][a-zA-Z0-9_]*:/ { exit }
        infn { print }
    ' "$td/rt.s"
}

n_vec_fwd=$(scoped rt_fwd | grep -cE "vmovdqu|movdqu" || true)
if [ "$n_vec_fwd" -gt 0 ]; then
    echo "FAIL: rt_fwd vectorized despite the store→load forwarding hazard ($n_vec_fwd vector stores)"
    exit 1
fi

n_vec_phi=$(scoped rt_phi | grep -cE "vmovdqu|movdqu" || true)
if [ "$n_vec_phi" -gt 0 ]; then
    echo "FAIL: rt_phi vectorized despite cross-block phi lane uses ($n_vec_phi vector stores)"
    exit 1
fi

n_vec_nr=$(scoped rt_w16_norestr | grep -cE "vmovdqu|movdqu|paddw|vpaddw" || true)
if [ "$n_vec_nr" -gt 0 ]; then
    echo "FAIL: rt_w16_norestr vectorized without restrict (cross-stream hazard) ($n_vec_nr)"
    exit 1
fi

if [ -n "$march" ]; then
    # 256-bit seed: the 4×i64 copy+add packs. The VEX memfold may fold the
    # stream load into the packed add (`vpaddq (%rdi), %ymm0, %ymm0`), so
    # the vectorized signature is the packed op itself plus the store —
    # counting only vmovdqu would reject the FOLDED (better) code.
    n_vec_precise=$(scoped rt_precise | grep -cE "vpaddq|vmovdqu" || true)
    if [ "$n_vec_precise" -lt 2 ]; then
        echo "FAIL: rt_precise did not vectorize (rule (e) over-rejects: $n_vec_precise vector ops)"
        exit 1
    fi
    n_w16=$(scoped rt_w16 | grep -cE "paddw|vpaddw" || true)
    n_w16_v=$(scoped rt_w16 | grep -cE "vpaddw" || true)
    if [ "$n_w16_v" -lt 1 ] && [ "$n_w16" -lt 2 ]; then
        echo "FAIL: rt_w16 did not pack (I16x16 preferred, I16x8 fallback: $n_w16 ops, $n_w16_v ymm)"
        exit 1
    fi
    n_w8=$(scoped rt_w8 | grep -cE "paddb|vpaddb" || true)
    n_w8_v=$(scoped rt_w8 | grep -cE "vpaddb" || true)
    if [ "$n_w8_v" -lt 1 ] && [ "$n_w8" -lt 2 ]; then
        echo "FAIL: rt_w8 did not pack (I8x32 preferred, I8x16 fallback: $n_w8 ops, $n_w8_v ymm)"
        exit 1
    fi
fi

echo "OK: bb_slp_redteam runtime + codegen contracts (fwd/phi/norestr scalar, precise/w16/w8 vectorized)"
