#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
ccc=${CCC:-target/fastbuild/lccc}
ref=${CC_REF:-gcc}
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT
for test in memory_value_forwarding adjacent_membership vector_address_folding integer_bounds dse_global_alias; do
    src="tests/regression/$test.c"
    oracle_opt=-O2
    # Optimized GCC/ICC disagree on this declared GNU alias (recorded in
    # global-alias-oracles.json). Pin the specified result, not a majority
    # vote or the same optimizer defect in two compilers.
    if [[ $test == dse_global_alias ]]; then oracle_opt=-O0; fi
    "$ref" "$oracle_opt" "$src" -o "$td/ref"
    "$td/ref" > "$td/expected"
    if [[ $test == dse_global_alias ]]; then
        [[ $(cat "$td/expected") == 'dse-alias 17 29' ]]
    fi
    for opt in -O0 -O1 -O2 -O3; do
        for isa in '' '-march=x86-64-v3'; do
            "$ccc" "$opt" $isa "$src" -o "$td/check"
            "$td/check" > "$td/actual"
            cmp "$td/expected" "$td/actual"
        done
        "$ref" -m32 "$oracle_opt" "$src" -o "$td/ref32"
        "$td/ref32" > "$td/expected32"
        if [[ $test == dse_global_alias ]]; then
            [[ $(cat "$td/expected32") == 'dse-alias 17 29' ]]
        fi
        "$ccc" "$opt" -m32 -msse2 -I"$($ref -print-file-name=include)" "$src" -o "$td/check32"
        "$td/check32" > "$td/actual32"
        cmp "$td/expected32" "$td/actual32"
    done
    # Both the established and new pipelines must preserve the contracts.
    for disabled in redundantloads,dse,range_fold,vecaddr,intbounds preinline_memory gaddrcse,gvn,redundantloads; do
        CCC_DISABLE_PASSES="$disabled" "$ccc" -O2 -march=x86-64-v3 "$src" -o "$td/check"
        "$td/check" > "$td/actual"
        cmp "$td/expected" "$td/actual"
    done
done
echo 'memory-value, predicate, vector-address and integer-bound contracts: PASS'
