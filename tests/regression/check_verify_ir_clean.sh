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
#   1. The exact number of verify::verify_after_pass(module, ...) call lines
#      in run_passes is pinned.  Adding or removing a hook must update the
#      pin in the same commit -- that is the deliberate-recovery step.
#   2. The "abort" mode spelling MUST exist in the mode parser, or
#      CCC_VERIFY_IR=abort silently degrades below.
#   3. The same-block self-use unit test MUST exist: it is the fixture that
#      keeps the verifier's strict def<use same-block rule detectable.
HOOKS=$(grep -c 'verify::verify_after_pass(module,' src/passes/mod.rs || true)
WANT_HOOKS=19
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
done < "$LIST"

if [ "$checked" -lt 50 ]; then
    echo "FAIL gate list too short ($checked files); regenerate it" >&2
    exit 1
fi
[ "$fail" -eq 0 ] || exit 1
echo "OK check_verify_ir_clean ($checked files, -O2, CCC_VERIFY_IR=abort)"
