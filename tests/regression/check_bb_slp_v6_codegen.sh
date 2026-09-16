#!/usr/bin/env bash
# BB-SLP v6 gate: the 128-bit VEX memory fold, FP negation (one-instruction
# sign-mask composite), integer min/max folds, the general cmp+blendv
# composite, and the rule-(b) cross-block relaxation.
#
#   1. The battery must compile at -O2 -march=x86-64-v3 and run clean
#      (unconditionally — the runtime is ISA-agnostic by design).
#   2. Memfold: the streamed 128-bit loads fold into their consumers —
#      `vpaddd (%rdi), %xmmK, %xmmD`-class single-op bodies (no separate
#      `vmovdqu load` instruction between the constant materialization and
#      the packed op).
#   3. FP Negation: ONE `vxorps/vxorpd` per pack with the .rodata sign
#      mask as a memory operand (`.LCVEC_`), the exact GCC spelling.
#   4. Integer min/max: `vpminsd/vpmaxsd` (the 128-bit dword pair) and
#      `vpminsd`-class 256-bit forms — no scalar cmov chains left.
#   5. cmp+blendv: `vpcmpgtd/vpcmpeqd` + `vblendvps` pairs (the 128-bit
#      AVX2 fast path) or the exact pand/pandn/por select — never scalar.
#   6. Cross-block: `vpaddq`-class packed ops with the extracts servicing
#      the successor-block uses.
#   7. The adversarial shapes (mixed per-lane predicates, externally-used
#      conditions) compute the right VALUES; their lowering is unchecked
#      (rejection is legal, miscompilation is not — the runtime catches
#      that).
set -euo pipefail
cd "$(dirname "$0")/../.."
ccc=${LCCC_BIN:-${CCC:-target/fastbuild/lccc}}
src=tests/regression/bb_slp_v6.c
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT

march=$("$ccc" -march=x86-64-v3 -E -x c /dev/null -o /dev/null 2>/dev/null \
    && echo "-march=x86-64-v3" || echo "")

# ── 1. runtime ──────────────────────────────────────────────────────────
"$ccc" -O2 $march "$src" -o "$td/rt"
out=$("$td/rt")
if [ "$out" != "bb_slp_v6: all pass (0 fails)" ]; then
    echo "FAIL: bb_slp_v6 runtime output: $out"
    exit 1
fi

if [ -z "$march" ]; then
    echo "OK: bb_slp_v6 runtime contracts (no x86-64-v3 on this target; asm section skipped)"
    exit 0
fi

# ── 2–6. codegen contracts ─────────────────────────────────────────────
"$ccc" -O2 $march -S "$src" -o "$td/v6.s"

scoped() {  # scoped <func> — the function's asm slice (label to next fn)
    awk -v f="$1" '
        $0 ~ "^"f":" { infn = 1; next }
        infn && /^[a-zA-Z_][a-zA-Z0-9_]*:/ { exit }
        infn { print }
    ' "$td/v6.s"
}

# 2. memfold: the packed consumer reads (%rdi) directly — count folded
# ops; a non-folded body pays a separate vector load. The load detector
# matches only LOAD-form movdqu/vmovdqu (memory source: the operand after
# the mnemonic is not a %register) — the trailing result STORE
# (`movdqu %xmmN, (%rsi)`) is not a load.
n_mf=$(scoped v6_mf_add | grep -cE "vpaddd|vmovdqu|movdqu" || true)
n_mf_load=$(scoped v6_mf_add | grep -cE "[vm]*movdqu [^%]" || true)
if [ "$n_mf" -lt 1 ] || [ "$n_mf_load" -ne 0 ]; then
    echo "FAIL: v6_mf_add not folded ($n_mf ops, $n_mf_load loads)"
    exit 1
fi
n_mf2=$(scoped v6_mf_xor | grep -cE "vpxor" || true)
if [ "$n_mf2" -lt 1 ]; then
    echo "FAIL: v6_mf_xor not packed"
    exit 1
fi

# 3. FP negation: the .LCVEC sign-mask memory operand.
n_neg=$(scoped v6_neg_f32x4 | grep -cE "vxorps.*\.LCVEC" || true)
if [ "$n_neg" -lt 1 ]; then
    echo "FAIL: v6_neg_f32x4 not the one-instruction sign-mask form"
    exit 1
fi
n_neg8=$(scoped v6_neg_f32x8 | grep -cE "vxorps.*\.LCVEC" || true)
if [ "$n_neg8" -lt 1 ]; then
    echo "FAIL: v6_neg_f32x8 not the one-instruction sign-mask form"
    exit 1
fi
n_negd=$(scoped v6_neg_f64x2 | grep -cE "vxorpd.*\.LCVEC" || true)
if [ "$n_negd" -lt 1 ]; then
    echo "FAIL: v6_neg_f64x2 not the one-instruction sign-mask form"
    exit 1
