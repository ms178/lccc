#!/usr/bin/env bash
# T6 gate: the IR verifier must stay clean on a fixed corpus subset.
#
# Every file in verify_ir_gate_files.txt is compiled at -O2 with
# CCC_VERIFY_IR=abort, which verifies the IR after every pass and turns the
# first violation into a failing compile.  Any non-zero exit is a pass that
# produced malformed IR.  Cost on the full 2 656-file sweep was +4.7% compile
# time (scripts/measure_verify_ir_cost.py); this gate uses 80 files, so it
# costs seconds.
#
# Scope: this proves the verifier is clean on the subset, and -- via the
# static liveness wiring below -- that the verifier is still invoked at all.
# Detection strength itself is covered by the unit tests in
# src/passes/verify/tests.rs, which feed it malformed IR.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
cd "$ROOT"

# Static liveness wiring, fail closed.  Without hooks this gate would pass
# vacuously forever (no hook calls -> no violations), so a neutered wiring
# must be as fatal as a real violation:
#   1. The verify::verify_after_pass(module, ...) wiring in src/passes/mod.rs
#      is pinned as a per-pass INVENTORY, not a bare integer: every literal
#      hook site is counted under its pass name, and the macro funnels (one
#      source line each, expanding per pass) are pinned separately.  Adding
#      or dropping a hook therefore fails with the pass name in the message,
#      and "recovering" means naming what changed -- bumping an integer
#      cannot silently re-baseline the gate.
#   2. The "abort" mode spelling MUST exist in the mode parser, or
#      CCC_VERIFY_IR=abort silently degrades below.
#   3. The same-block self-use unit test MUST exist: it is the fixture that
#      keeps the verifier's strict def<use same-block rule detectable.
HOOKS=$(grep -c 'verify::verify_after_pass(module,' src/passes/mod.rs || true)
# Literal sites, "<count> <pass>", sorted by name.  Duplicates are real: the
# pass is wired at two distinct points (pre-loop and in-loop / O1 and O2).
WANT_LITERAL=$(sort -k2 <<'EOF'
1 aggcopy16
2 aggregate_sroa
1 deadslotload
1 diffcmp
2 fmanegpeel
1 fp_double
1 inline
1 intbounds
1 interval
1 jump_thread
1 jump_thread_cleanup
1 load_forward
2 loop_carried_forward
1 loop_carried_forward_late
1 loop_idiom
1 loop_preheader
1 masked_guard_splat_hoist
1 pair_load_carry
1 reassoc_lat
1 satsub
1 slp
1 slp_late
2 sret_copy_elision
1 vec_interleave_late
1 vecaddr
1 vecfma
1 vecrelations
1 vecssa
EOF
)
# `[a-z0-9_]*`: pass names carry digits (`aggcopy16`), and a matcher that
# could not see them would silently pin an inventory that omits them.
GOT_LITERAL=$(grep -o 'verify::verify_after_pass(module, "[a-z0-9_]*")' src/passes/mod.rs \
    | sed 's/.*"\([a-z0-9_]*\)".*/\1/' | sort | uniq -c | awk '{print $1, $2}' | sort -k2)
if [ "$GOT_LITERAL" != "$WANT_LITERAL" ]; then
    echo "FAIL: literal verify_after_pass hook inventory drifted from the pin." >&2
    diff -u <(printf '%s\n' "$WANT_LITERAL") <(printf '%s\n' "$GOT_LITERAL") >&2 || true
    echo "      If a pass hook was added/removed on purpose, name it in WANT_LITERAL" >&2
    echo "      in this same change (count first, then pass name)." >&2
    exit 1
fi
# Macro funnels: one source line each, but each expands once per pass it
# wraps.  Pinned by macro name so a funnel cannot be deleted quietly.
WANT_MACRO_SITES=5
MACRO_SITES=$(grep -c 'verify::verify_after_pass(module, \$name)' src/passes/mod.rs || true)
if [ "$MACRO_SITES" -ne "$WANT_MACRO_SITES" ]; then
    echo "FAIL: $MACRO_SITES macro hook funnels (\$name) pinned at $WANT_MACRO_SITES." >&2
    exit 1
fi
for funnel in iphase_dump o1_checkpoint preloop_dump timed_pass; do
    grep -q "macro_rules! $funnel" src/passes/mod.rs \
        || { echo "FAIL: hook funnel $funnel is gone from src/passes/mod.rs" >&2; exit 1; }
done
WANT_HOOKS=37
if [ "$HOOKS" -ne "$WANT_HOOKS" ]; then
    echo "FAIL: $HOOKS verify::verify_after_pass hooks in src/passes/mod.rs, pinned at $WANT_HOOKS." >&2
    echo "      If a pass hook was added/removed on purpose, update WANT_HOOKS in this same change." >&2
    exit 1
fi
grep -q '"abort"' src/passes/verify.rs \
    || { echo 'FAIL: "abort" mode spelling missing from src/passes/verify.rs' >&2; exit 1; }
grep -q 'fn an_instruction_consuming_its_own_result_is_reported' src/passes/verify/tests.rs \
    || { echo 'FAIL: same-block self-use verifier unit test missing' >&2; exit 1; }
echo "liveness: $HOOKS verify_after_pass hooks pinned, abort spelling + self-use test present"

CCC=${CCC:-target/fastbuild/lccc}
LIST=tests/regression/verify_ir_gate_files.txt
tmp=$(mktemp -d "${TMPDIR:-/tmp}/verify-ir-gate.XXXXXX")
trap 'rm -rf "$tmp"' EXIT

checked=0
fail=0
while IFS= read -r src; do
    case $src in ''|'#'*) continue ;; esac
    [ -f "$src" ] || { echo "FAIL missing gate file: $src" >&2; fail=1; continue; }
    checked=$((checked + 1))
    if ! CCC_VERIFY_IR=abort "$CCC" -O2 -w -c "$src" -o "$tmp/o.o" 2>"$tmp/err"; then
        echo "FAIL verifier aborted on $src:" >&2
        tail -n 5 "$tmp/err" >&2
        fail=1
    fi
done < <(cat "$LIST" - <<'PINNED'
# Pinned regressions, outside the regenerated sample: synthesized functions
# the sampled sweep cannot reach. target_clones resolvers once carried
# next_label=0 below their live labels ("stale label counter").
tests/regression/target_clones_multiversion.c
PINNED
)

if [ "$checked" -lt 50 ]; then
    echo "FAIL gate list too short ($checked files); regenerate it" >&2
    exit 1
fi
[ "$fail" -eq 0 ] || exit 1
echo "OK check_verify_ir_clean ($checked files, -O2, CCC_VERIFY_IR=abort)"
