#!/usr/bin/env bash
# Declarator-placement contract for named address spaces.
#
# Every fixture in tests/regression/seg_decl/ is compiled with $CCC and with
# gcc when gcc is available:
#   neg_*.c  must be REJECTED (non-zero exit with an error diagnostic);
#   pos_*.c  must be ACCEPTED (zero exit).
# The file name is the expected verdict. When gcc is present, gcc must give
# the same verdict, so a fixture cannot pass with a wrong expectation.
#
# Covered: a named space on a parameter's own object (`long *__seg_gs p`,
# `long (*__seg_gs p)[4]`, scalar `__seg_gs long x`), named-space array decay
# to a generic pointer in an initializer, argument, or local initializer, and
# a pointer assignment across spaces. Accepted: a space on the pointee, a
# typedef pointer, a grouped array pointer, and an explicit cast.
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
dir=tests/regression/seg_decl
tmp=$(mktemp -d "${TMPDIR:-/tmp}/lccc-seg-decl.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
have_gcc=0
command -v gcc >/dev/null 2>&1 && have_gcc=1
fail=0
shopt -s nullglob
files=("$dir"/neg_*.c "$dir"/pos_*.c)
if [ "${#files[@]}" -eq 0 ]; then
    echo "FAIL: no fixtures in $dir" >&2
    exit 1
fi
for f in "${files[@]}"; do
    base=$(basename "$f")
    case "$base" in
        neg_*) want=reject ;;
        pos_*) want=accept ;;
    esac
    set +e
    lerr=$("$CCC" -c -o "$tmp/x.o" "$f" 2>&1)
    lrc=$?
    set -e
    if [ "$lrc" -ne 0 ]; then got=reject; else got=accept; fi
    if [ "$want" = reject ] && [ "$got" = reject ] && ! grep -q 'error' <<<"$lerr"; then
        echo "FAIL: lccc rejects $base without an error diagnostic" >&2
        fail=1
        continue
    fi
    if [ "$got" != "$want" ]; then
        echo "FAIL: lccc $got $base (expected $want)" >&2
        grep -m2 . <<<"$lerr" >&2 || true
        fail=1
        continue
    fi
    if [ "$have_gcc" -eq 1 ]; then
        set +e
        gcc -std=gnu11 -c -o "$tmp/g.o" "$f" >/dev/null 2>&1
        grc=$?
        set -e
        if [ "$grc" -ne 0 ]; then gg=reject; else gg=accept; fi
        if [ "$gg" != "$want" ]; then
            echo "FAIL: fixture $base expects $want but gcc $gg; fix the fixture" >&2
            fail=1
            continue
        fi
    fi
    echo "PASS: $base lccc $got$( [ "$have_gcc" -eq 1 ] && echo " (gcc $gg)")"
done
if [ "$fail" -ne 0 ]; then
    echo "check_seg_declarator_space: FAILED" >&2
    exit 1
fi
echo "check_seg_declarator_space: PASSED (${#files[@]} fixtures)"
