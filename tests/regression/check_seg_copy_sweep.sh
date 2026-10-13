#!/usr/bin/env bash
# Size sweep for aggregate copies to and from __seg_gs objects.
#
# gen_seg_copy_sweep.py emits one self-checking program with a __seg_gs object
# per size (192 sizes by default: every length 1..=64, the lengths 65..=512
# with residue 1, 8 or 15 modulo 16, and the loop edges 511, 512, 513..=544,
# 1000, 1023..=1025, 2047..=2049, 4095..=4097 and 5000). This script:
#
#   1. builds the program with $CCC at -O0 and -O2, links it through the normal
#      driver, and runs each binary;
#   2. requires the final PASSED line and exit status 0 from every lccc build;
#   3. requires each lccc run to print exactly the same lines as a gcc -O0 build
#      (GCC is the oracle for the accepted behaviour).
#
# The copy path is the straight-line code up to SEG_COPY_UNROLL_MAX (512 bytes)
# and the counted loop with its tail above it. The sweep covers both, plus the
# tails of every width, so an off-by-one in either path fails here.
#
# Skips (exit 0, with a message) on non-x86-64 hosts: the program uses the
# x86-64 GS base and arch_prctl(ARCH_SET_GS).
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
gen=tests/regression/seg_gs_runtime/gen_seg_copy_sweep.py

if [ "$(uname -m)" != "x86_64" ] || [ "$(uname -s)" != "Linux" ]; then
    echo "SKIP: seg copy sweep needs x86-64 Linux (arch_prctl ARCH_SET_GS)"
    exit 0
fi

tmp=$(mktemp -d "${TMPDIR:-/tmp}/lccc-seg-copy-sweep.XXXXXX")
trap 'rm -rf "$tmp"' EXIT

python3 "$gen" "$tmp/sweep.c"

fail=0
. tests/regression/lib/gcc_oracle.sh
require_gcc_oracle "check_seg_copy_sweep" || exit 1
"$GCC_ORACLE_BIN" -O0 -w -o "$tmp/oracle" "$tmp/sweep.c"
"$tmp/oracle" > "$tmp/oracle.out" 2>&1 || true
if [ "$(tail -n 1 "$tmp/oracle.out")" != "seg_copy_sweep PASSED (192 sizes)" ]; then
    echo "FAIL: gcc -O0 oracle did not pass the sweep"
    tail -n 5 "$tmp/oracle.out"
    exit 1
fi

for opt in -O0 -O2; do
    bin="$tmp/sweep$opt"
    if ! "$CCC" "$opt" -w "$tmp/sweep.c" -o "$bin" > "$tmp/build$opt.log" 2>&1; then
        echo "FAIL: lccc $opt build"
        tail -5 "$tmp/build$opt.log"
        fail=1
        continue
    fi
    rc=0
    "$bin" > "$tmp/run$opt.out" 2>&1 || rc=$?
    if [ "$rc" -ne 0 ] || [ "$(tail -n 1 "$tmp/run$opt.out")" != "seg_copy_sweep PASSED (192 sizes)" ]; then
        echo "FAIL: lccc $opt run rc=$rc"
        sed -n '/FAIL/p' "$tmp/run$opt.out" | sed -n '1,10p'
        tail -n 3 "$tmp/run$opt.out"
        fail=1
        continue
    fi
    if ! diff -u "$tmp/oracle.out" "$tmp/run$opt.out" > "$tmp/diff$opt"; then
        echo "FAIL: lccc $opt output differs from gcc -O0"
        sed -n '1,20p' "$tmp/diff$opt"
        fail=1
        continue
    fi
    echo "ok   lccc $opt: $(wc -l < "$tmp/run$opt.out") lines identical to gcc -O0"
done

if [ "$fail" -ne 0 ]; then
    echo "seg copy sweep: FAILED"
    exit 1
fi
echo "seg copy sweep: PASSED"
