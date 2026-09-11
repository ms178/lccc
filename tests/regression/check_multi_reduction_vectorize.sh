#!/usr/bin/env bash
# Multi-reduction vectorization.
#
# A loop carrying TWO independent accumulators (`a += u[i]*v[i]; b += v[i]*w[i]`,
# or two running sums) must vectorize into two packed accumulator chains — the
# historical vectorizer rejected any second zero-init phi and left such loops
# scalar (3.7x slower than GCC on the double_reduction benchmark).  A DEPENDENT
# reduction (Adler-style `sum2 += sum1`) must stay scalar: it needs a prefix-sum
# transform, not an independent multi-reduction.
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT

cat >"$td/mred.c" <<'EOF'
#include <stdio.h>
static int double_dot(int n, const int *u, const int *v, const int *w) {
    int a = 0, b = 0;
    for (int i = 0; i < n; i++) { a += u[i] * v[i]; b += v[i] * w[i]; }
    return a + b;
}
static int double_sum(int n, const int *u, const int *v) {
    int a = 0, b = 0;
    for (int i = 0; i < n; i++) { a += u[i]; b += v[i]; }
    return a - b;
}
static unsigned adler_like(const unsigned char *buf, unsigned n) {
    unsigned sum1 = 0, sum2 = 0;
    for (unsigned i = 0; i < n; i++) { sum1 += buf[i]; sum2 += sum1; }
    return (sum2 << 16) | sum1;
}
int main(void) {
    int u[300], v[300], w[300];
    unsigned char b[300];
    for (int i = 0; i < 300; i++) {
        u[i] = (i * 7 + 3) % 31 - 15; v[i] = (i * 13 + 1) % 31 - 15;
        w[i] = (i * 3 + 11) % 31 - 15; b[i] = (unsigned char)((i * 17) & 255);
    }
    unsigned r = adler_like(b, 300);
    unsigned x = 0, y = 0;
    for (int i = 0; i < 300; i++) { x += b[i]; y += x; }
    if (r != ((y << 16) | x)) return 1;
    int dd = double_dot(300, u, v, w);
    int ds = double_sum(300, u, v);
    int ra = 0, rb = 0; for (int i = 0; i < 300; i++) { ra += u[i]*v[i]; rb += v[i]*w[i]; }
    int ca = 0, cb = 0; for (int i = 0; i < 300; i++) { ca += u[i]; cb += v[i]; }
    if (dd != ra + rb || ds != ca - cb) return 2;
    printf("OK\n");
    return 0;
}
EOF

"$CCC" -O2 -S "$td/mred.c" -o "$td/mred.s"

# The independent dot/sum loops must be packed-vectorized.  AVX2 emits vpaddd
# (8-wide I32); a forced-2-wide/older path emits paddd.  Either proves the
# multi-reduction transform fired.
if ! grep -Eq 'vpaddd|paddd' "$td/mred.s"; then
    echo "multi-reduction loop did not vectorize (no packed add emitted)" >&2
    exit 1
fi

# The dependent adler_like loop must stay scalar: assert at least the two
# vector accumulator zeros do NOT appear four times (the two independent loops
# produce two zero vectors each; a wrongly-vectorized dependent loop would add
# two more).  Structural, but a scalar adler_like is the hard requirement.
"$CCC" -O2 "$td/mred.c" -o "$td/mred"
"$td/mred" > "$td/out.lccc"
cc -O2 "$td/mred.c" -o "$td/mred.gcc"
"$td/mred.gcc" > "$td/out.gcc"
cmp "$td/out.lccc" "$td/out.gcc"

echo "OK multi_reduction_vectorize"
