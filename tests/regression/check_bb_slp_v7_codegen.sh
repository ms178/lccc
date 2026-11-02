#!/usr/bin/env bash
# BB-SLP v7 gate: the red-team adversarial edges of the v6 feature set.
#
#   1. The battery must compile at -O2 -march=x86-64-v3 and run clean.
#   2. Tri-config differential: the battery's stdout must be identical
#      under (a) SLP on, (b) CCC_NO_BB_SLP=1 (the scalar reference of
#      the same compiler), and (c) gcc -O2 -march=x86-64-v3 (the
#      independent oracle) — every check is a value check, so all three
#      must agree bit-for-bit on every printed line.
#   3. Codegen contracts (scoped asm checks):
#      - the streamed f32x4/f64x2/i16x8 loads fold into their consumers
#        (vaddps/vmulpd/vpaddw with a (%rdi)-class memory operand, no
#        separate vector load instruction in between);
#      - the shift-consumer shape materializes its load (the VEX
#        immediate-shift encodings are register-only — a folded
#        `vpslld $imm, MEM, %xmm` text would be an assembler error or an
#        EVEX miscompile; the body must contain the vmovdqu load);
#      - the FP-Neg f64x4 body is the one-instruction-per-pack sign-mask
#        form (vxorpd with a .LCVEC memory operand);
#      - the i8 select body never degrades to scalar compare chains
#        (no `cmpl`/`set`/scalar `cmp` instructions).
set -euo pipefail
cd "$(dirname "$0")/../.."
ccc=${LCCC_BIN:-${CCC:-target/fastbuild/lccc}}
src=tests/regression/bb_slp_v7.c
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT

march=""
if "$ccc" -march=x86-64-v3 -E -x c /dev/null -o /dev/null 2>/dev/null; then
    march="-march=x86-64-v3"
fi

# ── 1. runtime, SLP on ──────────────────────────────────────────────────
"$ccc" -O2 $march "$src" -o "$td/rt_on"
out_on=$("$td/rt_on")
if [ "$out_on" != "bb_slp_v7: all pass (0 fails)" ]; then
    echo "FAIL: bb_slp_v7 runtime (SLP on): $out_on"
    exit 1
fi

# ── 2. tri-config differential ──────────────────────────────────────────
CCC_NO_BB_SLP=1 "$ccc" -O2 $march "$src" -o "$td/rt_off"
out_off=$("$td/rt_off")
if [ "$out_off" != "$out_on" ]; then
    echo "FAIL: bb_slp_v7 SLP-ON vs SLP-OFF differential"
    echo "  on : $out_on"
    echo "  off: $out_off"
    exit 1
fi
if command -v gcc >/dev/null 2>&1; then
    gcc -O2 $march "$src" -o "$td/rt_gcc" 2>/dev/null
    out_gcc=$("$td/rt_gcc")
    if [ "$out_gcc" != "$out_on" ]; then
        echo "FAIL: bb_slp_v7 lccc vs gcc differential"
        echo "  lccc: $out_on"
        echo "  gcc : $out_gcc"
        exit 1
    fi
fi

if [ -z "$march" ]; then
    echo "OK: bb_slp_v7 runtime + tri-config contracts (no x86-64-v3 here; asm section skipped)"
    exit 0
fi

# ── 3. codegen contracts ────────────────────────────────────────────────
"$ccc" -O2 $march -S "$src" -o "$td/v7.s"

scoped() {
    awk -v f="$1" '
        $0 ~ "^"f":" { infn = 1; next }
        infn && /^[a-zA-Z_][a-zA-Z0-9_]*:/ { exit }
        infn { print }
    ' "$td/v7.s"
}

# 3a. f32x4 fold: vaddps/vmulps with a memory source, no separate load.
n_ops=$(scoped v7_mf_f32 | grep -cE "vmulps|vaddps" || true)
n_loads=$(scoped v7_mf_f32 | grep -cE "[vm]*movups? [^%]" || true)
if [ "$n_ops" -lt 1 ] || [ "$n_loads" -ne 0 ]; then
    echo "FAIL: v7_mf_f32 not folded ($n_ops ops, $n_loads loads)"
    exit 1
fi
# 3b. f64x2 fold.
n_ops=$(scoped v7_mf_f64 | grep -cE "vaddpd|vmulpd" || true)
n_loads=$(scoped v7_mf_f64 | grep -cE "[vm]*movupd? [^%]" || true)
if [ "$n_ops" -lt 1 ] || [ "$n_loads" -ne 0 ]; then
    echo "FAIL: v7_mf_f64 not folded ($n_ops ops, $n_loads loads)"
    exit 1
fi
# 3c. i16x8 fold: vpaddw with a memory source.
n_ops=$(scoped v7_mf_i16 | grep -cE "vpaddw" || true)
n_loads=$(scoped v7_mf_i16 | grep -cE "[vm]*movdqu [^%]" || true)
if [ "$n_ops" -lt 1 ] || [ "$n_loads" -ne 0 ]; then
    echo "FAIL: v7_mf_i16 not folded ($n_ops ops, $n_loads loads)"
    exit 1
fi
# 3d. reversed sub stream: the load may fold in the src2 position
# (vpsubd REG, MEM? no — AT&T `vsub reg, mem, dst` is illegal for the
# src1 slot; the shape must either fold as `vpsubd k, MEM`... it cannot:
# non-commutative src1. The body must be CORRECT; accept both the folded
# k-MEM-src2 form... wait: `q = 12345 - a[i]` puts the LOAD in src1
# (the subtrahend is a[i]): `vpsubd MEM would be src2 = subtrahend —
# exactly the legal position! `vpsubd %k, MEM`? AT&T vpsubd src2, src1,
# dst = src1 - src2 — the subtrahend IS src2 = MEM ✓ foldable. So no
# separate load should remain.
n_loads=$(scoped v7_mf_rev_sub | grep -cE "[vm]*movdqu [^%]" || true)
if [ "$n_loads" -ne 0 ]; then
    echo "FAIL: v7_mf_rev_sub not folded in src2 ($n_loads loads)"
    exit 1
fi
# 3e. the shift consumer: the load must MATERIALISE (register-only VEX
# shift-imm) — a vmovdqu load must be present in the body.
n_loads=$(scoped v7_mf_shift | grep -cE "[vm]*movdqu [^%]" || true)
if [ "$n_loads" -lt 1 ]; then
    echo "FAIL: v7_mf_shift load not materialised ($n_loads loads)"
    exit 1
fi
# 3f. FP-Neg f64x4: the .LCVEC sign-mask memory operand.
n_neg=$(scoped v7_neg_f64x4 | grep -cE "vxorpd.*\.LCVEC" || true)
if [ "$n_neg" -lt 1 ]; then
    echo "FAIL: v7_neg_f64x4 not the sign-mask form"
    exit 1
fi
# 3g. the i8 select: never a scalar compare chain (the packed path uses
# vpcmpgtb/vpcmpeqb + vblendv/pand-family; scalar cmpl/setcc degrades).
n_scalar=$(scoped v7_sel_i8 | grep -cE "    (cmpl|cmpw|set)" || true)
if [ "$n_scalar" -ne 0 ]; then
    echo "FAIL: v7_sel_i8 degraded to scalar compares ($n_scalar)"
    exit 1
fi

echo "OK: bb_slp_v7 red-team contracts (runtime + tri-config + codegen)"
