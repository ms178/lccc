#!/usr/bin/env bash
# The SSE2 (128-bit, non-AVX2) widening `long += int[i]` reductions must use
# the 4-lane value-shaped composites, not the old 2-lane form:
#
#   sum_all      -> VecWidenAddValueI32x4ToI64x2:
#                   movdqa load + movdqa x2 + psrad $31 + punpckldq +
#                   punpckhdq + paddq x2  (GCC's canonical SSE2 form)
#   sum_positive -> VecWidenMaskedAddValueI32x4ToI64x2: the same dual
#                   sign-unpack with a pcmpgtd + pand mask in front
#                   (guarded `if (p[i] > 0) s += p[i]`, the late-rerun
#                   admission that exists for exactly this shape)
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
EOF

body_of() {
    sed -n "/^$1:/,/^\.size $1/p" "$2"
}

fail() {
    echo "FAIL: $1"
    echo "--- sum_all:"
    body_of sum_all "$td/test.s" || true
    echo "--- sum_positive:"
    body_of sum_positive "$td/test.s" || true
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
grep -q 'psrad[[:space:]]*\$31' <<<"$pos" || fail "sum_positive: no psrad \$31 sign extraction"
grep -qE 'punpckhdq[[:space:]]+%xmm[0-9]+, *%xmm[0-9]+' <<<"$pos" || fail "sum_positive: no punpckhdq high-half widen"

# --- AVX2 control: the memory-shaped composite keeps vpmovsxdq ------------
"$CCC" -O2 -march=x86-64-v3 -S "$td/test.c" -o "$td/test3.s"
all3=$(body_of sum_all "$td/test3.s")
grep -q 'vpmovsxdq' <<<"$all3" || fail "AVX2: sum_all lost the vpmovsxdq memory-shaped composite"

echo "PASS: SSE2 4-lane widening sums (plain + masked) and AVX2 composite pinned"
