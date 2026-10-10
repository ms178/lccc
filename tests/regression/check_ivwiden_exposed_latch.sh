#!/usr/bin/env bash
# Executed regression: the iv_widen theorem-5' trajectory gates and the
# exposed-latch intercept, over four loop shapes that carry a runtime
# ring-top seed (see tests/regression/ivwiden_exposed_latch.c).
#
# The fixture self-checks and exits non-zero when a case reads one of the
# markers placed past the 2^32 ring top, so this script's job is to run it
# under the matrix and compare against the gcc oracle.  Three arms:
#   1. the default pipeline,
#   2. the CCC_NO_IV_WIDEN=1 kill switch (an escape hatch nobody exercises
#      rots into a no-op or a crash),
#   3. -m32, where the probe cannot distinguish narrow wrap from wide
#      no-wrap and must print SKIP with exit 0.
set -euo pipefail
cd "$(dirname "$0")/../.."
ccc=${CCC:-target/fastbuild/lccc}
src=tests/regression/ivwiden_exposed_latch.c
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT
inc=$(gcc -print-file-name=include)
for opt in -O0 -O1 -O2 -O3; do
    gcc "$opt" -w "$src" -o "$td/ref"
    ref=$("$td/ref")
    # The fixture self-checks and exits non-zero on a marker read; capture
    # rc and output so the FAIL line carries the discriminating values.
    echo "IVWIDEN: default $opt"
    "$ccc" "$opt" "$src" -o "$td/test"
    rc=0
    got=$("$td/test") || rc=$?
    { [ "$rc" = 0 ] && [ "$got" = "$ref" ]; } || { echo "FAIL default $opt: rc=$rc got [$got] want [$ref]"; exit 1; }
    echo "PASS"
    echo "IVWIDEN: no-iv-widen $opt"
    CCC_NO_IV_WIDEN=1 "$ccc" "$opt" "$src" -o "$td/test-off"
    rc=0
    got_off=$("$td/test-off") || rc=$?
    { [ "$rc" = 0 ] && [ "$got_off" = "$ref" ]; } || { echo "FAIL no-iv-widen $opt: rc=$rc got [$got_off] want [$ref]"; exit 1; }
    echo "PASS"
    echo "IVWIDEN: m32 $opt"
    "$ccc" "$opt" -m32 -msse2 -I"$inc" "$src" -o "$td/test32"
    got32=$("$td/test32")
    case "$got32" in
        *SKIP*) ;;
        *) echo "FAIL m32 $opt: expected SKIP, got [$got32]"; exit 1 ;;
    esac
    echo "PASS"
done
echo "IVWIDEN-EXPOSED-LATCH: PASS"
