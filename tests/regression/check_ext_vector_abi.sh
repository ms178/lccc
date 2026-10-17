#!/usr/bin/env bash
# ExtVector cross-ABI gate: argument AND return classification, both directions.
#
# A same-compiler build cannot see a carrier mismatch: if a compiler returns a
# 2-lane double vector in xmm0/xmm1 but the caller expects two x87 values, both
# halves come from that compiler and stay consistent. Splitting the callee
# (provider) and the caller into two translation units and mixing the compilers
# makes every classification disagreement observable.
#
# Four legs per (bits, opt) combination:
#   clang -> clang   the oracle agrees with itself (oracle sanity)
#   clang -> lccc    LCCC must *call* Clang-compiled ExtVector functions
#   lccc  -> clang   Clang must call LCCC-compiled ExtVector functions
#   lccc  -> lccc    LCCC agrees with itself
#
# The oracle sanity leg matters: a shape that Clang cannot even agree with
# itself on is an oracle defect, not an LCCC defect, and is excluded here with
# the measurement recorded (see SKIP below).
set -euo pipefail
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
ccc=${CCC:-$root/target/fastbuild/lccc}
gcc_bin=${GCC:-gcc}
# The oracle is the pinned Clang release, the same one the ABI grid
# (check_ext_vector_abi_grid.sh) and lccc's own i386 ext-vector convention
# follow. "Whatever clang is on PATH" is not a reference: Clang's i386
# return convention for 8-byte vectors (u32x2, u64x1 below) changed between
# releases, so an older oracle reports lccc's correct Clang-23 convention as
# a mismatch. A missing oracle FAILS (never a silent skip of the cross legs).
# shellcheck source=tests/regression/lib/clang_oracle.sh
. "$root/tests/regression/lib/clang_oracle.sh"
require_clang_oracle check_ext_vector_abi || exit 1
clang_bin=$CLANG_ORACLE_BIN
source "$root/tests/regression/i386_exec.sh"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
dir=$root/tests/regression/ext_vector_abi
provider=$dir/ext_vector_abi_provider.c
caller=$dir/ext_vector_abi_caller.c

[ -x "$ccc" ] || { echo "FAIL: compiler not built: $ccc" >&2; exit 1; }


compile_with() {
    # compile_with <compiler-id> <out> <src> <extra flags...>
    local who=$1 out=$2 src=$3
    shift 3
    case $who in
        lccc) "$ccc" "$@" "$src" -o "$out" ;;
        clang) "$clang_bin" "$@" "$src" -o "$out" ;;
        *) echo "FAIL: unknown compiler id $who" >&2; exit 1 ;;
    esac
}

# No shape is excluded. The caller supports -DABI_SKIP_<SHAPE> for a
# MEASURED oracle defect; the exclusions this gate carried were measurements
# of Clang 19 (x87-pair returns, a self-inconsistent i386 bool4 at -O2) and
# of lccc's former i386 sret convention, all superseded by the Clang 23.1.3
# convention lccc now implements (tests/regression/check_ext_vector_abi_grid.sh
# records the i386 and x86-64 measurements shape by shape). Re-adding one
# requires its evidence here, as before.

run_leg() {
    local bits=$1 opt=$2 prov=$3 call=$4 tag=$5
    local -a flags=(-m"$bits" "$opt" -no-pie)
    local obj_p=$tmp/prov-$tag-$bits$opt.o obj_c=$tmp/call-$tag-$bits$opt.o exe=$tmp/exe-$tag-$bits$opt
    if ! compile_with "$prov" "$obj_p" "$provider" "${flags[@]}" -c >"$tmp/$tag-p.log" 2>&1; then
        echo "FAIL: ext-vector ABI provider ($prov) compile -m$bits $opt" >&2
        cat "$tmp/$tag-p.log" >&2
        exit 1
    fi
    if ! compile_with "$call" "$obj_c" "$caller" "${flags[@]}" -c >"$tmp/$tag-c.log" 2>&1; then
        echo "FAIL: ext-vector ABI caller ($call) compile -m$bits $opt" >&2
        cat "$tmp/$tag-c.log" >&2
        exit 1
    fi
    if ! "$gcc_bin" -m"$bits" -no-pie -o "$exe" "$obj_p" "$obj_c" >"$tmp/$tag-link.log" 2>&1; then
        echo "FAIL: ext-vector ABI link ($prov -> $call) -m$bits $opt" >&2
        cat "$tmp/$tag-link.log" >&2
        exit 1
    fi
    local rc=0
    "$exe" >"$tmp/$tag-run.out" || rc=$?
    if [[ $rc -ne 0 ]]; then
        echo "FAIL: ext-vector cross-ABI $prov -> $call -m$bits $opt (exit $rc)" >&2
        cat "$tmp/$tag-run.out" >&2
        exit 1
    fi
    echo "PASS: ext-vector cross-ABI -m$bits ($prov provider, $call caller, $opt)"
}

for bits in 64 32; do
    if [[ $bits == 32 ]] && ! i386_run_or_skip "$gcc_bin"; then
        if [[ ${LCCC_REQUIRE_I686:-0} == 1 ]]; then exit 1; fi
        continue
    fi
    for opt in -O0 -O1 -O2; do
        run_leg "$bits" "$opt" lccc lccc lccc-lccc
        run_leg "$bits" "$opt" clang clang clang-clang
        run_leg "$bits" "$opt" clang lccc clang-lccc
        run_leg "$bits" "$opt" lccc clang lccc-clang
    done
done
