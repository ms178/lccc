#!/usr/bin/env bash
# Implicit conversions between named and generic pointers, through identifier
# arrays, struct members of a __seg_gs object, subscripts of a __seg_gs array,
# arguments, assignments, returns and initializers.
#
# Every fixture in tests/regression/seg_conv/ is compiled with $CCC and with gcc
# when gcc is available:
#   neg_*.c  must be REJECTED, with an address-space diagnostic;
#   pos_*.c  must be ACCEPTED.
# The file name is the expected verdict. When gcc is present, gcc must give the
# same verdict, so a fixture cannot pass with a wrong expectation. GCC 14.2 is
# the oracle for the verdicts.
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
dir=tests/regression/seg_conv
tmp=$(mktemp -d "${TMPDIR:-/tmp}/lccc-seg-conv.XXXXXX")
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
    if [ "$got" != "$want" ]; then
        echo "FAIL: lccc $got $base (expected $want)" >&2
        grep -m2 . <<<"$lerr" >&2 || true
        fail=1
        continue
    fi
    # Assignment, initializer and return conversions say "incompatible address
    # space"; an argument keeps the argument diagnostic (same rule as
    # check_seg_named_decl_qualifier.sh).
    if [ "$want" = reject ] && ! grep -q -e 'address space' -e 'incompatible pointer type for argument' <<<"$lerr"; then
        echo "FAIL: lccc rejects $base without an address-space diagnostic" >&2
        grep -m2 . <<<"$lerr" >&2 || true
        fail=1
        continue
    fi
    if [ "$have_gcc" -eq 1 ]; then
        set +e
        gcc -w -c -o "$tmp/g.o" "$f" >/dev/null 2>&1
        grc=$?
        set -e
        if [ "$grc" -ne 0 ]; then gg=reject; else gg=accept; fi
        if [ "$gg" != "$want" ]; then
            echo "FAIL: fixture $base expects $want but gcc $gg; fix the fixture" >&2
            fail=1
            continue
        fi
    fi
done
if [ "$fail" -ne 0 ]; then
    echo "check_seg_conv_diagnostics: FAILED"
    exit 1
fi
echo "check_seg_conv_diagnostics: PASSED (${#files[@]} files)"
