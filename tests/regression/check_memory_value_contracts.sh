#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
ccc=${CCC:-target/fastbuild/lccc}
ref=${CC_REF:-gcc}
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT
for test in memory_value_forwarding adjacent_membership vector_address_folding integer_bounds; do
    src="tests/regression/$test.c"
    "$ref" -O2 "$src" -o "$td/ref"
    "$td/ref" > "$td/expected"
    for opt in -O0 -O1 -O2 -O3; do
        for isa in '' '-march=x86-64-v3'; do
            "$ccc" "$opt" $isa "$src" -o "$td/check"
            "$td/check" > "$td/actual"
            cmp "$td/expected" "$td/actual"
        done
        "$ref" -m32 -O2 "$src" -o "$td/ref32"
        "$td/ref32" > "$td/expected32"
        "$ccc" "$opt" -m32 -msse2 -I"$($ref -print-file-name=include)" "$src" -o "$td/check32"
        "$td/check32" > "$td/actual32"
        cmp "$td/expected32" "$td/actual32"
    done
    # Both the established and new pipelines must preserve the contracts.
    CCC_DISABLE_PASSES=redundantloads,dse,range_fold,vecaddr,intbounds "$ccc" -O2 -march=x86-64-v3 "$src" -o "$td/check"
    "$td/check" > "$td/actual"
    cmp "$td/expected" "$td/actual"
done
echo 'memory-value, predicate and vector-address contracts: PASS'
