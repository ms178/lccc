#!/usr/bin/env bash
# check_multi_entry_loop.sh — a loop header with MORE THAN ONE out-of-loop
# predecessor must never be treated as having a single preheader.
#
# WHY THIS GATE EXISTS
# --------------------
# `vectorize.rs::find_loop_preheader` picks the block the invariant loop bound
# is hoisted into.  It used to recognise only `Branch` and `CondBranch`
# terminators and answered `false` for everything else, so a header entered
# both from a `Switch` case and from a plain `Branch` yielded the `Branch`
# block as *the* preheader -- a block that does not dominate the header.  The
# hoisted bound's definition would then not dominate its use on the switch
# path: a use-before-def, i.e. a miscompile rather than a missed optimisation.
#
# This was found by the audit of PR #686 (finding F2) and is fixed in both
# trees; the audit's Task 4(c) asked for exactly this gate and neither PR
# shipped it.
#
# WHAT IT CANNOT PROVE, STATED PLAINLY
# ------------------------------------
# No shape tried so far makes the vectorizer's reduction detection fire on a
# multi-entry loop, so these cases do not currently reach the buggy branch --
# `LCCC_DEBUG_VECTORIZE=1` reports zero dynamic-limit/preheader activity on
# them.  They are therefore a *behavioural smoke test and a compile contract*,
# not a reproduction of the miscompile.  The fix's correctness rests on the
# dominance proof recorded at `find_loop_preheader`, not on this gate.  What
# the gate does guarantee is that a future change to terminator handling cannot
# silently start producing wrong answers on switch- or computed-goto-entered
# loops: the expected values are pinned against the C semantics, not against
# LCCC's current output.
#
# Every case needs only $CCC and `cc`; nothing here is LCCC-specific.
set -euo pipefail

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "$here/../.." && pwd)
CCC=${CCC:-$root/target/fastbuild/lccc}
CC=${CC:-cc}
[[ -x $CCC ]] || { echo "check_multi_entry_loop: lccc not found at $CCC" >&2; exit 1; }
command -v "$CC" >/dev/null 2>&1 || { echo "check_multi_entry_loop: no C compiler ($CC)" >&2; exit 1; }

work=$(mktemp -d "${TMPDIR:-/tmp}/lccc-multientry.XXXXXX")
trap 'rm -rf "$work"' EXIT
fail=0

# Case 1: header entered from two switch cases (one of which pre-seeds the
# accumulator) and refused on the default arm.  Runtime trip count.
cat > "$work/switch_entry.c" <<'EOF'
#include <stdio.h>
#include <stdlib.h>
long sw_red(const int *a, int n, int mode) {
    long t = 0;
    int i = 0;
    switch (mode) {
    case 0: goto loop;
    case 1: t = 5; goto loop;
    default: return -1;
    }
loop:
    for (; i < n; i++) t += a[i & 63] * 3;
    return t;
}
int main(int argc, char **argv) {
    int a[64];
    for (int i = 0; i < 64; i++) a[i] = i - 30;
    int n = argc > 1 ? atoi(argv[1]) : 64;
    printf("%ld %ld %ld\n", sw_red(a, n, 0), sw_red(a, n, 1), sw_red(a, n, 9));
    return 0;
}
EOF

# Case 2: header entered via a computed goto (IndirectBranch).
cat > "$work/computed_goto.c" <<'EOF'
#include <stdio.h>
long cg(const int *a, int n, int start) {
    static void *labels[2] = { &&L0, &&L1 };
    long t = 0;
    int i = 0;
    goto *labels[start & 1];
L0:
    for (; i < n; i++) t += a[i & 15];
    return t;
L1:
    t = 7;
    for (; i < n; i++) t += a[i & 15] * 2;
    return t;
}
int main(void) {
    int a[16];
    for (int i = 0; i < 16; i++) a[i] = i + 1;
    printf("%ld %ld\n", cg(a, 16, 0), cg(a, 16, 1));
    return 0;
}
EOF

check() { # check <name> <src> [args...]
    local name=$1 src=$2; shift 2
    local refbin lbin ref got
    refbin=$work/$name.ref; lbin=$work/$name.lccc
    if ! "$CC" -O2 "$src" -o "$refbin" 2>"$work/$name.cc.err"; then
        echo "  SKIP $name: the host compiler rejects it (gnu extensions?)" >&2
        head -2 "$work/$name.cc.err" >&2
        return 0
    fi
    ref=$("$refbin" "$@" 2>&1)
    for opt in -O0 -O1 -O2 -O3 -Os; do
        if ! "$CCC" $opt -march=x86-64-v3 "$src" -o "$lbin" 2>"$work/$name.lccc.err"; then
            echo "  FAIL $name: lccc $opt failed to compile" >&2
            head -3 "$work/$name.lccc.err" >&2
            fail=1; continue
        fi
        got=$("$lbin" "$@" 2>&1) || { echo "  FAIL $name: lccc $opt crashed at runtime" >&2; fail=1; continue; }
        if [ "$got" != "$ref" ]; then
            echo "  FAIL $name at $opt: expected '$ref', got '$got'" >&2
            fail=1
        fi
    done
    echo "  ok   $name (5 opt levels agree with $CC: '$ref')"
}

check switch_entry  "$work/switch_entry.c" 137
check computed_goto "$work/computed_goto.c"

if [ "$fail" -ne 0 ]; then exit 1; fi
echo "check_multi_entry_loop: PASS"
