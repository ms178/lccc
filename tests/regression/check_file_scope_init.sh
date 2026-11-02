#!/usr/bin/env bash
# File-scope initializers of static-storage objects must be constants
# (C11 6.7.9p4), and every form lccc accepts must be emitted with its value.
#
# Two checks against GCC 16.2 (tests/regression/lib/gcc_oracle.sh):
#   * tests/regression/file_scope_init/cases.tsv: each row is one translation
#     unit. The oracle must still give the recorded verdict (drift check), lccc
#     must give the same verdict, and a rejection must carry the diagnostic.
#   * values.c: lccc and GCC 16.2 are linked and run; the printed values must
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

# Runtime values: the same program through both compilers.
vsrc=$dir/values.c
"$GCC_ORACLE_BIN" -w -c "$vsrc" -o "$tmp/values_gcc.o"
"$CCC" -c "$vsrc" -o "$tmp/values_lccc.o"
"$GCC_ORACLE_BIN" -o "$tmp/values_gcc" "$tmp/values_gcc.o"
"$GCC_ORACLE_BIN" -o "$tmp/values_lccc" "$tmp/values_lccc.o"
"$tmp/values_gcc" > "$tmp/values_gcc.out"
"$tmp/values_lccc" > "$tmp/values_lccc.out"
if ! cmp -s "$tmp/values_gcc.out" "$tmp/values_lccc.out"; then
    echo "FAIL values: folded values differ from GCC 16.2" >&2
    echo "  gcc:  $(cat "$tmp/values_gcc.out")" >&2
    echo "  lccc: $(cat "$tmp/values_lccc.out")" >&2
    fail=$((fail + 1))
fi

# &*p cancels even for a null pointer, but its operand must still be a
# constant in a static initializer. Compile (not merely -fsyntax-only), so
# these checks exercise lowering, the emitted relocations and runtime values.
for width in 32 64; do
    for std in gnu11 gnu2x; do
        for opt in 0 2; do
            flags=("-m$width" "-std=$std" "-O$opt")
            for compiler in "$GCC_ORACLE_BIN" "$CCC"; do
                "$compiler" "${flags[@]}" -c "$dir/addrof_deref_values.c" -o "$tmp/addrof.o"
                "$GCC_ORACLE_BIN" "-m$width" "$tmp/addrof.o" -o "$tmp/addrof"
                if [[ $("$tmp/addrof") != 1 ]]; then
                    echo "FAIL addrof-deref values: $compiler ${flags[*]}" >&2
                    fail=$((fail + 1))
                fi
            done
        done
    done
done

# String-literal truth in a static conditional: check values AND selected
# pointer relocations, at both widths and optimization levels.
for width in 32 64; do
    for std in gnu11 gnu2x; do
        for opt in 0 2; do
            flags=("-m$width" "-std=$std" "-O$opt")
            for compiler in "$GCC_ORACLE_BIN" "$CCC"; do
                "$compiler" "${flags[@]}" -c "$dir/string_condition_values.c" -o "$tmp/string_cond.o"
                "$GCC_ORACLE_BIN" "-m$width" "$tmp/string_cond.o" -o "$tmp/string_cond"
                if [[ $("$tmp/string_cond") != 1 ]]; then
                    echo "FAIL string-condition values: $compiler ${flags[*]}" >&2
                    fail=$((fail + 1))
                fi
            done
        done
    done
done
# Nonconstant conditions, including an address truncated to an integer, must
# not become true merely because a string occurs somewhere beneath a cast.
for decl in \
    'extern char *p; int x = p ? 1 : 2;' \
    'int x = (unsigned char)(unsigned long)"s" ? 1 : 2;'; do
    printf '%s\n' "$decl" > "$tmp/string_cond_bad.c"
    for compiler in "$GCC_ORACLE_BIN" "$CCC"; do
        if "$compiler" -std=gnu11 -w -c "$tmp/string_cond_bad.c" -o "$tmp/string_cond_bad.o" >"$tmp/string_cond_bad.log" 2>&1; then
            echo "FAIL string-condition nonconstant: $compiler accepted $decl" >&2
            fail=$((fail + 1))
        fi
    done
done

# Cancellation does not make a read of a mutable pointer or a call constant,
# and an address of an automatic object remains invalid at static scope.
for decl in \
    'int *p; int *q = &*p;' \
    'int *get(void); int *q = &*get();' \
    'void f(int *p) { static int *q = &*p; }' \
    'void f(void) { int a; static int *q = &*(&a); }'; do
    printf '%s\n' "$decl" > "$tmp/addrof_bad.c"
    for compiler in "$GCC_ORACLE_BIN" "$CCC"; do
        if "$compiler" -std=gnu11 -c "$tmp/addrof_bad.c" -o "$tmp/addrof_bad.o" >"$tmp/addrof_bad.log" 2>&1; then
            echo "FAIL addrof-deref nonconstant: $compiler accepted $decl" >&2
            fail=$((fail + 1))
        elif ! grep -q 'not constant' "$tmp/addrof_bad.log"; then
            echo "FAIL addrof-deref nonconstant: $compiler missing diagnostic for $decl" >&2
            cat "$tmp/addrof_bad.log" >&2
            fail=$((fail + 1))
        fi
    done
done

if (( fail > 0 )); then
    echo "check_file_scope_init: FAILED ($fail failure(s) of $rows rows + values)" >&2
    exit 1
fi
echo "check_file_scope_init: PASSED ($rows rows vs GCC $GCC_ORACLE_MAJOR; values match; &* m32/m64 GNU11/GNU2x O0/O2 and nonconstant controls)"
