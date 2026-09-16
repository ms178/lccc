#!/usr/bin/env bash
# BB-SLP v5 gate: packed shifts, rotate decomposition (canonical and raw
# spellings), Not/Neg composites, the Sub(x,1) all-ones idiom, and the FP
# strict min/max fold — plus the adversarial rejections.
#
#   1. The battery must compile at -O2 -march=x86-64-v3 and run clean
#      (unconditionally — the runtime is ISA-agnostic by design).
#   2. Shifts emit their packed immediate forms (vpslld/vpsrld/vpsrad/
#      vpsllw/vpsraw/vpsllq/vpsrlq) with no scalar shift chains left.
#   3. Rotates emit the shl+shr+or triple over ONE vector load (the
#      same-source look-through is the whole point: two loads is the bug
#      it exists to prevent), in both spellings and operand orders.
#   4. x-1 is the vpcmpeqd+vpaddd idiom (no movl $1/movd/pshufd staging);
#      ~x is vpcmpeqd+vpxor; -x is vpxor+psub.
#   5. FP strict min/max fold to vminps/vmaxps/vminpd/vmaxpd; the
#      adversarial shapes (non-strict compare, cmp with an external use,
#      interleaved write between the rotate's two loads) stay SCALAR.
#
# The asm section (2–5) is AVX2-specific and guarded on -march support,
# exactly like check_bb_slp_v3/v4_codegen.sh.
set -euo pipefail
cd "$(dirname "$0")/../.."
ccc=${LCCC_BIN:-${CCC:-target/fastbuild/lccc}}
src=tests/regression/bb_slp_v5.c
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT

march=$("$ccc" -march=x86-64-v3 -E -x c /dev/null -o /dev/null 2>/dev/null \
    && echo "-march=x86-64-v3" || echo "")

# ── 1. runtime (also the gcc-differential via the suite's A/B) ─────────
"$ccc" -O2 $march "$src" -o "$td/rt" -lm
out=$("$td/rt")
if [ "$out" != "bb_slp_v5: all pass (0 fails)" ]; then
    echo "FAIL: bb_slp_v5 runtime output: $out"
    exit 1
fi

if [ -z "$march" ]; then
    echo "OK: bb_slp_v5 runtime contracts (no x86-64-v3 on this target; asm section skipped)"
    exit 0
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

fail=0
expect() {  # expect <desc> <func> <grep-args...>
    local desc=$1 fn=$2
    shift 2
    if ! scoped "$fn" | grep -qE "$@"; then
        echo "FAIL: $desc ($fn: missing '$*')"
        fail=1
    fi
}
refuse() {  # refuse <desc> <func> <grep-args...>
    local desc=$1 fn=$2
    shift 2
    if scoped "$fn" | grep -qE "$@"; then
        echo "FAIL: $desc ($fn: unexpected '$*')"
        fail=1
    fi
}

# 2. Packed immediate shifts.
expect "shl i32 packed"      v5_shl_i32   'vpslld \$3'
expect "sar i32 packed"      v5_sar_i32   'vpsrad \$3'
expect "shr u32 packed"      v5_shr_u32   'vpsrld \$3'
expect "shl i16 packed"      v5_shl_i16   'vpsllw \$5'
expect "sar i16 packed"      v5_sar_i16   'vpsraw \$5'
expect "shl i64 packed"      v5_shl_i64   'vpsllq \$9'
expect "shr u64 packed"      v5_shr_u64   'vpsrlq \$9'
expect "shl i32x8 packed"    v5_shl_i32x8 'vpslld \$7'
refuse "shl i32 no scalar chain" v5_shl_i32   'shll \$3'
refuse "sar i32 no scalar chain" v5_sar_i32   'sarl \$3'
refuse "shl i64 no scalar chain" v5_shl_i64   'shlq \$9'

