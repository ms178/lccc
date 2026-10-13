#!/usr/bin/env bash
# File-scope initializers of static-storage objects must be constants
# (C11 6.7.9p4), and every form lccc accepts must be emitted with its value.
#
# Two checks against the oracle (tests/regression/lib/gcc_oracle.sh):
#   * tests/regression/file_scope_init/cases.tsv: each row is one translation
#     unit. The oracle must still give the recorded verdict (drift check), lccc
#     must give the same verdict, and a rejection must carry the diagnostic.
#   * values.c: lccc and the oracle are linked and run; the printed values must
#     match. A form accepted but zeroed would pass the verdict check and fail
#     here.
set -euo pipefail
. tests/regression/lib/gcc_oracle.sh
require_gcc_oracle check_file_scope_init || exit 1

CCC=${CCC:-target/fastbuild/lccc}
dir=tests/regression/file_scope_init
cases=$dir/cases.tsv
[[ -x $CCC ]] || { echo "check_file_scope_init: FAIL: compiler not found at $CCC" >&2; exit 1; }
[[ -r $cases ]] || { echo "check_file_scope_init: FAIL: $cases missing" >&2; exit 1; }

tmp=$(mktemp -d "${TMPDIR:-/tmp}/lccc-file-scope.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
gcc_oracle_positive_control "$CCC" "$tmp" || exit 1
gcc_oracle_positive_control "$GCC_ORACLE_BIN" "$tmp" || exit 1

prelude='struct s2 { int a; int b; };'
fail=0 rows=0
while IFS=$'\t' read -r id expect diag decl; do
    [[ -z $id || $id == \#* ]] && continue
    rows=$((rows + 1))
    tu=$tmp/$id.c
    printf '%s\n%s\n' "$prelude" "$decl" > "$tu"

    if "$GCC_ORACLE_BIN" -fsyntax-only -w "$tu" >/dev/null 2>&1; then gv=accept; else gv=reject; fi
    if [[ $gv != "$expect" ]]; then
        echo "FAIL $id: oracle drift: GCC $GCC_ORACLE_MAJOR says $gv, the table says $expect" >&2
        fail=$((fail + 1)); continue
    fi
    lerr=$tmp/$id.err
    if "$CCC" -fsyntax-only "$tu" >/dev/null 2>"$lerr"; then lv=accept; else lv=reject; fi
    if [[ $lv != "$expect" ]]; then
        echo "FAIL $id: lccc $lv, GCC $GCC_ORACLE_MAJOR $expect" >&2
        sed -n '1,3p' "$lerr" >&2
        fail=$((fail + 1)); continue
    fi
    if [[ $expect == reject && $diag != - ]] && ! grep -q -- "$diag" "$lerr"; then
        echo "FAIL $id: rejected, but without '$diag'" >&2
        sed -n '1,3p' "$lerr" >&2
        fail=$((fail + 1)); continue
    fi
done < "$cases"

# Runtime values: the same program through both compilers.
vsrc=$dir/values.c
"$GCC_ORACLE_BIN" -w -c "$vsrc" -o "$tmp/values_gcc.o"
"$CCC" -c "$vsrc" -o "$tmp/values_lccc.o"
"$GCC_ORACLE_BIN" -o "$tmp/values_gcc" "$tmp/values_gcc.o"
"$GCC_ORACLE_BIN" -o "$tmp/values_lccc" "$tmp/values_lccc.o"
"$tmp/values_gcc" > "$tmp/values_gcc.out"
"$tmp/values_lccc" > "$tmp/values_lccc.out"
if ! cmp -s "$tmp/values_gcc.out" "$tmp/values_lccc.out"; then
    echo "FAIL values: folded values differ from GCC $GCC_ORACLE_MAJOR" >&2
    echo "  gcc:  $(cat "$tmp/values_gcc.out")" >&2
    echo "  lccc: $(cat "$tmp/values_lccc.out")" >&2
    fail=$((fail + 1))
fi

if (( fail > 0 )); then
    echo "check_file_scope_init: FAILED ($fail failure(s) of $rows rows + values)" >&2
    exit 1
fi
echo "check_file_scope_init: PASSED ($rows rows vs GCC $GCC_ORACLE_MAJOR; values match)"