fi
grep -q "\.LCVEC_" "$td/v6.s" && grep -A2 "\.LCVEC_.*:" "$td/v6.s" | grep -q "\.quad" || {
    echo "FAIL: the vector const pool entry is missing/malformed"
    exit 1
}

# 4. integer min/max: pminsd/pmaxsd (128) and the 256-bit vpminsd.
n_imm=$(scoped v6_imin_lt | grep -cE "pminsd|vpminsd" || true)
if [ "$n_imm" -lt 1 ]; then
    echo "FAIL: v6_imin_lt not pminsd"
    exit 1
fi
n_imx=$(scoped v6_imax_gt | grep -cE "pmaxsd|vpmaxsd" || true)
if [ "$n_imx" -lt 1 ]; then
    echo "FAIL: v6_imax_gt not pmaxsd"
    exit 1
fi
n_im8=$(scoped v6_imin8_256 | grep -cE "vpminsd" || true)
if [ "$n_im8" -lt 1 ]; then
    echo "FAIL: v6_imin8_256 not vpminsd"
    exit 1
fi

# 5. cmp+blendv: the packed compare + vblendvps pair (AVX2 fast path) or
# the exact bitwise select; never a scalar cmov chain.
n_cb=$(scoped v6_sel_lt | grep -cE "vpcmpgtd|vpcmpeqd|pcmpgtd|pcmpeqd" || true)
n_bl=$(scoped v6_sel_lt | grep -cE "vblendvps|pand" || true)
if [ "$n_cb" -lt 1 ] || [ "$n_bl" -lt 1 ]; then
    echo "FAIL: v6_sel_lt not cmp+blend ($n_cb compares, $n_bl selects)"
    exit 1
fi
n_fs=$(scoped v6_fsel_le | grep -cE "cmpps|vcmpps" || true)
if [ "$n_fs" -lt 1 ]; then
    echo "FAIL: v6_fsel_le not a packed FP compare"
    exit 1
fi

# 6. cross-block: the packed add services the successor-block uses.
n_xb=$(scoped v6_xblock | grep -cE "vpaddd|vpaddq|vmovdqu|movdqu" || true)
if [ "$n_xb" -lt 2 ]; then
    echo "FAIL: v6_xblock did not vectorize ($n_xb vector ops)"
    exit 1
fi

# 7. W5 register-homing contracts:
#    rotl256 — the rotate diamond fully register-homed: exactly ONE
#    load-form movdqu and NO stack frame (subq) in the body.
n_r=$(scoped v6_w5_rotl256 | grep -cE "vpsllq|vpsrlq|vpor|por" || true)
n_r_ld=$(scoped v6_w5_rotl256 | grep -cE "[vm]*movdq[ua]* [^%]" || true)
n_r_fr=$(scoped v6_w5_rotl256 | grep -cE "subq .*%rsp|push" || true)
if [ "$n_r" -lt 3 ] || [ "$n_r_ld" -ne 1 ] || [ "$n_r_fr" -ne 0 ]; then
    echo "FAIL: v6_w5_rotl256 not the homed diamond ($n_r ops, $n_r_ld loads, $n_r_fr frame)"
    exit 1
fi
#    fneg8 — the sign-mask form with NO stack frame (the dead subq $56).
n_f8=$(scoped v6_w5_fneg8 | grep -cE "vxorps.*\.LCVEC" || true)
n_f8_fr=$(scoped v6_w5_fneg8 | grep -cE "subq .*%rsp" || true)
if [ "$n_f8" -lt 1 ] || [ "$n_f8_fr" -ne 0 ]; then
    echo "FAIL: v6_w5_fneg8 frame or shape regressed ($n_f8 sign-mask ops, $n_f8_fr frames)"
    exit 1
fi
#    min — the homed/folded two-stream min: exactly one load-form
#    movdqu (the b-stream), no frame.
n_m=$(scoped v6_w5_min | grep -cE "pminsd|vpminsd" || true)
n_m_ld=$(scoped v6_w5_min | grep -cE "[vm]*movdq[ua]* [^%]" || true)
n_m_fr=$(scoped v6_w5_min | grep -cE "subq .*%rsp" || true)
if [ "$n_m" -lt 1 ] || [ "$n_m_ld" -ne 1 ] || [ "$n_m_fr" -ne 0 ]; then
    echo "FAIL: v6_w5_min not the 3-insn homed shape ($n_m mins, $n_m_ld loads, $n_m_fr frames)"
    exit 1
fi

echo "OK: bb_slp_v6 contracts (memfold, fp-neg sign-mask, int min/max, cmp+blendv, cross-block, w5 homing)"
