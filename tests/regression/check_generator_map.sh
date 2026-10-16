#!/usr/bin/env bash
# Executed regression for the generator-map vectorizer
# (src/passes/vectorize_generator_map.rs).
#
#  1. tests/regression/generator_map_vec.c is self-checking. Its output must be
#     the same literal under gcc-16 and under the lccc default pipeline, and
#     under the CCC_NO_GENERATOR_MAP=1 kill switch.
#  2. The transform must FIRE on the packed-group kernels. A silent decline on
#     every kernel would pass step 1 while measuring nothing, so the debug
#     trace must show at least the expected number of "[VEC-GEN]" rewrites.
#  3. With the kill switch set, the trace must show zero rewrites.
set -euo pipefail
cd "$(dirname "$0")/../.."
ccc=${CCC:-target/fastbuild/lccc}
ref_cc=gcc-16
command -v "$ref_cc" >/dev/null 2>&1 || ref_cc=gcc
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT

src=tests/regression/generator_map_vec.c
want="generator_map_vec: ok"
flags=(-std=gnu17 -O2 -march=x86-64-v3 -fwrapv)

"$ref_cc" -O2 -fwrapv -w "$src" -o "$td/ref"
ref_out=$("$td/ref") || { echo "FAIL gcc reference did not pass" >&2; exit 1; }
[ "$ref_out" = "$want" ] || { echo "FAIL gcc reference output: $ref_out" >&2; exit 1; }

run_one() { # <label> <env...>
    local label=$1
    shift
    local out="$td/$label"
    env "$@" "$ccc" "${flags[@]}" -o "$out.bin" "$src"
    local got rc=0
    got=$("$out.bin") || rc=$?
    if [ "$rc" != 0 ] || [ "$got" != "$want" ]; then
        echo "FAIL $label: rc=$rc got [$got]" >&2
        exit 1
    fi
}

# Default pipeline and kill switch must both reproduce the reference.
run_one default LCCC_NO_SUCH_SWITCH=1
run_one killswitch CCC_NO_GENERATOR_MAP=1

fired() {
    env "$@" LCCC_DEBUG_VECTORIZE=1 "$ccc" "${flags[@]}" -S -o /dev/null "$src" 2>&1 |
        grep -c '^\[VEC-GEN\] fn=' || true
}
on=$(fired LCCC_NO_SUCH_SWITCH=1)
off=$(fired CCC_NO_GENERATOR_MAP=1)
if [ "$on" -lt 6 ]; then
    echo "FAIL generator-map fired $on time(s); expected at least 6" >&2
    exit 1
fi
if [ "$off" -ne 0 ]; then
    echo "FAIL kill switch left $off rewrite(s) in place" >&2
    exit 1
fi
echo "generator_map: ok (fired=$on, kill-switch=$off)"
