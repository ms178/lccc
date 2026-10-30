#!/usr/bin/env bash
# Prove the post-phi dead-IV countdown on the original switch/LUT reproducer.
# A positive constant trip count changes 0 -> N / +1 / (iv < N) into
# N -> 0 / -1 / (remaining != 0), making the entry guard redundant.  The
# kill-switch arm is both the structural control and a runtime oracle.
set -euo pipefail

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "$here/../.." && pwd)
CCC=${CCC:-$root/target/fastbuild/lccc}
GCC=${GCC:-gcc-16}
[[ -x $CCC ]] || { echo "check_loop_downcount: no lccc at $CCC" >&2; exit 1; }
command -v "$GCC" >/dev/null 2>&1 || { echo "check_loop_downcount: no GCC oracle" >&2; exit 1; }
CCC=$(readlink -f "$CCC")
GCC=$(readlink -f "$(command -v "$GCC")")
work=$(mktemp -d "${TMPDIR:-/tmp}/lccc-downcount.XXXXXX")
trap 'rm -rf "$work"' EXIT

src=$root/tests/benchmark/programs/switch_lut_bool_bench.c
body() { sed -n '/^main:/,/^\.size[[:space:]]*main/p' "$1"; }

# Keep the structural check at a small, easily recognisable bound.  There must
# be no residual dynamic compare against N in the optimized main, while the
# independent control must retain both its entry and latch comparisons.
env -u CCC_NO_LOOP_DOWNCOUNT "$CCC" -O2 -DN=250000 -S "$src" -o "$work/on.s"
CCC_NO_LOOP_DOWNCOUNT=1 "$CCC" -O2 -DN=250000 -S "$src" -o "$work/off.s"
on=$(body "$work/on.s")
off=$(body "$work/off.s")
grep -qE 'movl[[:space:]]+\$250000,' <<<"$on" || {
    echo "downcount did not initialise the remaining-trip counter" >&2; echo "$on" >&2; exit 1;
}
grep -qE 'subl[[:space:]]+\$1,' <<<"$on" || {
    echo "downcount did not emit the decrement" >&2; echo "$on" >&2; exit 1;
}
grep -qE '^jne[[:space:]]' <<<"$on" || {
    echo "downcount did not reuse decrement flags for the latch" >&2; echo "$on" >&2; exit 1;
}
if grep -qE 'cmpl[[:space:]]+\$250000,' <<<"$on"; then
    echo "entry/latch compare against the old bound survived" >&2; echo "$on" >&2; exit 1
fi
grep -qE 'addl[[:space:]]+\$1,' <<<"$off" || {
    echo "kill-switch control lost the original increment" >&2; echo "$off" >&2; exit 1;
}
grep -qE 'cmpl[[:space:]]+\$250000,' <<<"$off" || {
    echo "kill-switch control lost the original bound compare" >&2; echo "$off" >&2; exit 1;
}

# Zero trips must still skip the body; one and two trips pin the off-by-one
# boundary.  Larger counts exercise the same loop with a realistic body.
for opt in -O0 -O1 -O2 -O3 -Os; do
    for trips in 0 1 2 17 250000; do
        "$GCC" "$opt" "-DN=$trips" "$src" -o "$work/ref"
        env -u CCC_NO_LOOP_DOWNCOUNT CCC_VALIDATE_SSA=1 CCC_VERIFY_IR=abort \
            "$CCC" "$opt" "-DN=$trips" "$src" -o "$work/on"
        CCC_NO_LOOP_DOWNCOUNT=1 CCC_VALIDATE_SSA=1 CCC_VERIFY_IR=abort \
            "$CCC" "$opt" "-DN=$trips" "$src" -o "$work/off"
        timeout 10 "$work/ref" >"$work/ref.out"
        timeout 10 "$work/on" >"$work/on.out"
        timeout 10 "$work/off" >"$work/off.out"
        cmp "$work/ref.out" "$work/on.out" || {
            echo "GCC/default divergence at $opt N=$trips" >&2; exit 1;
        }
        cmp "$work/off.out" "$work/on.out" || {
            echo "kill-switch/default divergence at $opt N=$trips" >&2; exit 1;
        }
    done
done

# A C-level live-out control complements the IR unit tests: the induction
# value is returned after the loop, so replacing it by a remaining counter
# without rewriting that use would be wrong.  A volatile body keeps the loop.
cat >"$work/live.c" <<'EOF'
#include <stdio.h>
volatile unsigned sink;
__attribute__((noinline)) unsigned live_out(void) {
    unsigned i = 0;
    for (; i < 17; ++i)
        sink = sink * 33u + 1u;
    return i;
}
int main(void) {
    unsigned got = live_out();
    printf("live-out: %u %u\n", got, sink);
    return got != 17;
}
EOF
"$GCC" -O2 "$work/live.c" -o "$work/live-ref"
env -u CCC_NO_LOOP_DOWNCOUNT "$CCC" -O2 "$work/live.c" -o "$work/live-on"
CCC_NO_LOOP_DOWNCOUNT=1 "$CCC" -O2 "$work/live.c" -o "$work/live-off"
"$work/live-ref" >"$work/live-ref.out"
"$work/live-on" >"$work/live-on.out"
"$work/live-off" >"$work/live-off.out"
cmp "$work/live-ref.out" "$work/live-on.out"
cmp "$work/live-ref.out" "$work/live-off.out"

echo "OK loop_downcount (guard/latch structure, kill control, 25 GCC parity cells, live-out)"
