#!/usr/bin/env bash
# GVN per-object epochs for disjoint restrict parameters (OP-32).
#
# A store through `x[i]` must NOT invalidate the cached load of `y[i]` when x
# and y are `restrict` (provably disjoint): the second `y[i]` load in
#
#     s += y[i]; x[i] = i*2; s += y[i];
#
# must be value-numbered to the first load, eliminating one memory load per
# iteration.  Without per-object epochs every store through an unclassified
# pointer bumped the global generation and reloaded `y[i]`.
#
# Soundness counterpart (also pinned): when the arrays are NOT restrict, the
# store may alias `y`, so the second load MUST survive.
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT

cat >"$td/r.c" <<'EOF'
int f_restrict(int n, int *restrict x, int *restrict y) {
    int s = 0;
    for (int i = 0; i < n; i++) {
        s += y[i];
        x[i] = i * 2;
        s += y[i];
    }
    return s;
}
int f_plain(int n, int *x, int *y) {
    int s = 0;
    for (int i = 0; i < n; i++) {
        s += y[i];
        x[i] = i * 2;
        s += y[i];
    }
    return s;
}
EOF

"$CCC" -O2 -S "$td/r.c" -o "$td/r.s"
restrict_body=$(sed -n '/^f_restrict:/,/^\.size[[:space:]]*f_restrict/p' "$td/r.s")
plain_body=$(sed -n '/^f_plain:/,/^\.size[[:space:]]*f_plain/p' "$td/r.s")

# The restrict version must load y exactly once in the loop body (the second
# y[i] read is CSE'd to the first; the store through x has its own epoch).
r_loads=$(grep -cE 'movslq[[:space:]]*\(' <<<"$restrict_body" || true)
# The plain version must load y twice (the store may alias y, so the second
# read reloads it).
p_loads=$(grep -cE 'movslq[[:space:]]*\(' <<<"$plain_body" || true)

if [ "$r_loads" -gt 1 ]; then
    echo "restrict loop should load y once; got $r_loads movslq loads" >&2
    echo "$restrict_body" >&2
    exit 1
fi
if [ "$p_loads" -lt 2 ]; then
    echo "plain loop must reload y (store to x may alias y); got $p_loads movslq loads" >&2
    echo "$plain_body" >&2
    exit 1
fi

# Runtime correctness against the scalar reference.
cat >"$td/main.c" <<'EOF'
#include <stdio.h>
int f_restrict(int, int *restrict, int *restrict);
int f_plain(int, int *, int *);
int main(void) {
    int x[64], y[64];
    for (int i = 0; i < 64; i++) { x[i] = -1; y[i] = i; }
    int r = f_restrict(64, x, y);
    long e = 0; for (int i = 0; i < 64; i++) e += (long)y[i] * 2;
    if (r != e) { printf("restrict %d vs %ld\n", r, e); return 1; }
    /* f_plain with x==y: the store aliases y, so the second read sees i*2. */
    for (int i = 0; i < 64; i++) x[i] = i;
    r = f_plain(64, x, x);
    e = 0; for (int i = 0; i < 64; i++) e += (long)i + (long)(i * 2);
    if (r != e) { printf("plain-alias %d vs %ld\n", r, e); return 2; }
    return 0;
}
EOF
"$CCC" -O2 "$td/r.c" "$td/main.c" -o "$td/r"
"$td/r"
cc -O2 "$td/r.c" "$td/main.c" -o "$td/r.gcc"
"$td/r.gcc"

echo "OK gvn_disjoint_epochs"
