#!/usr/bin/env bash
# The SSE2 (128-bit, non-AVX2) widening `long += int[i]` reductions must use
# the 4-lane value-shaped composites, not the old 2-lane form:
#
#   sum_all      -> VecWidenAddValueI32x4ToI64x2:
#                   movdqa load + movdqa x2 + psrad $31 + punpckldq +
#                   punpckhdq + paddq x2  (GCC's canonical SSE2 form)
#   sum_positive -> VecWidenMaskedAddValueI32x4ToI64x2: a pcmpgtd + pand
#                   mask in front of the unpack (guarded `if (p[i] > 0)
#                   s += p[i]`, the late-rerun admission that exists for
#                   exactly this shape).  Every surviving lane is > 0, so
#                   the qwords split by ZERO-extension -- `pand` low-dword
#                   mask + `psrlq $32`, no psrad $31 and no punpck (the
#                   accumulator is only ever horizontally summed).
#   sum_above_k  -> constant guard K = 1000: compared against a .rodata
#                   splat (no per-iteration movd/pshufd), zero-extended.
#   sum_above_m5 -> constant guard K = -5: negative lanes survive, so the
#                   sign-unpack (psrad $31) is REQUIRED -- the control that
#                   keeps the zero-extension proof from over-applying.
#
# and the AVX2 path must keep its memory-shaped composite (vpmovsxdq), so
# the ISA selection is pinned on both sides of the gate.
set -euo pipefail
CCC=${CCC:-./target/release/lccc}
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT
cat >"$td/test.c" <<'EOF'
__attribute__((noinline)) long sum_all(const int *p, int n) {
    long s = 0;
    for (int i = 0; i < n; i++)
        s += p[i];
    return s;
}
__attribute__((noinline)) long sum_positive(const int *p, int n) {
    long s = 0;
    for (int i = 0; i < n; i++)
        if (p[i] > 0)
            s += p[i];
    return s;
}
__attribute__((noinline)) long sum_above_k(const int *p, int n) {
    long s = 0;
    for (int i = 0; i < n; i++)
        if (p[i] > 1000)
            s += p[i];
    return s;
}
__attribute__((noinline)) long sum_above_m5(const int *p, int n) {
    long s = 0;
    for (int i = 0; i < n; i++)
        if (p[i] > -5)
            s += p[i];
    return s;
}
EOF

body_of() {
    sed -n "/^$1:/,/^\.size $1/p" "$2"
}

fail() {
    echo "FAIL: $1"
    echo "--- sum_all:"
    body_of sum_all "$td/test.s" || true
    for f in sum_positive sum_above_k sum_above_m5; do
        echo "--- $f:"
        body_of "$f" "$td/test.s" || true
    done
    exit 1
}

# --- SSE2 baseline: 4-lane dual sign-unpack, no 2-lane widen load ---------
"$CCC" -O2 -mno-avx -S "$td/test.c" -o "$td/test.s"
all=$(body_of sum_all "$td/test.s")
pos=$(body_of sum_positive "$td/test.s")

grep -q 'psrad[[:space:]]*\$31' <<<"$all" || fail "sum_all: no psrad \$31 sign extraction (4-lane form missing)"
grep -qE 'punpckldq[[:space:]]+%xmm[0-9]+, *%xmm[0-9]+' <<<"$all" || fail "sum_all: no punpckldq low-half widen"
grep -qE 'punpckhdq[[:space:]]+%xmm[0-9]+, *%xmm[0-9]+' <<<"$all" || fail "sum_all: no punpckhdq high-half widen"
[ "$(grep -cE 'paddq[[:space:]]+%xmm[0-9]+, *%xmm[0-9]+' <<<"$all")" -ge 2 ] || fail "sum_all: expected 2 paddq folds per 4-element step"
grep -q 'pcmpgtd' <<<"$all" && fail "sum_all: unguarded sum must not carry a mask (pcmpgtd present)"

grep -q 'pcmpgtd' <<<"$pos" || fail "sum_positive: no pcmpgtd mask (guarded sum stayed scalar or wrong form)"
grep -qE 'pand[[:space:]]+%xmm[0-9]+, *%xmm[0-9]+' <<<"$pos" || fail "sum_positive: no pand value masking"
grep -q 'psrad' <<<"$pos" && fail "sum_positive: masked lanes are > 0, zero-extend (redundant psrad sign extraction)"
grep -q 'punpck' <<<"$pos" && fail "sum_positive: non-negative lanes split with pand/psrlq, not shuffle-port punpck"
grep -qE 'psrlq[[:space:]]+\$32, *%xmm[0-9]+' <<<"$pos" || fail "sum_positive: no psrlq \$32 high-dword split"
grep -qE 'pand[[:space:]]+\.L[A-Za-z0-9_]+\(%rip\), *%xmm[0-9]+' <<<"$pos" || fail "sum_positive: no low-dword pand split"

abk=$(body_of sum_above_k "$td/test.s")
grep -qE 'pcmpgtd[[:space:]]+\.L[A-Za-z0-9_]+\(%rip\), *%xmm[0-9]+' <<<"$abk" || fail "sum_above_k: constant guard not compared against a .rodata splat"
grep -qE 'movd[[:space:]]+%eax|pshufd[[:space:]]+\$0,' <<<"$abk" && fail "sum_above_k: constant guard re-splatted in the loop"
grep -q 'psrad' <<<"$abk" && fail "sum_above_k: masked lanes are > 1000, zero-extend (redundant psrad)"
grep -qE 'psrlq[[:space:]]+\$32, *%xmm[0-9]+' <<<"$abk" || fail "sum_above_k: no psrlq \$32 high-dword split"

abm=$(body_of sum_above_m5 "$td/test.s")
grep -qE 'pcmpgtd[[:space:]]+\.L[A-Za-z0-9_]+\(%rip\), *%xmm[0-9]+' <<<"$abm" || fail "sum_above_m5: constant guard not compared against a .rodata splat"
grep -q 'psrad[[:space:]]*\$31' <<<"$abm" || fail "sum_above_m5: lanes in (-5, 0) survive the mask, sign extension (psrad \$31) required"
grep -q 'psrlq' <<<"$abm" && fail "sum_above_m5: zero-extension split applied to a guard that admits negative lanes"

# --- AVX2 control: the memory-shaped composite keeps vpmovsxdq ------------
"$CCC" -O2 -march=x86-64-v3 -S "$td/test.c" -o "$td/test3.s"
all3=$(body_of sum_all "$td/test3.s")
grep -q 'vpmovsxdq' <<<"$all3" || fail "AVX2: sum_all lost the vpmovsxdq memory-shaped composite"

echo "PASS: SSE2 4-lane widening sums (plain + masked, zero-extension proof, .rodata guard) and AVX2 composite pinned"
