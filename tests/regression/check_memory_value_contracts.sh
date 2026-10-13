#!/usr/bin/env bash
set -Eeuo pipefail
cd "$(dirname "$0")/../.."
ccc=${CCC:-target/fastbuild/lccc}
. tests/regression/lib/gcc_oracle.sh
require_gcc_oracle "check_memory_value_contracts" || exit 1
ref=${CC_REF:-$GCC_ORACLE_BIN}
# The i686 leg runs "$ref" -m32; fail up front with the install hint if that
# reference cannot link (a bare link error here reads as a compiler failure).
[ -n "${CC_REF:-}" ] || gcc_oracle_require_m32 "check_memory_value_contracts" || exit 1
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT
test=setup opt=none isa=native disabled=none phase=setup
trap 'printf "FAIL test=%s opt=%s isa=%s disabled=%s phase=%s line=%s command=%s\n" "$test" "$opt" "${isa:-x86-64}" "$disabled" "$phase" "$LINENO" "$BASH_COMMAND" >&2' ERR
grep -Eq '^[[:space:]]*LCCC_NO_COMPARE=1[[:space:]]*$' tests/regression/dse_global_alias.env
gcc_include=$("$ref" -print-file-name=include)
for test in memory_value_forwarding adjacent_membership vector_address_folding integer_bounds dse_global_alias bb_slp_pure_store_fn; do
    src="tests/regression/$test.c"
    oracle_opt=-O2
    # Optimized GCC/ICC disagree on this declared GNU alias (recorded in
    # global-alias-oracles.json). Pin the specified result, not a majority
    # vote or the same optimizer defect in two compilers.
    if [[ $test == dse_global_alias ]]; then oracle_opt=-O0; fi
    phase=reference opt=$oracle_opt isa=x86-64 disabled=none
    "$ref" "$oracle_opt" "$src" -o "$td/ref"
    "$td/ref" > "$td/expected"
    if [[ $test == dse_global_alias ]]; then
        [[ $(cat "$td/expected") == 'dse-alias 17 29' ]]
    fi
    isa=i686
    "$ref" -m32 "$oracle_opt" "$src" -o "$td/ref32"
    "$td/ref32" > "$td/expected32"
    if [[ $test == dse_global_alias ]]; then
        [[ $(cat "$td/expected32") == 'dse-alias 17 29' ]]
    fi
    phase=runtime
    for opt in -O0 -O1 -O2 -O3 -Os -Oz; do
        for isa in '' '-march=x86-64-v3'; do
            "$ccc" "$opt" $isa "$src" -o "$td/check"
            "$td/check" > "$td/actual"
            cmp "$td/expected" "$td/actual"
        done
        isa=i686-sse2
        "$ccc" "$opt" -m32 -msse2 -I"$gcc_include" "$src" -o "$td/check32"
        "$td/check32" > "$td/actual32"
        cmp "$td/expected32" "$td/actual32"
    done
    if [[ $test == memory_value_forwarding ]]; then
        isa=i686-x87 opt=-O2 phase=round-store
        "$ccc" -O2 -m32 -mno-sse -mno-sse2 -I"$gcc_include" "$src" -o "$td/x87"
        "$td/x87" > "$td/x87.actual"
        cmp "$td/expected32" "$td/x87.actual"
    fi
    if [[ $test == bb_slp_pure_store_fn ]]; then
        isa=x86-64-v3 opt=-O2 phase=store-effect-controls
        for disabled in slp ipcp vecaddr; do
            CCC_DISABLE_PASSES="$disabled" "$ccc" -O2 -march=x86-64-v3 "$src" -o "$td/check"
            "$td/check" > "$td/actual"
            cmp "$td/expected" "$td/actual"
        done
    fi
    isa=x86-64-v3 opt=-O2 phase=placement-controls
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
phase=pass-dump test=expat_siphash24 isa=x86-64 disabled=none
# The existing pass dump proves placement without a dedicated timing hook.
for opt in -O2 -O3 -Os -Oz; do
    src=tests/benchmark/programs/expat_siphash24.c
    disabled=none
    CCC_DUMP_EACH_PASS=1 "$ccc" "$opt" -S "$src" -o "$td/policy.s" 2> "$td/default.ir"
    CCC_ENABLE_PREINLINE_MEMORY=1 CCC_DUMP_EACH_PASS=1 "$ccc" "$opt" -S "$src" -o "$td/policy.s" 2> "$td/enabled.ir"
    disabled=preinline_memory
    CCC_ENABLE_PREINLINE_MEMORY=1 CCC_DUMP_EACH_PASS=1 CCC_DISABLE_PASSES=preinline_memory "$ccc" "$opt" -S "$src" -o "$td/policy.s" 2> "$td/disabled.ir"
    if grep -q 'pre-loop memory-values-before-inline' "$td/default.ir" "$td/disabled.ir"; then
        echo 'FAIL: early memory cleanup ran without effective permission' >&2
        exit 1
    fi
    grep -q 'pre-loop memory-values-before-inline' "$td/enabled.ir" || {
        echo 'FAIL: early memory opt-in is ineffective' >&2
        exit 1
    }
done
# Check an isolated switch's emitted dispatch, not a literal cmpq spelling.
phase=switch-shape test=integer_bounds opt=-O2 isa=x86-64-v3 disabled=none
"$ccc" -O2 -march=x86-64-v3 -S tests/regression/integer_bounds.c -o "$td/bounded.s"
disabled=intbounds
CCC_DISABLE_PASSES=intbounds "$ccc" -O2 -march=x86-64-v3 -S tests/regression/integer_bounds.c -o "$td/unbounded.s"
disabled=none-versus-intbounds
python3 scripts/check_switch_bound_shape.py "$td/bounded.s" "$td/unbounded.s"
phase=general-runner test=dse_global_alias opt=-O2 isa=x86-64 disabled=none
python3 tests/regression/run_regression.py --filter dse_global_alias --lccc "$ccc" --gcc "$ref"
echo 'memory-value, predicate, vector-address and integer-bound contracts: PASS'
