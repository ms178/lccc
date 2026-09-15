#!/usr/bin/env bash
# BB-SLP v3 gate: constant-lane materialization contracts (runtime +
# per-function asm). See tests/regression/bb_slp_v3.c for the defect
# provenance of every shape.
#
#   1. The battery must compile at -O2 -march=x86-64-v3 and run clean.
#   2. v3_gather_fp_const must stage the FP constant lane through its
#      BIT PATTERN (movabsq) — the old path zeroed it.
#   3. v3_zstore_f64 / _i32 / _i64 must emit the zero vector + ONE
#      vector store (the root-Splat scheduling fix); no 4-scalar-store
#      fallback.
#   4. v3_ones_i32 must materialize all-ones with the self-compare
#      (pcmpeqd) and one store; v3_ones_i64 with vpcmpeqd on the 256-bit
#      family.
#   5. v3_c15_f64 must broadcast from the const pool (vbroadcastsd); no
#      per-lane movsd stores.
set -euo pipefail
cd "$(dirname "$0")/../.."
ccc=${LCCC_BIN:-${CCC:-target/fastbuild/lccc}}
src=tests/regression/bb_slp_v3.c
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT

march=$("$ccc" -march=x86-64-v3 -E -x c /dev/null -o /dev/null 2>/dev/null \
    && echo "-march=x86-64-v3" || echo "")

# ── 1. runtime ──────────────────────────────────────────────────────────
"$ccc" -O2 $march "$src" -o "$td/rt" -lm
out=$("$td/rt")
if [ "$out" != "bb_slp_v3: all pass (0 fails)" ]; then
    echo "FAIL: bb_slp_v3 runtime output: $out"
    exit 1
fi

# ── 2–5. per-function codegen contracts ────────────────────────────────
"$ccc" -O2 $march -S "$src" -o "$td/rt.s"

scoped() {  # scoped <func> — the function's asm slice (label to next fn)
    awk -v f="$1" '
        $0 ~ "^"f":" { infn = 1; next }
        infn && /^[a-zA-Z_][a-zA-Z0-9_]*:/ { exit }
        infn { print }
    ' "$td/rt.s"
}

# 2. FP const gather lane: bit-pattern staging, never the zeroing path.
n_movabs=$(scoped v3_gather_fp_const | grep -cE "movabsq \\\$4609434218613702656" || true)
if [ "$n_movabs" -lt 1 ]; then
    echo "FAIL: v3_gather_fp_const must movabsq the 1.5 bit pattern (got $n_movabs)"
    exit 1
fi
n_vst=$(scoped v3_gather_fp_const | grep -cE "vmovupd|vmovdqu|movupd|movdqu" || true)
if [ "$n_vst" -lt 1 ]; then
    echo "FAIL: v3_gather_fp_const must vectorize (no vector store found)"
    exit 1
fi

# 3. Zero splats: one vector store; and the zero vector construction
#    (xor-family) present. The scalar fallback would show 3+ movsd/movl.
for fn in v3_zstore_f64 v3_zstore_i32 v3_zstore_i64; do
    n_vst=$(scoped "$fn" | grep -cE "vmovupd|vmovdqu|movupd|movdqu" || true)
    if [ "$n_vst" -lt 1 ]; then
        echo "FAIL: $fn must emit a vector store (root-Splat scheduling)"
        exit 1
    fi
    n_xor=$(scoped "$fn" | grep -cE "vxorpd|vxorps|vpxor|xorpd|xorps|pxor" || true)
    if [ "$n_xor" -lt 1 ]; then
        echo "FAIL: $fn must construct the zero vector (xor-family op missing)"
        exit 1
    fi
done

# 4. All-ones splats: self-compare materialization.
n_cmp=$(scoped v3_ones_i32 | grep -cE "pcmpeqd" || true)
if [ "$n_cmp" -lt 1 ]; then
    echo "FAIL: v3_ones_i32 must use pcmpeqd self-compare"
    exit 1
fi
n_pshuf=$(scoped v3_ones_i32 | grep -cE "pshufd|punpckl|movd " || true)
if [ "$n_pshuf" -ne 0 ]; then
    echo "FAIL: v3_ones_i32 must not stage through movd/shuffles ($n_pshuf)"
    exit 1
fi
n_vcmp=$(scoped v3_ones_i64 | grep -cE "vpcmpeqd" || true)
if [ "$n_vcmp" -lt 1 ]; then
    echo "FAIL: v3_ones_i64 must use the 256-bit vpcmpeqd self-compare"
    exit 1
fi

# 5. FP const broadcast: no per-lane scalar stores.
n_bcast=$(scoped v3_c15_f64 | grep -cE "vbroadcastsd" || true)
if [ "$n_bcast" -lt 1 ]; then
    echo "FAIL: v3_c15_f64 must broadcast the constant (vbroadcastsd)"
    exit 1
