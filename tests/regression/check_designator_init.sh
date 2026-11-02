#!/usr/bin/env bash
# Designator validity in braced initializers (C11 6.7.9p2-p6).
#
# A `.name` designator must name a member (searching anonymous members); a
# `[i]` designator must be a non-negative index within a known array length.
# Rows are in tests/regression/designator_init/cases.tsv and were measured
# against GCC 16.2. Each row is checked twice:
#   * the oracle must still give the recorded verdict (drift check), and
#   * lccc must give the same verdict; a rejection must carry the diagnostic.
# The oracle is GCC 16.2 (tests/regression/lib/gcc_oracle.sh). A missing oracle
# fails the gate; it is never skipped.
set -euo pipefail
. tests/regression/lib/gcc_oracle.sh
require_gcc_oracle check_designator_init || exit 1

CCC=${CCC:-target/fastbuild/lccc}
cases=tests/regression/designator_init/cases.tsv
[[ -x $CCC ]] || { echo "check_designator_init: FAIL: compiler not found at $CCC" >&2; exit 1; }

tmp=$(mktemp -d "${TMPDIR:-/tmp}/lccc-designator.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
gcc_oracle_positive_control "$CCC" "$tmp" || exit 1
gcc_oracle_positive_control "$GCC_ORACLE_BIN" "$tmp" || exit 1

fail=0 rows=0
while IFS=$'\t' read -r id expect diag body; do
    [[ -z $id || $id == \#* ]] && continue
    rows=$((rows + 1))
    tu=$tmp/$id.c
    printf '%s\n' "$body" > "$tu"

    if "$GCC_ORACLE_BIN" -fsyntax-only -w "$tu" >/dev/null 2>&1; then gv=accept; else gv=reject; fi
    if [[ $gv != "$expect" ]]; then
        echo "FAIL $id: oracle drift: GCC 16.2 says $gv, the table says $expect" >&2
        fail=$((fail + 1)); continue
    fi

    lerr=$tmp/$id.err
    if "$CCC" -fsyntax-only "$tu" >/dev/null 2>"$lerr"; then lv=accept; else lv=reject; fi
    if [[ $lv != "$expect" ]]; then
        echo "FAIL $id: lccc $lv, GCC 16.2 $expect" >&2
        sed -n '1,3p' "$lerr" >&2
        fail=$((fail + 1)); continue
    fi
    if [[ $expect == reject && $diag != - ]] && ! grep -q -- "$diag" "$lerr"; then
        echo "FAIL $id: rejected, but without '$diag'" >&2
        sed -n '1,3p' "$lerr" >&2
        fail=$((fail + 1)); continue
    fi
done < "$cases"

if (( fail > 0 )); then
    echo "check_designator_init: FAILED ($fail of $rows rows)" >&2
    exit 1
fi
echo "check_designator_init: PASSED ($rows rows vs GCC 16)"
