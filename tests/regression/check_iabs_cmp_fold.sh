#!/usr/bin/env bash
# Integer absolute-value comparison folds in simplify.
#
# `abs(a) <cmp> k` against a constant folds the way GCC's and Clang's
# value-range propagation folds it: for k <= 0 the abs disappears into
# `a == 0` / `a != 0` / a constant, and for k > 0 into the single unsigned
# range test on the biased value ((unsigned)(a + k - 1) <= 2k - 2, and
# the <=/>/>= twins). Both families rest on abs(a) being non-negative,
# which holds in C only because negating INT_MIN is undefined behaviour;
# under `-fwrapv` abs(INT_MIN) wraps to INT_MIN and the folded answers
# would invert, so there the folds must stay off and the wrapping
# computation must survive. This gate pins:
#   * the folded x86 shapes (and that the abs instructions are gone),
#   * the kill-switch ownership (CCC_SIMPLIFY_SKIP=cmp restores the
#     negate/select/compare sequence, proving these folds own it),
#   * runtime equality against an overflow-free reference over dense
#     and boundary values, at every predicate and both width families,
#   * the -fwrapv contract at INT_MIN/LONG_MIN (lccc's wrapping answers
#     agree with Clang; GCC's ABS value range ignores the wraparound
#     when the idiom and the comparison share a function, so GCC is
#     deliberately not the oracle for that leg).
set -euo pipefail
CCC=${LCCC_BIN:-${CCC:-target/fastbuild/lccc}}
tmp=${TMPDIR:-/tmp}/lccc-iabs-cmp.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"

cat > "$tmp/shapes.c" <<'C'
#include <stdlib.h>
int a_le0(int x) { return abs(x) <= 0; }
int a_lt5(int x) { return abs(x) < 5; }
int a_delta(int x, int y) { return abs(x - y) <= 3; }
C

"$CCC" -O2 -S "$tmp/shapes.c" -o "$tmp/shapes.s"
CCC_SIMPLIFY_SKIP=cmp "$CCC" -O2 -S "$tmp/shapes.c" -o "$tmp/shapes_ctl.s"

fn_body() { # $1 = asm file, $2 = label; body between the label and ret
    sed -n "/^$2:/,/ret/p" "$1"
}
has() { # $1 = text, $2 = pattern
    grep -q -e "$2" <<<"$1"
}
lacks() { # $1 = text, $2 = pattern
    if grep -q -e "$2" <<<"$1"; then
        echo "unexpected pattern '$2' in:" >&2
        echo "$1" >&2
        return 1
    fi
}

body=$(fn_body "$tmp/shapes.s" a_le0)
has "$body" 'sete' || { echo "a_le0 not folded to a == 0:" >&2; echo "$body" >&2; exit 1; }
lacks "$body" 'negl' || exit 1
lacks "$body" 'cmov' || exit 1

body=$(fn_body "$tmp/shapes.s" a_lt5)
has "$body" 'addl \$4' || { echo "a_lt5 missing bias add:" >&2; echo "$body" >&2; exit 1; }
has "$body" 'setbe' || { echo "a_lt5 missing unsigned range test:" >&2; echo "$body" >&2; exit 1; }
lacks "$body" 'negl' || exit 1
lacks "$body" 'cmov' || exit 1

body=$(fn_body "$tmp/shapes.s" a_delta)
has "$body" 'addl \$3' || { echo "a_delta missing bias add:" >&2; echo "$body" >&2; exit 1; }
has "$body" 'setbe' || { echo "a_delta missing unsigned range test:" >&2; echo "$body" >&2; exit 1; }
lacks "$body" 'negl' || exit 1
lacks "$body" 'cmov' || exit 1

# Kill-switch control: with the cmp family skipped the original
# negate/select/compare sequence returns, so the folds own the delta.
body=$(fn_body "$tmp/shapes_ctl.s" a_le0)
if ! has "$body" 'negl'; then
    echo "control a_le0 does not restore the abs sequence:" >&2
    echo "$body" >&2
    exit 1