fi
n_movsd_mem=$(scoped v3_c15_f64 | grep -cE "movsd[ ]+%xmm[0-9]+, \(" || true)
if [ "$n_movsd_mem" -gt 0 ]; then
    echo "FAIL: v3_c15_f64 stores scalars per lane ($n_movsd_mem)"
    exit 1
fi

# 6. 256-bit byte family: 32-byte copy = one ymm load + one ymm store;
#    in-place add packs at I8x32 (vpaddb), with the VEX memory-operand
#    fold on the added stream when the shape allows.
n_vmov=$(scoped v3_bcopy32 | grep -cE "vmovdqu" || true)
if [ "$n_vmov" -ne 2 ]; then
    echo "FAIL: v3_bcopy32 must be exactly vmovdqu+vmovdqu (got $n_vmov)"
    exit 1
fi
n_paddb=$(scoped v3_badd32 | grep -cE "vpaddb" || true)
if [ "$n_paddb" -lt 1 ]; then
    echo "FAIL: v3_badd32 must pack at I8x32 (vpaddb)"
    exit 1
fi
n_bscalar=$(scoped v3_badd32 | grep -cE "^[[:space:]]+(addb|movb)" || true)
if [ "$n_bscalar" -gt 0 ]; then
    echo "FAIL: v3_badd32 has scalar byte ops left ($n_bscalar)"
    exit 1
fi

# 7. 256-bit halfword family: 16-lane copy = one ymm load + store; the
#    nested promoted tree demotes to vpmullw chains (the mod-2^16
#    equivalence contract).
n_vmov=$(scoped v3_wcopy16 | grep -cE "vmovdqu" || true)
if [ "$n_vmov" -ne 2 ]; then
    echo "FAIL: v3_wcopy16 must be exactly vmovdqu+vmovdqu (got $n_vmov)"
    exit 1
fi
n_pmullw=$(scoped v3_wmul16 | grep -cE "vpmullw" || true)
if [ "$n_pmullw" -lt 2 ]; then
    echo "FAIL: v3_wmul16 must demote BOTH levels to vpmullw (got $n_pmullw)"
    exit 1
fi

# 8. 256-bit lane extracts: the live-out seeds must fire (vector store
#    present) AND service the external uses via vextracti128/pshufd (not
#    by staying scalar).
n_lo_v=$(scoped v3_liveout_i64 | grep -cE "vmovdqu|vmovupd" || true)
if [ "$n_lo_v" -lt 2 ]; then
    echo "FAIL: v3_liveout_i64 must vectorize ($n_lo_v vector ops)"
    exit 1
fi
n_lo_x=$(scoped v3_liveout_i64 | grep -cE "vextracti128|pshufd" || true)
if [ "$n_lo_x" -lt 1 ]; then
    echo "FAIL: v3_liveout_i64 must extract lanes (vextracti128/pshufd)"
    exit 1
fi
n_lof_v=$(scoped v3_liveout_f64 | grep -cE "vmovdqu|vmovupd" || true)
if [ "$n_lof_v" -lt 2 ]; then
    echo "FAIL: v3_liveout_f64 must vectorize ($n_lof_v vector ops)"
    exit 1
fi
# Lane 2 must be serviced from the VECTOR (vextracti128 for a register
# home, or the direct +16 half-slot load for a slot home) — never by
# re-reading the source memory scalarly.
n_lof_reload=$(scoped v3_liveout_f64 | grep -cE "(movsd|movss)[^(]*\(%rdi\)" || true)
if [ "$n_lof_reload" -gt 0 ]; then
    echo "FAIL: v3_liveout_f64 re-reads the source scalarly ($n_lof_reload)"
    exit 1
fi
n_lof_x=$(scoped v3_liveout_f64 | grep -cE "vextracti128|vmovdqu [0-9-]+\(%r(sp|bp)\), %xmm1" || true)
if [ "$n_lof_x" -lt 1 ]; then
    echo "FAIL: v3_liveout_f64 must stage the high half (vextracti128 or half-slot load)"
    exit 1
fi

# 9. Per-lane commutative reordering: the mixed-spelling seed must
#    vectorize (pmulld on the flipped load side + splat), not stay scalar.
n_ms_vec=$(scoped v3_mixed_sides | grep -cE "pmulld|vpmulld" || true)
if [ "$n_ms_vec" -lt 1 ]; then
    echo "FAIL: v3_mixed_sides must vectorize via the per-lane flip ($n_ms_vec pmulld)"
    exit 1
fi
n_ms_imul=$(scoped v3_mixed_sides | grep -cE "imull" || true)
if [ "$n_ms_imul" -gt 0 ]; then
    echo "FAIL: v3_mixed_sides left scalar multiplies ($n_ms_imul imull)"
    exit 1
fi

echo "OK: bb_slp_v3 constant-lane contracts (gather bits, zero/ones splats, FP const broadcast, 256-bit families, live-out extracts, per-lane flip)"
