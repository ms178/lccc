#!/usr/bin/env bash
# CC-O0CALL-1: output/exit matrix, including the no-peephole and no-GLA arms.
set -euo pipefail
cd "$(dirname "$0")/../.."
CCC=${CCC:-target/fastbuild/lccc}
GCC=${GCC:-gcc}
t=$(mktemp -d)
trap 'rm -rf "$t"' EXIT
src=tests/regression/call_secondary_cache_clobber.c
"$GCC" -O2 "$src" -o "$t/ref"
timeout 20 "$t/ref" > "$t/expected"
count=0
for opt in -O0 -O1 -O2 -O3 -Os; do
    for mode in default no-peephole no-gla neither; do
        knobs=()
        case "$mode" in
            no-peephole) knobs=(CCC_NO_PEEPHOLE=1) ;;
            no-gla) knobs=(CCC_RA_GLOBAL_LOCATION=0) ;;
            neither) knobs=(CCC_NO_PEEPHOLE=1 CCC_RA_GLOBAL_LOCATION=0) ;;
        esac
        env -u CCC_NO_PEEPHOLE -u CCC_RA_GLOBAL_LOCATION "${knobs[@]}" \
            "$CCC" "$opt" "$src" -o "$t/run"
        timeout 20 "$t/run" > "$t/actual"
        cmp "$t/expected" "$t/actual"
        count=$((count + 1))
    done
done
printf 'call-secondary-cache: PASS (%d output-checked arms)\n' "$count"
