#!/usr/bin/env bash
# Persist-gate verdict pins for the four C-level probe nests.
#
# WHY THIS EXISTS
#
# The unit tests in `loop_unroll.rs` (G1-G18) pin the gate's verdicts on
# synthetic IR fixtures. These four pins cover the same decisions END TO
# END from C sources: they catch frontend/canonicalization drift that moves
# a bound across the gate's rule boundary (e.g. if-conversion routing a
# goto through a Select instead of a multi-entry phi) as well as gate
# regressions. Each fixture lives in the regression corpus (so
# run_regression.py also pins its runtime bit-exactness vs GCC); THIS gate
# pins only the gate VERDICT (veto trace) and the resulting nest SHAPE
# (rolled loops keep their jumps, the cascading nest keeps none).
#
#   unroll_gate_goto_bound.c      veto arm=dynamic-bound (Select-routed init)
#   unroll_gate_runtime_limit.c   veto arm=dynamic-bound + rolled (jumps stay)
#   unroll_gate_iv_decided_limit.c veto arm=dynamic-bound + rolled (jumps stay)
#   unroll_gate_folded_limit.c    SILENT (allow) + fully unrolled (no jumps)
#
# The goto case pins no jump count: its `if (c)` forward branch survives in
# every shape, so a jump count cannot discriminate rolled from unrolled
# there. Every other pin is fail-closed: a missing function label (e.g. the
# probe got inlined away) fails rather than counting zero jumps.
#
# Usage:
#   tests/regression/check_unroll_gate_verdicts.sh
# Environment:
#   LCCC            compiler to test (default: target/fastbuild/lccc)
set -uo pipefail

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$repo_root"

LCCC=${LCCC:-target/fastbuild/lccc}
[[ "$LCCC" == /* ]] || LCCC="$repo_root/$LCCC"
[[ -x "$LCCC" ]] || { echo "FAIL: no compiler at $LCCC" >&2; exit 1; }

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

pass=0; fail=0
ok()  { pass=$((pass+1)); printf 'ok   %s\n' "$*"; }
bad() { fail=$((fail+1)); printf 'FAIL %s\n' "$*"; }

# $1 = test name stem (file tests/regression/unroll_gate_<stem>.c,
# function <fn>), $2 = function name
compile_probe() {
    local stem=$1 fn=$2
    CCC_UNROLL_GATE_TRACE=1 "$LCCC" -O2 -S -o "$work/$stem.s" \
        "tests/regression/unroll_gate_$stem.c" 2>"$work/$stem.trace" \
        || { bad "$stem: lccc could not compile the probe"; return 1; }
    grep -q "^$fn:" "$work/$stem.s" \
        || { bad "$stem: function $fn missing from asm (inlined away?)"; return 1; }
    return 0
}

jumps_in_fn() {
    local stem=$1 fn=$2
    sed -n "/^$fn:/,/^.size $fn,/p" "$work/$stem.s" \
        | grep -cE "^[ \t]*j(mp|b|be|ae|a|e|ne|le|l|ge|g)[ \t]"
}

expect_veto() {
    local stem=$1 fn=$2 want_jumps=$3
    compile_probe "$stem" "$fn" || return 0
    if grep -q "veto fn=$fn .* arm=dynamic-bound" "$work/$stem.trace"; then
        ok "$stem: gate vetoes (arm=dynamic-bound)"
    else
        bad "$stem: no dynamic-bound veto for $fn"
    fi
    if [[ "$want_jumps" == "yes" ]]; then
        local n
        n=$(jumps_in_fn "$stem" "$fn")
        if [[ "$n" -ge 1 ]]; then
            ok "$stem: nest stays rolled ($n jumps in $fn)"
        else
            bad "$stem: nest unrolled despite the veto (0 jumps in $fn)"
        fi
    fi
}

expect_veto goto_bound goto_bound no
expect_veto runtime_limit runtime_limit yes
expect_veto iv_decided_limit iv_decided_limit yes

if compile_probe folded_limit folded_limit; then
    if grep -q "veto fn=folded_limit " "$work/folded_limit.trace"; then
        bad "folded_limit: gate vetoed the foldable nest (must allow)"
    else
        ok "folded_limit: gate silent (allow)"
    fi
    n=$(jumps_in_fn folded_limit folded_limit)
    if [[ "$n" -eq 0 ]]; then
        ok "folded_limit: nest fully cascades (0 jumps)"
    else
        bad "folded_limit: nest not fully unrolled ($n jumps survive)"
    fi
fi

echo
echo "unroll-gate-verdicts gate: PASS=$pass FAIL=$fail"
[[ "$fail" -eq 0 ]] || exit 1
exit 0
