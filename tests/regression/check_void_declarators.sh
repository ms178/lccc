#!/usr/bin/env bash
# C's constraints on `void` in declarators and on how declarator suffixes
# compose (C11 6.7p7, 6.7.2.1p3, 6.7.6.2p1, 6.7.6.3p1/p10, 6.5.2.5p1): arrays
# of void, void objects and members, misplaced void parameters, void compound
# literals, functions returning arrays or functions, arrays of functions.
#
# tests/regression/void_declarators/cases.tsv holds one translation unit per
# row. GCC 16.2 (tests/regression/lib/gcc_oracle.sh) must still give the
# recorded verdict and first error (drift check); lccc must give the same
# verdict and its recorded first error, and accepted rows must compile.
set -euo pipefail
. tests/regression/lib/gcc_oracle.sh
require_gcc_oracle check_void_declarators || exit 1

CCC=${CCC:-target/fastbuild/lccc}
cases=tests/regression/void_declarators/cases.tsv
[[ -x $CCC ]] || { echo "check_void_declarators: FAIL: compiler not found at $CCC" >&2; exit 1; }
[[ -r $cases ]] || { echo "check_void_declarators: FAIL: $cases missing" >&2; exit 1; }

tmp=$(mktemp -d "${TMPDIR:-/tmp}/lccc-void-decl.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
gcc_oracle_positive_control "$CCC" "$tmp" || exit 1
gcc_oracle_positive_control "$GCC_ORACLE_BIN" "$tmp" || exit 1

# The first error message of a compile, without the location prefix.
first_error() {
    local out
    if out=$("$@" 2>&1); then return 0; fi
    printf '%s\n' "$out" | grep -m1 'error: ' | sed 's/.*error: //'
    return 1
}

fail=0 rows=0
while IFS=$'\t' read -r id expect diag gcc_diag src; do
    [[ -z $id || $id == \#* ]] && continue
    rows=$((rows + 1))
    tu=$tmp/$id.c
    printf '%s\n' "$src" > "$tu"
    [[ $gcc_diag == = ]] && gcc_diag=$diag

    if gerr=$(first_error "$GCC_ORACLE_BIN" -std=gnu17 -w -c -o "$tmp/$id.gcc.o" "$tu"); then
        gv=accept; gerr=-
    else
        gv=reject
    fi
    if [[ $gv != "$expect" || $gerr != "$gcc_diag" ]]; then
        echo "FAIL $id: oracle drift: GCC 16.2 gives $gv ($gerr), the table says $expect ($gcc_diag)" >&2
        fail=$((fail + 1)); continue
    fi
    if lerr=$(first_error "$CCC" -std=gnu17 -c -o "$tmp/$id.lccc.o" "$tu"); then
        lv=accept; lerr=-
    else
        lv=reject
    fi
    if [[ $lv != "$expect" || $lerr != "$diag" ]]; then
        echo "FAIL $id: lccc gives $lv ($lerr), expected $expect ($diag)" >&2
        fail=$((fail + 1)); continue
    fi
done < "$cases"

if (( rows == 0 )); then
    echo "check_void_declarators: FAIL: no rows in $cases" >&2
    exit 1
fi
if (( fail )); then
    echo "check_void_declarators: FAIL: $fail of $rows rows" >&2
    exit 1
fi
echo "check_void_declarators: PASS ($rows rows)"
