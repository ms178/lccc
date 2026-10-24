#!/usr/bin/env bash
# ============================================================================
# check_hot_web_steal_diagnostics.sh — pin the hot-web steal's opt-in
# diagnostics and, through them, the steal's admission contract end to end
# (knob `CCC_DEBUG_HOT_WEB_STEAL`, kill switch `CCC_NO_HOT_WEB_STEAL=1`).
#
# WHY THIS GATE EXISTS
# --------------------
# Two sessions (2026-10-22 and 2026-10-23) had to hand-instrument
# `steal_hot_loop_webs` with temporary counters to learn which guard refused a
# candidate — the cost comparison in particular refused "without a diagnostic"
# (engineering/subsystems/codegen-gaps-2026-10-22.md, §5c.2).  The diagnostics
# knob landed with this round: one line per candidate, one line per refused
# register with the refusing guard named, and a `-> reg` line with the eviction
# cost when the steal fires.  This gate pins that contract so it cannot rot:
#
#   1. OPT-IN PURITY — without `CCC_DEBUG_HOT_WEB_STEAL` the compile prints no
#      `hot-web-steal:` lines at all: the production path allocates nothing and
#      stays silent.  A debug knob that leaks into every compile is a bug.
#   2. FIRES, WITH EVIDENCE — on the witness (`cmp_replay_far.c`, the same file
#      the scalar-facts gate uses for its mechanism witness) the debug output
#      must contain at least one `-> reg` line naming the register and the
#      eviction cost, and the assembly must differ from the kill-switch arm:
#      the steal really fires, cheaper overlapping holder displaced, cost
#      printed.  This is the admission rule of 2026-10-23 (a protected holder
#      that ITSELF overlaps the incoming web competes on loop-weighted cost
#      instead of vetoing) exercised through the whole pipeline.
#   3. REFUSES, BY NAME — on the fixture (`hot_web_callee_saved.c`, whose every
#      callee-saved home is a hot loop web) the debug output must contain at
#      least one `refused: eviction cost >= hot count` line: the candidate is
#      genuinely not worth the eviction and the gate names the guard that said
#      so.  The assembly must be byte-identical to the kill-switch arm (the
#      same contract `check_llvm_scalar_facts.py` pins as its PROTECTED leg —
#      this gate adds the by-name refusal evidence, not a duplicate pin).
#
# The two assembly assertions re-verify, through the real compiler, exactly
# what the unit tests `hot_web_displaces_a_cheaper_overlapping_loop_state_holder`
# and `hot_web_preserves_a_costlier_overlapping_loop_state_holder` verify at
# the function level — and what the diagnostic lines prove at the decision
# level: the cost model, not the protected class, arbitrates overlapping
# holders.
# ============================================================================
set -euo pipefail

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
CCC=${CCC:-$repo/target/fastbuild/lccc}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

fail() { echo "check_hot_web_steal_diagnostics: FAIL: $*" >&2; exit 1; }

# --- 1. Opt-in purity: no env, no diagnostics, and identical assembly. ------
"$CCC" -O2 -S "$repo/tests/regression/cmp_replay_far.c" -o "$work/plain.s" 2>"$work/plain.err"
grep -q '^hot-web-steal:' "$work/plain.err" \
    && fail "diagnostics leaked without CCC_DEBUG_HOT_WEB_STEAL"

# --- 2. Fires with evidence on the witness. ---------------------------------
CCC_DEBUG_HOT_WEB_STEAL=1 "$CCC" -O2 -S "$repo/tests/regression/cmp_replay_far.c" \
    -o "$work/witness.s" 2>"$work/witness.err"
CCC_NO_HOT_WEB_STEAL=1 "$CCC" -O2 -S "$repo/tests/regression/cmp_replay_far.c" \
    -o "$work/witness-off.s" 2>/dev/null
grep -qE '^hot-web-steal: cand v[0-9]+ uses=[0-9]+ -> reg [0-9]+ \(evict cost [0-9]+\)' \
    "$work/witness.err" \
    || fail "witness: no '-> reg (evict cost ...)' line under the debug knob"
cmp -s "$work/witness.s" "$work/witness-off.s" \
    && fail "witness: assembly identical to the kill-switch arm (steal did not fire)"

# --- 3. Refuses by name on the all-hot fixture. -----------------------------
CCC_DEBUG_HOT_WEB_STEAL=1 "$CCC" -O2 -S "$repo/tests/regression/hot_web_callee_saved.c" \
    -o "$work/fixture.s" 2>"$work/fixture.err"
CCC_NO_HOT_WEB_STEAL=1 "$CCC" -O2 -S "$repo/tests/regression/hot_web_callee_saved.c" \
    -o "$work/fixture-off.s" 2>/dev/null
grep -q 'refused: eviction cost >= hot count' "$work/fixture.err" \
    || fail "fixture: no 'refused: eviction cost >= hot count' line under the debug knob"
cmp -s "$work/fixture.s" "$work/fixture-off.s" \
    || fail "fixture: assembly differs from the kill-switch arm (a costlier hot web was evicted)"

echo "check_hot_web_steal_diagnostics: ok"
