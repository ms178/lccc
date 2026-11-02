#!/usr/bin/env bash
# ============================================================================
# check_bool_pair_prune.sh — pin the And-split post-RA prune's atomicity.
# (Kept name: the gate predates the De Morgan split that superseded the
# bool-pair fusion; the contract it pins is unchanged.)
#
# The De Morgan split skips both legs' setcc/movzbl (plus the And/Or) and
# re-emits the compares at the branch with a short-circuit jump between
# them. When a leg operand has no readable home at the branch (post-RA
# prune in prologue.rs), the split must be dropped ATOMICALLY: the And/Or
# entry AND both legs' skip memberships. A prune that forgets the leg
# dests leaves both setccs skipped while the And/Or falls back to a real
# `andl`/`orl` over never-written boolean homes (half-pruned miscompile
# — every leg reads stale garbage).
#
# Pruning is deterministic under CCC_NO_FOLDED_INDEX_LIVENESS=1 (register
# homes lose their replay guarantee there), so each input below runs
# differentially (lccc vs gcc) at default flags AND with the prune-forcing
# env var. Pre-fix binaries fail the nofold leg (all-true branches).
# ============================================================================
set -euo pipefail

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
CCC=${CCC:-$here/../../target/fastbuild/lccc}
[[ -x $CCC ]] || { echo "check_bool_pair_prune: lccc not found at $CCC" >&2; exit 1; }

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

fail=0

# BinOp-fed legs: each leg compares an immediately-produced value, so under
# the nofold switch the leg operands are unreadable at the branch and the
# pair must prune back to two materialized booleans + andl. The global
# accumulator is load-bearing: its load/add/store traffic shapes the
# register homes so the prune actually fires (a local accumulator keeps
# everything homed and the test would pass vacuously — verified: pre-fix
# binaries print hits=1024 here instead of 256).
cat > "$work/prune_binop_legs.c" <<'EOF'
#include <stdio.h>
int hits;
void f(unsigned *a, unsigned n) {
    for (unsigned i = 0; i < n; i++) {
        unsigned x = a[i];
        if (((x & 1u) != 0u) && ((x & 2u) != 0u)) hits++;
    }
}
int main(void) {
    static unsigned a[1024];
    for (unsigned i = 0; i < 1024; i++) a[i] = (i * 2654435761u) >> 8;
    f(a, 1024);
    printf("hits=%d\n", hits);
    return hits != 256;
}
EOF

# Arithmetic-fed legs with mixed widths: same prune contract, different
# operand home shapes (truncation + extension traffic near the legs).
cat > "$work/prune_arith_legs.c" <<'EOF'
#include <stdio.h>
int hits;
void f(unsigned *a, unsigned char *b, unsigned n) {
    for (unsigned i = 0; i < n; i++) {
        unsigned v = a[i] + b[i];
        if ((int)v > 1000 && (v & 255u) < 200u) hits++;
    }
}
int main(void) {
    static unsigned a[512];
    static unsigned char b[512];
    for (unsigned i = 0; i < 512; i++) { a[i] = i * 7u + 3u; b[i] = (unsigned char)(i * 13u); }
    f(a, b, 512);
    printf("hits=%d\n", hits);
    return 0;
}
EOF

for t in prune_binop_legs prune_arith_legs; do
    if ! gcc -O2 "$work/$t.c" -o "$work/$t.gcc" 2>"$work/cc.err"; then
        echo "FAIL($t): gcc reference compile"; head -5 "$work/cc.err"; exit 1
    fi
    if ! "$CCC" -O2 "$work/$t.c" -o "$work/$t.lccc" 2>"$work/cl.err"; then
        echo "FAIL($t): lccc compile"; head -5 "$work/cl.err"; exit 1
    fi
    if ! CCC_NO_FOLDED_INDEX_LIVENESS=1 "$CCC" -O2 "$work/$t.c" -o "$work/$t.nofold" 2>"$work/cn.err"; then
        echo "FAIL($t): lccc nofold compile"; head -5 "$work/cn.err"; exit 1
    fi
    rc_gcc=0; "$work/$t.gcc" > "$work/$t.out.gcc" || rc_gcc=$?
    rc_lccc=0; "$work/$t.lccc" > "$work/$t.out.lccc" || rc_lccc=$?
    rc_nofold=0; "$work/$t.nofold" > "$work/$t.out.nofold" || rc_nofold=$?
    if ! cmp -s "$work/$t.out.gcc" "$work/$t.out.lccc" || [ $rc_gcc -ne $rc_lccc ]; then
        echo "FAIL($t): default differential mismatch (lccc vs gcc)"
        diff "$work/$t.out.gcc" "$work/$t.out.lccc" | head -5 || true
        fail=1
    else
        echo "ok($t/default): outputs identical"
    fi
    if ! cmp -s "$work/$t.out.gcc" "$work/$t.out.nofold" || [ $rc_gcc -ne $rc_nofold ]; then
        echo "FAIL($t): nofold differential mismatch (prune path miscompile?)"
        diff "$work/$t.out.gcc" "$work/$t.out.nofold" | head -5 || true
        fail=1
    else
        echo "ok($t/nofold): prune-path outputs identical"
    fi
done

# Control: with the fusion disabled the nofold path must agree too (guards
# against a broken test input rather than a broken fusion).
if ! CCC_NO_FOLDED_INDEX_LIVENESS=1 CCC_NO_DEMORGAN=1 "$CCC" -O2 "$work/prune_binop_legs.c" -o "$work/ctl" 2>"$work/ctl.err"; then
    echo "FAIL(control): lccc nofold+nodm compile"; head -5 "$work/ctl.err"; exit 1
fi
rc_ctl=0; "$work/ctl" > "$work/ctl.out" || rc_ctl=$?
if ! cmp -s "$work/prune_binop_legs.out.gcc" "$work/ctl.out"; then
    echo "FAIL(control): fusion-disabled output differs from gcc (bad input?)"
    fail=1
else
    echo "ok(control): fusion-disabled path agrees with gcc"
fi

exit $fail
