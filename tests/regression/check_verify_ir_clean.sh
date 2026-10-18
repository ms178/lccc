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
# Scope: this proves the verifier is clean on the subset, not that it is
# live.  The verifier's detection itself is covered by its unit tests in
# src/passes/verify/tests.rs, which feed it malformed IR.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
cd "$ROOT"

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