# 3. Rotate triples: exactly ONE vector load feeding the shl+shr+or
#    (the look-through's shared-operand contract).
rot_one_load() {  # rot_one_load <func> <shl-imm> <shr-imm>
    local fn=$1 shl_imm=$2 shr_imm=$3
    local n_vec n_shl
    # The shared-operand contract: exactly ONE vector LOAD from the input
    # pointer (a store's destination-first operand form distinguishes it;
    # stack staging of unhomed intermediates is a separate homing-quality
    # follow-up, not a shared-operand defect).
    n_vec=$(scoped "$fn" | grep -cE '(vmovdqu|vmovups|movdqu|movups) \((%rdi|%rsi|%rdx|%rcx|%r8|%r9)[,)]' || true)
    n_shl=$(scoped "$fn" | grep -cE "vpslld [$]${shl_imm}|vpsllq [$]${shl_imm}" || true)
    if [ "$n_vec" -ne 1 ] || [ "$n_shl" -lt 1 ]; then
        echo "FAIL: rotate shared-operand ($fn: $n_vec input-pointer loads (want 1), $n_shl shl)"
        fail=1
    fi
    if ! scoped "$fn" | grep -qE "vpsrld [$]${shr_imm}|vpsrlq [$]${shr_imm}"; then
        echo "FAIL: rotate complementary shr ($fn)"
        fail=1
    fi
    if ! scoped "$fn" | grep -qE 'vpor|por '; then
        echo "FAIL: rotate or ($fn)"
        fail=1
    fi
}
rot_one_load v5_rotl_u32 7 25
rot_one_load v5_rotr_u32 13 19
rot_one_load v5_rotl_u64 13 51
rot_one_load v5_rotl_swapped 7 25
refuse "rotate no scalar rol/ror" v5_rotl_u32 '\brol[a-z]* \$7|\bror[a-z]* '

# 4. Idiom composites: all-ones splat is ONE self-compare.
expect "sub1 i32 vpcmpeqd"  v5_sub1_i32  'pcmpeqd %xmm|vpcmpeqd %xmm'
expect "sub1 i32 vpaddd"    v5_sub1_i32  'paddd|vpaddd'
refuse "sub1 i32 no const staging" v5_sub1_i32 'movl \$1,'
expect "sub1 u64 vpcmpeqd"  v5_sub1_u64  'pcmpeqd %xmm|vpcmpeqd %xmm'
expect "sub1 u64 vpaddq"    v5_sub1_u64  'paddq|vpaddq'
expect "not i32 vpxor"      v5_not_i32   'pxor|vpxor'
expect "not i32 vpcmpeqd"   v5_not_i32   'pcmpeqd %xmm|vpcmpeqd %xmm'
refuse "not i32 no scalar not" v5_not_i32 '\bnotl\b'
expect "neg i32 psubd"      v5_neg_i32   'subd %|psubd'
expect "neg i64 psubq"      v5_neg_i64   'subq %xmm|psubq'
refuse "neg i32 no scalar neg" v5_neg_i32 '\bnegl\b'

# 5. FP strict min/max folds; adversarial shapes stay scalar.
expect "min f32 vminps"     v5_min_f32   'minps|vminps'
expect "max f32 vmaxps"     v5_max_f32   'maxps|vmaxps'
refuse "min f32 no scalar cmov" v5_min_f32 'cmov'
# The mirrored spellings fold to the MIRRORED intrinsic: `l < r ? r : l`
# is Max(r, l) — vmaxpd with the false arm (l) in the src2 slot.
expect "min f64 swapped vmaxpd" v5_min_f64_swapped 'maxpd|vmaxpd'
expect "max f64 swapped vminpd" v5_max_f64_swapped 'minpd|vminpd'
expect "clamp f32x8 vmaxps" v5_clamp_f32x8 'maxps|vmaxps'
expect "clamp f32x8 vminps" v5_clamp_f32x8 'minps|vminps'
# The ChaCha-shaped words: packed adds + rotate triples + xors.
expect "chacha packed add"  chacha_words 'paddd|vpaddd'
expect "chacha packed rot"  chacha_words 'vpslld \$7'
expect "chacha packed xor"  chacha_words 'pxor|vpxor'
# Adversarial rejections.
refuse "varying shift stays scalar" v5_varying_shift 'vpslld'
refuse "non-strict min stays scalar" v5_nonstrict_min 'minps|vminps'
refuse "cmp external use stays scalar" v5_cmp_external_use 'minps|vminps'
refuse "interleaved write stays scalar" v5_rot_interleaved_write 'vpslld'

if [ "$fail" -ne 0 ]; then
    exit 1
fi
echo "OK: bb_slp_v5 contracts (packed shifts, rotate triples with one load, all-ones idioms, FP min/max folds, adversarial rejections)"
