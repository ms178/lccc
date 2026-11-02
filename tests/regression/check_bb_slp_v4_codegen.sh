#!/usr/bin/env bash
# BB-SLP v4 gate: affine window addressing, rule-(d) escape, extract
# families, and the review follow-up contracts (F1/F2/F5). See
# tests/regression/bb_slp_v4.c for the defect provenance of every shape.
#
#   1. The battery must compile at -O2 -march=x86-64-v3 and run clean
#      (unconditionally — the runtime is ISA-agnostic by design).
#   2. Affine windows must emit the movdqu pair (one vector load, one
#      vector store) — no scalar element copies.
#   3. The extract families must service external uses from the VECTOR
#      (pextrd/vextracti128 staging), never by staying scalar.
#   4. rule_d_disjoint must vectorize; rule_d_samestream must vectorize
#      while keeping the out-of-window write; rule_d_alias must stay
#      SCALAR (the may-alias interleaved write preserved).
#   5. F5: the halfword zero splat is ONE vpxor; the ones splat is ONE
#      vpcmpeqd; the word bitwise map has vpand/vpor/vpxor and no scalar
#      and/or/xor word ops left.
#
# The asm section (2–5) is AVX2-specific and guarded on -march support,
# exactly like check_bb_slp_v3_codegen.sh.
set -euo pipefail
cd "$(dirname "$0")/../.."
ccc=${LCCC_BIN:-${CCC:-target/fastbuild/lccc}}
src=tests/regression/bb_slp_v4.c
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT

march=$("$ccc" -march=x86-64-v3 -E -x c /dev/null -o /dev/null 2>/dev/null \
    && echo "-march=x86-64-v3" || echo "")

# ── 1. runtime ─────────────────────────────────────────────────────────
"$ccc" -O2 $march "$src" -o "$td/rt" -lm
out=$("$td/rt")
if [ "$out" != "bb_slp_v4: all pass (0 fails)" ]; then
    echo "FAIL: bb_slp_v4 runtime output: $out"
    exit 1
fi

if [ -z "$march" ]; then
    echo "OK: bb_slp_v4 runtime contracts (no x86-64-v3 on this target; asm section skipped)"
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

# 2. Affine windows: one vector load + one vector store; zero scalar
#    element copies (movl/movslq with a memory operand).
for fn in v4_affine_window_i32 v4_affine_window_f64 v4_affine_window_f32 \
          v4_affine_shl_i64 v4_affine_window_i16; do
    n_vec=$(scoped "$fn" | grep -cE "movdqu|movups|movupd|vmovdqu|vmovups|vmovupd" || true)
    if [ "$n_vec" -lt 2 ]; then
        echo "FAIL: $fn must emit the vector load+store pair (got $n_vec vector ops)"
        exit 1
    fi
    # Scalar element accesses = scalar movs WITH a memory operand (loads:
    # `movl -4(%rdi,%r8), %eax`; stores: `movl %eax, (%rsi)`; sign/zero
    # extensions from memory: `movslq 12(%rdi), %r11`). The register-to-
    # register index extension (`movslq %edx, %r8` — no memory operand)
    # and the VECTOR loads (`movdqu -4(%rdi,%r8), %xmm2` — the displaced
    # affine-window form) are NOT scalar accesses; the old pattern
    # (`movslq|...|[a-z]+ -?[0-9]+\(%rdi`) matched both and false-failed
    # every affine kernel once the induction work moved the index into a
    # sign-extended register.
    n_scalar=$(scoped "$fn" | grep -cE "mov(s|z)[a-z]* [^,]*\(|mov[bwl] ([^,]*\(|%[a-z0-9]+, *[^,]*\()" || true)
    if [ "$n_scalar" -gt 1 ]; then
        echo "FAIL: $fn has scalar element accesses left ($n_scalar)"
        exit 1
    fi
done

