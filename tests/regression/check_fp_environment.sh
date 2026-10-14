#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
ccc=${CCC:-target/fastbuild/lccc}
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT
src=tests/regression/strict_recip_traps.c
for opt in -O0 -O1 -O2 -O3 -Os -Oz; do
    for mode in pragma flag both; do
        flags=()
        case $mode in
            flag) flags=(-DNO_FENV_PRAGMA -ftrapping-math);;
            both) flags=(-ftrapping-math);;
        esac
        "$ccc" "$opt" -march=x86-64-v3 -ffp-contract=off "${flags[@]}" "$src" -lm -o "$td/trap"
        "$td/trap"
        echo "FP first-trap order: $opt $mode PASS"
    done
done
# No observable FP environment requested: the exact same loop must remain
# eligible. This is a structural check, not execution with contradictory
# trapping assumptions (-fno-trapping-math plus feenableexcept).
"$ccc" -O2 -march=x86-64-v3 -DNO_FENV_PRAGMA -ftrapping-math -fno-trapping-math -ffp-contract=off -S "$src" -o "$td/free.s"
grep -q 'vdivpd' "$td/free.s"
"$ccc" -O2 -march=x86-64-v3 -DNO_FENV_PRAGMA -fno-trapping-math -ftrapping-math -ffp-contract=off -S "$src" -o "$td/trapping.s"
if grep -q 'vdivpd' "$td/trapping.s"; then
    echo 'FAIL: packed reciprocal escaped observable-FP environment fence' >&2; exit 1
fi
