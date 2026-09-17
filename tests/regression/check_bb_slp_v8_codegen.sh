#!/usr/bin/env bash
# BB-SLP v8 gate: the session-52 feature set's red-team edges.
#
#   1. The battery must compile at -O2 -march=x86-64-v3 and run clean.
#   2. Tri-config differential: the battery's stdout must be identical
#      under (a) SLP on, (b) CCC_NO_BB_SLP=1 (the scalar reference of
#      the same compiler), and (c) gcc -O2 -march=x86-64-v3 (the
#      independent oracle).
#   3. SSE2-baseline parity: -march=x86-64 (no FMA3, no AVX2) must run
#      clean too — the FMA packs and the field-pair packs must fail
#      closed, never miscompile, when the ISA refuses them.
#   4. Codegen contracts (scoped asm checks):
#      - the struct-field pair shape packs (vmulpd on the field pair,
#        one vmovupd pair store — no per-field scalar mul/store pairs);
#      - the pair-update body carries the packed FMA contraction
#        (vfnmadd/vfmadd *pd — NOT a separate vmulpd+vsubpd pair);
#      - the F32 quad contracts (vfnmadd*ps);
#      - the gap-FMA Sub shape contracts at scalar width
#        (vfnmadd231sd with the accumulator register);
#      - the forward-chained body reuses the difference vector (two
#        packed FMA groups, no re-gathered scalar differences).
set -euo pipefail
cd "$(dirname "$0")/../.."
ccc=${LCCC_BIN:-${CCC:-target/fastbuild/lccc}}
src=tests/regression/bb_slp_v8.c
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT

march=""
if "$ccc" -march=x86-64-v3 -E -x c /dev/null -o /dev/null 2>/dev/null; then
    march="-march=x86-64-v3"
fi

# ── 1. runtime, SLP on ──────────────────────────────────────────────────
"$ccc" -O2 $march "$src" -o "$td/rt_on" -lm
out_on=$("$td/rt_on")
if [ "$out_on" != "bb_slp_v8: all pass (0 fails)" ]; then
    echo "FAIL: bb_slp_v8 runtime (SLP on): $out_on"
    exit 1
fi

# ── 2. tri-config differential ──────────────────────────────────────────
CCC_NO_BB_SLP=1 "$ccc" -O2 $march "$src" -o "$td/rt_off" -lm
out_off=$("$td/rt_off")
if [ "$out_off" != "$out_on" ]; then
    echo "FAIL: bb_slp_v8 SLP-ON vs SLP-OFF differential"
    echo "  on : $out_on"
    echo "  off: $out_off"
    exit 1
fi
if command -v gcc >/dev/null 2>&1; then
    gcc -O2 $march "$src" -o "$td/rt_gcc" -lm 2>/dev/null
    out_gcc=$("$td/rt_gcc")
    if [ "$out_gcc" != "$out_on" ]; then
        echo "FAIL: bb_slp_v8 lccc vs gcc differential"
        echo "  lccc: $out_on"
        echo "  gcc : $out_gcc"
        exit 1
    fi
fi

# ── 3. SSE2-baseline parity (fail-closed ISA discipline) ────────────────
"$ccc" -O2 "$src" -o "$td/rt_sse2" -lm
out_sse2=$("$td/rt_sse2")
if [ "$out_sse2" != "$out_on" ]; then
    echo "FAIL: bb_slp_v8 SSE2-baseline differential"
    echo "  v3  : $out_on"
    echo "  sse2: $out_sse2"
    exit 1
fi

if [ -z "$march" ]; then
    echo "OK: bb_slp_v8 runtime + tri-config + baseline contracts (no x86-64-v3 here; asm section skipped)"
    exit 0
fi

# ── 4. codegen contracts ────────────────────────────────────────────────
"$ccc" -O2 $march -S "$src" -o "$td/v8.s"

scoped() {
    awk -v f="$1" '
        $0 ~ "^"f":" { infn = 1; next }
        infn && /^[a-zA-Z_][a-zA-Z0-9_]*:/ { exit }
        infn { print }
    ' "$td/v8.s"
}

# 4a. struct-field pair map: the pair multiply + one pair store; no
# scalar per-field mul+store sequence.
n_vmul=$(scoped v8_field_map | grep -cE "vmulpd" || true)
n_pair_store=$(scoped v8_field_map | grep -cE "movupd .*%xmm" || true)
n_scalar_mul=$(scoped v8_field_map | grep -cE "vmulsd" || true)
if [ "$n_vmul" -lt 1 ] || [ "$n_pair_store" -lt 1 ] || [ "$n_scalar_mul" -ne 0 ]; then
    echo "FAIL: v8_field_map not packed (vmulpd=$n_vmul pair-store=$n_pair_store scalar-mul=$n_scalar_mul)"
    exit 1
fi

# 4b. pair body: the packed FMA contraction fires for every velocity
# update (vfnmadd/vfmadd pd). The ONE remaining vsubpd is the (dx,dy)
# difference vector pack itself — expected, not an uncontracted
# velocity update.
n_fma_pd=$(scoped v8_pair | grep -cE "v[fn]*madd[0-9]*pd" || true)
n_subpd=$(scoped v8_pair | grep -cE "v?subpd" || true)
if [ "$n_fma_pd" -lt 2 ] || [ "$n_subpd" -gt 1 ]; then
    echo "FAIL: v8_pair FMA contraction (fma_pd=$n_fma_pd subpd=$n_subpd)"
    exit 1
fi

# 4c. F32 quad contracts.
n_fma_ps=$(scoped v8_fma_f32 | grep -cE "v[fn]*madd[0-9]*ps" || true)
if [ "$n_fma_ps" -lt 1 ]; then
    echo "FAIL: v8_fma_f32 contraction (fma_ps=$n_fma_ps)"
    exit 1
fi

# 4d. scalar gap-FMA Sub: vfnmadd231sd with a register accumulator.
n_fnm_sd=$(scoped v8_gap1 | grep -cE "vfnmadd[0-9]*sd" || true)
n_vsub=$(scoped v8_gap1 | grep -cE "vsubsd" || true)
if [ "$n_fnm_sd" -lt 1 ] || [ "$n_vsub" -ne 0 ]; then
    echo "FAIL: v8_gap1 gap contraction (fnm_sd=$n_fnm_sd vsubsd=$n_vsub)"
    exit 1
fi

# 4e. forward chains: both velocity pairs consume the shared difference
# vector — two packed FMA groups with no re-computed scalar
# differences (vsubsd) in the body.
n_fma_fwd=$(scoped v8_forward | grep -cE "v[fn]*madd[0-9]*pd" || true)
n_vsubsd=$(scoped v8_forward | grep -cE "vsubsd" || true)
if [ "$n_fma_fwd" -lt 2 ] || [ "$n_vsubsd" -ne 0 ]; then
    echo "FAIL: v8_forward chains (fma_pd=$n_fma_fwd vsubsd=$n_vsubsd)"
    exit 1
fi

echo "OK: bb_slp_v8 contracts (field pairs, field-disjointness, same-source splats, forward chains, packed FMA parity, gap-FMA sub)"