# 3. Extract families: the external use is serviced from the vector.
n_x=$(scoped v4_extract_i32x8 | grep -cE "pextrd|vpextrd" || true)
if [ "$n_x" -lt 1 ]; then
    echo "FAIL: v4_extract_i32x8 must use pextrd (got $n_x)"
    exit 1
fi
n_v=$(scoped v4_extract_i32x8 | grep -cE "vmovdqu|movdqu" || true)
if [ "$n_v" -lt 1 ]; then
    echo "FAIL: v4_extract_i32x8 must vectorize the store (got $n_v)"
    exit 1
fi
n_x=$(scoped v4_extract_f32x8 | grep -cE "vextracti128|vextractf128|vmovdqu [0-9-]+\(%r(sp|bp)\), %xmm1" || true)
if [ "$n_x" -lt 1 ]; then
    echo "FAIL: v4_extract_f32x8 must stage the high half (vextractf128 or half-slot load; got $n_x)"
    exit 1
fi
n_x=$(scoped v4_extract_i16x16 | grep -cE "pextrw" || true)
if [ "$n_x" -lt 1 ]; then
    echo "FAIL: v4_extract_i16x16 must use pextrw (got $n_x)"
    exit 1
fi

# 4. Rule-(d) shapes: disjoint interleaves vectorize; the aliased shape
#    stays scalar (≥ 2 immediate movl stores prove the preserved order).
n_v=$(scoped v4_rule_d_disjoint | grep -cE "movdqu|vmovdqu" || true)
if [ "$n_v" -lt 2 ]; then
    echo "FAIL: v4_rule_d_disjoint must vectorize (got $n_v vector ops)"
    exit 1
fi
n_v=$(scoped v4_rule_d_samestream | grep -cE "movdqu|vmovdqu" || true)
if [ "$n_v" -lt 2 ]; then
    echo "FAIL: v4_rule_d_samestream must vectorize (got $n_v vector ops)"
    exit 1
fi
n_s=$(scoped v4_rule_d_alias | grep -cE "movl +\\\$[0-9]," || true)
if [ "$n_s" -lt 2 ]; then
    echo "FAIL: v4_rule_d_alias must stay scalar ($n_s immediate stores)"
    exit 1
fi

# 5. F5 contracts: one-instruction splats and the word bitwise map.
n_xor=$(scoped v4_zstore_i16 | grep -cE "vpxor" || true)
if [ "$n_xor" -lt 1 ]; then
    echo "FAIL: v4_zstore_i16 must use vpxor (got $n_xor)"
    exit 1
fi
n_staged=$(scoped v4_zstore_i16 | grep -cE "vpbroadcastw|movd +%e" || true)
if [ "$n_staged" -gt 0 ]; then
    echo "FAIL: v4_zstore_i16 staged the zero splat ($n_staged)"
    exit 1
fi
n_vst=$(scoped v4_zstore_i16 | grep -cE "vmovdqu|movdqu" || true)
if [ "$n_vst" -lt 1 ]; then
    echo "FAIL: v4_zstore_i16 must emit a vector store (got $n_vst)"
    exit 1
fi
n_cmp=$(scoped v4_ones_i16 | grep -cE "vpcmpeqd" || true)
if [ "$n_cmp" -lt 1 ]; then
    echo "FAIL: v4_ones_i16 must use vpcmpeqd (got $n_cmp)"
    exit 1
fi
n_bit=$(scoped v4_word_bitwise16 | grep -cE "vpand|vpor|vpxor" || true)
if [ "$n_bit" -lt 2 ]; then
    echo "FAIL: v4_word_bitwise16 must use the word bitwise ops (got $n_bit)"
    exit 1
fi
n_scalar_bit=$(scoped v4_word_bitwise16 | grep -cE "^[[:space:]]+(andw|orw|xorw)" || true)
if [ "$n_scalar_bit" -gt 0 ]; then
    echo "FAIL: v4_word_bitwise16 left scalar word bitwise ops ($n_scalar_bit)"
    exit 1
fi

echo "OK: bb_slp_v4 contracts (affine windows, extracts, rule-d escapes, F1/F2/F5 follow-ups)"
