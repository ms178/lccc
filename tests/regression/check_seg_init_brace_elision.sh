#!/usr/bin/env bash
# Named-address-space initializers through brace elision and anonymous members
# (C11 6.7.9p17-p20, 6.7.2.2p13), checked against GCC 16.2 line by line.
#
# A braced list is walked as the object model says: positional items take the
# next subobject, an unmatched expression elides the braces, designators reach
# into unnamed struct and union members, and unnamed bit-fields take no slot.
# Before this gate, lccc skipped any record with an unnamed member, so
# `struct A { int x; struct { int y; gp_t p; }; } l = { 1, 2, q };` with a
# generic `q` compiled and stored a generic pointer into a __seg_gs object.
#
# Each fixture under tests/regression/seg_init_brace/ is compiled by GCC 16.2 and
# by lccc. The set of diagnostic lines must be identical. `pos_*` fixtures are
# valid and must have no errors in either compiler; `neg_*` fixtures must be
# rejected by both, on the same lines.
set -euo pipefail
. tests/regression/lib/gcc_oracle.sh
require_gcc_oracle check_seg_init_brace_elision || exit 1

CCC=${CCC:-target/fastbuild/lccc}
[[ -x $CCC ]] || { echo "check_seg_init_brace_elision: FAIL: compiler not found at $CCC" >&2; exit 1; }
tmp=$(mktemp -d "${TMPDIR:-/tmp}/lccc-seginit.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
gcc_oracle_positive_control "$CCC" "$tmp" || exit 1
gcc_oracle_positive_control "$GCC_ORACLE_BIN" "$tmp" || exit 1

# Sorted unique line numbers of `error` diagnostics (warnings are not compared:
# GCC reports excess elements as warnings, and lccc may word them differently).
error_lines() {
    grep -E '^[^ ]+:[0-9]+:[0-9]+: error' "$1" | cut -d: -f2 | sort -un | tr '\n' ' ' || true
}

fail=0
checked=0
for f in tests/regression/seg_init_brace/*.c; do
    name=$(basename "$f")
    "$GCC_ORACLE_BIN" -w -c -o "$tmp/g.o" "$f" >"$tmp/g.err" 2>&1 || true
    "$CCC" -c -o "$tmp/l.o" "$f" >"$tmp/l.err" 2>&1 || true
    g=$(error_lines "$tmp/g.err")
    l=$(error_lines "$tmp/l.err")
    checked=$((checked + 1))
    case $name in
        pos_*)
            if [[ -n $g || -n $l ]]; then
                echo "FAIL $name: valid fixture rejected (GCC 16.2 lines: [${g}] lccc lines: [${l}])" >&2
                fail=$((fail + 1))
            fi
            ;;
        neg_*)
            if [[ -z $g ]]; then
                echo "FAIL $name: fixture drift, GCC 16.2 accepts it" >&2
                fail=$((fail + 1))
            elif [[ $g != "$l" ]]; then
                echo "FAIL $name: error lines differ (GCC 16.2: [${g}] lccc: [${l}])" >&2
                diff <(grep -E ': error' "$tmp/g.err" | sort) <(grep -E ': error' "$tmp/l.err" | sort) >&2 || true
                fail=$((fail + 1))
            fi
            ;;
        *)
            echo "FAIL $name: fixture name must start with pos_ or neg_" >&2
            fail=$((fail + 1))
            ;;
    esac
done
if (( checked == 0 )); then
    echo "check_seg_init_brace_elision: FAIL: no fixtures found" >&2
    exit 1
fi
if (( fail > 0 )); then
    echo "check_seg_init_brace_elision: FAILED ($fail of $checked fixtures)" >&2
    exit 1
fi
echo "check_seg_init_brace_elision: PASSED ($checked fixtures, error lines match GCC 16.2)"