fi
if ! has "$body" 'setle'; then
    echo "control a_le0 does not restore the signed compare:" >&2
    echo "$body" >&2
    exit 1
fi

cat > "$tmp/exec.c" <<'C'
#include <stdio.h>
#include <stdlib.h>
#include <limits.h>

/* Wrapping abs via unsigned negation: cannot be the fold's own idiom
   (a BinOp, not a Neg arm), and agrees with abs() except at the min
   value, where it states the -fwrapv contract explicitly. */
static int wabs(int x) { return x < 0 ? (int)(0u - (unsigned)x) : x; }
static long wlabs(long x) { return x < 0 ? (long)(0ul - (unsigned long)x) : x; }

static int preds_i(int ax) {
    return (ax <= 0) + 2 * (ax < 0) + 4 * (ax > 0) + 8 * (ax >= 0)
         + 16 * (ax < 5) + 32 * (ax <= 3) + 64 * (ax > 7) + 128 * (ax >= 9)
         + 256 * (ax == 0) + 512 * (ax != 0) + 1024 * (ax < INT_MAX)
         + 2048 * (ax <= INT_MAX) + 4096 * (ax > INT_MAX) + 8192 * (ax < 1)
         + 16384 * (ax < -3) + 32768 * (ax > -3) + 65536 * (ax == -1);
}
static int preds_l(long ax) {
    return (ax <= 0) + 2 * (ax < 5L) + 4 * (ax <= 3000000000L)
         + 8 * (ax > 4611686018427387903L) + 16 * (ax >= 4611686018427387903L)
         + 32 * (ax < LONG_MAX) + 64 * (ax <= LONG_MAX) + 128 * (ax < -1L);
}

volatile int vi;
volatile long vl;

int main(void) {
    for (vi = -300; vi <= 300; vi++) {
        if (preds_i(abs(vi)) != preds_i(wabs(vi))) {
            printf("BAD int %d\n", vi);
            return 1;
        }
    }
    int ei[] = {INT_MAX, INT_MAX - 1, -INT_MAX, -INT_MAX + 1,
                1000000007, -1000000007};
#if WRAPV_LEG
    /* Under -fwrapv the min value is defined too (abs wraps), and the
       folds must not have fired: abs() and wabs() agree there only if
       the wrapping computation is what actually ran. */
    vi = INT_MIN;
    if (preds_i(abs(vi)) != preds_i(wabs(vi))) {
        printf("BAD int min wrapv\n");
        return 3;
    }
    vl = LONG_MIN;
    if (preds_l(labs(vl)) != preds_l(wlabs(vl))) {
        printf("BAD long min wrapv\n");
        return 4;
    }
#endif
    for (unsigned i = 0; i < sizeof(ei) / sizeof(*ei); i++) {
        vi = ei[i];
        if (preds_i(abs(vi)) != preds_i(wabs(vi))) {
            printf("BAD int edge %d\n", vi);
            return 1;
        }
    }
    long el[] = {0, 1, -1, 4, -4, 5, -5, LONG_MAX, LONG_MAX - 1, -LONG_MAX,
                 3000000000L, -3000000000L,
                 4611686018427387903L, -4611686018427387903L};
    for (unsigned i = 0; i < sizeof(el) / sizeof(*el); i++) {
        vl = el[i];
        if (preds_l(labs(vl)) != preds_l(wlabs(vl))) {
            printf("BAD long %ld\n", vl);
            return 2;
        }
    }
    printf("OK\n");
    return 0;
}
C

for opt in -O2 -O3; do
    "$CCC" $opt "$tmp/exec.c" -o "$tmp/exec_plain"
    out=$("$tmp/exec_plain")
    if [ "$out" != "OK" ]; then
        echo "exec leg $opt: $out" >&2
        exit 1
    fi
    "$CCC" $opt -fwrapv -DWRAPV_LEG=1 "$tmp/exec.c" -o "$tmp/exec_wrap"
    out=$("$tmp/exec_wrap")
    if [ "$out" != "OK" ]; then
        echo "exec leg $opt -fwrapv: $out" >&2
        exit 1
    fi
done

echo "iabs-cmp-fold gate OK"
