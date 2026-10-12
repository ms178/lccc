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
    for opt in -O0 -O1 -O2 -O3 -Os -Oz; do
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
    # Early placement remains available, but is no longer the runtime default.
    CCC_ENABLE_PREINLINE_MEMORY=1 "$ccc" -O2 -march=x86-64-v3 "$src" -o "$td/check"
    "$td/check" > "$td/actual"
    cmp "$td/expected" "$td/actual"
    # Both the established and new pipelines must preserve the contracts.
    for disabled in redundantloads,dse,range_fold,vecaddr,intbounds preinline_memory gaddrcse,gvn,redundantloads; do
        CCC_ENABLE_PREINLINE_MEMORY=1 CCC_DISABLE_PASSES="$disabled" "$ccc" -O2 -march=x86-64-v3 "$src" -o "$td/check"
        "$td/check" > "$td/actual"
        cmp "$td/expected" "$td/actual"
    done
done
# Presence/absence of the stage's own timer proves the control is effective.
for opt in -O2 -O3 -Os -Oz; do
    src=tests/benchmark/programs/expat_siphash24.c
    CCC_TIME_PREINLINE_MEMORY=1 "$ccc" "$opt" -S "$src" -o "$td/policy.s" 2> "$td/default.time"
    CCC_ENABLE_PREINLINE_MEMORY=1 CCC_TIME_PREINLINE_MEMORY=1 "$ccc" "$opt" -S "$src" -o "$td/policy.s" 2> "$td/enabled.time"
    CCC_ENABLE_PREINLINE_MEMORY=1 CCC_TIME_PREINLINE_MEMORY=1 CCC_DISABLE_PASSES=preinline_memory "$ccc" "$opt" -S "$src" -o "$td/policy.s" 2> "$td/disabled.time"
    if grep -q '\[TIME\] preinline memory:' "$td/default.time" "$td/disabled.time"; then
        echo 'FAIL: early memory cleanup ran without effective permission' >&2
        exit 1
    fi
    grep -q '\[TIME\] preinline memory:' "$td/enabled.time" || {
        echo 'FAIL: early memory opt-in is ineffective' >&2
        exit 1
    }
done
# A module-scoped disable control must reach post-phi switch emission too.
# Output equality alone would let a no-op control pass unnoticed.
"$ccc" -O2 -march=x86-64-v3 -fwrapv -S tests/benchmark/programs/switch_dispatch.c -o "$td/bounded.s"
CCC_DISABLE_PASSES=intbounds "$ccc" -O2 -march=x86-64-v3 -fwrapv -S tests/benchmark/programs/switch_dispatch.c -o "$td/unbounded.s"
if grep -Eq '^[[:space:]]*cmpq[[:space:]]+\$16,' "$td/bounded.s"; then
    echo 'FAIL: proved switch still has the outer bound comparison' >&2
    exit 1
fi
grep -Eq '^[[:space:]]*cmpq[[:space:]]+\$16,' "$td/unbounded.s" || {
    echo 'FAIL: disabling intbounds did not restore the switch guard' >&2
    exit 1
}
echo 'memory-value, predicate, vector-address and integer-bound contracts: PASS'
