#!/usr/bin/env bash
# By-value ext_vector_type ABI grid against the Clang 23.1.3 oracle.
#
# GCC has no ext_vector_type, so Clang is the reference implementation of
# its calling convention (tests/regression/lib/clang_oracle.sh pins the
# exact release). For every shape below, on -m64 and -m32 at -O0 and -O2,
# ext_vector_abi/abi_grid.c is built as provider and as caller by both
# compilers and linked in all four combinations:
#   clang -> clang   oracle sanity (a failure here is an oracle defect)
#   clang -> lccc    lccc calls Clang-compiled functions
#   lccc  -> clang   Clang calls lccc-compiled functions
#   lccc  -> lccc    lccc agrees with itself
# The caller checks every lane of identity returns, a call whose fourth
# vector lands past all register pieces (i386: %xmm0-2), an argument-only
# hash and variadic va_arg passing.
#
# Shapes whose Clang convention has no lccc carrier (a return in two to four
# %xmm, %rax/%rdx/%rcx or %st(0)/%st(1); a _Bool vector over 64 lanes as an
# argument) must be REJECTED with a diagnostic, never silently compiled; the
# rest of their ABI (arguments, varargs) is still checked with -DNO_RET.
# The lists are the measurements recorded in the i386 and x86-64 tables of
# the Clang 23.1.3 ext-vector ABI notes (engineering/FOLLOWUP-2026-10-15-pr833.md).
set -euo pipefail
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
ccc=${CCC:-$root/target/fastbuild/lccc}
gcc_bin=${GCC:-gcc}
# shellcheck source=tests/regression/lib/clang_oracle.sh
. "$root/tests/regression/lib/clang_oracle.sh"
require_clang_oracle check_ext_vector_abi_grid || exit 1
# shellcheck source=tests/regression/i386_exec.sh
source "$root/tests/regression/i386_exec.sh"
[ -x "$ccc" ] || { echo "FAIL: compiler not built: $ccc" >&2; exit 1; }
src=$root/tests/regression/ext_vector_abi/abi_grid.c
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

shapes="u8x1 u8x2 u8x3 u8x4 u8x5 u8x7 u8x8 u8x12 u8x16 u8x32
u16x1 u16x2 u16x3 u16x4 u16x5 u16x8 u16x12 u16x16
u32x1 u32x2 u32x3 u32x4 u32x5 u32x8 u32x16
u64x1 u64x2 u64x3 u64x4 u64x8
f32x1 f32x2 f32x3 f32x4 f32x5 f32x8 f32x16
f64x1 f64x2 f64x3 f64x4 f64x8
ldx1 ldx2 ldx3
bx1 bx2 bx3 bx4 bx8 bx16 bx32 bx64 bx128"

elem_of() {
    case ${1%x*} in
        u8) echo "unsigned char" ;;
        u16) echo "unsigned short" ;;
        u32) echo "unsigned" ;;
        u64) echo "unsigned long long" ;;
        f32) echo "float" ;;
        f64) echo "double" ;;
        ld) echo "long double" ;;
        b) echo "_Bool" ;;
        *) echo "FAIL: unknown shape $1" >&2; exit 1 ;;
    esac
}

in_list() { [[ " $2 " == *" $1 "* ]]; }

# Clang 23.1.3 returns these in a multi-register sequence on both targets.
ret_unsupported_both="u8x32 u16x16 u32x8 u32x16 u64x4 u64x8 f32x8 f32x16 f64x4 f64x8 ldx2"
ret_unsupported() {
    local bits=$1 shape=$2
    in_list "$shape" "$ret_unsupported_both" && return 0
    if [[ $bits == 64 ]]; then in_list "$shape" "u64x3 f64x3"; else in_list "$shape" "bx128"; fi
}
arg_unsupported() { [[ $2 == bx128 ]]; }

fails=0
fail() {
    echo "FAIL: $*" >&2
    fails=$((fails + 1))
}

# expect_rejected <bits> <shape> <what> <flags...>: lccc must refuse the
# provider with the by-value diagnostic, and Clang must accept it.
expect_rejected() {
    local bits=$1 shape=$2 what=$3
    shift 3
    if "$ccc" "$@" -c "$src" -DPROVIDER -o "$tmp/neg.o" >"$tmp/neg.log" 2>&1; then
        fail "-m$bits $shape: lccc accepted a by-value $what Clang passes in an unsupported register sequence"
    elif ! grep -q "by value is not supported" "$tmp/neg.log"; then
        fail "-m$bits $shape: lccc rejected the $what without the by-value diagnostic"
        cat "$tmp/neg.log" >&2
    fi
    if ! "$CLANG_ORACLE_BIN" "$@" -c "$src" -DPROVIDER -o "$tmp/neg.o" >"$tmp/neg.log" 2>&1; then
        fail "-m$bits $shape: the Clang oracle rejected the negative-control source"
        cat "$tmp/neg.log" >&2
    fi
}

checked=0
rejected=0
for bits in 64 32; do
    if [[ $bits == 32 ]] && ! i386_run_or_skip "$gcc_bin"; then
        if [[ ${LCCC_REQUIRE_I686:-0} == 1 ]]; then exit 1; fi
        continue
    fi
    for opt in -O0 -O2; do
        for shape in $shapes; do
            elem=$(elem_of "$shape")
            lanes=${shape##*x}
            flags=(-m"$bits" "$opt" -no-pie -w "-DELEM=$elem" "-DLANES=$lanes")
            if arg_unsupported "$bits" "$shape"; then
                if [[ $opt == -O0 ]]; then
                    expect_rejected "$bits" "$shape" argument "${flags[@]}"
                    rejected=$((rejected + 1))
                fi
                continue
            fi
            if ret_unsupported "$bits" "$shape"; then
                if [[ $opt == -O0 ]]; then
                    expect_rejected "$bits" "$shape" return "${flags[@]}"
                    rejected=$((rejected + 1))
                fi
                flags+=(-DNO_RET)
            fi
            ok=1
            for who in lccc clang; do
                cc=$ccc
                [[ $who == clang ]] && cc=$CLANG_ORACLE_BIN
                for side in p c; do
                    def=()
                    [[ $side == p ]] && def=(-DPROVIDER)
                    if ! "$cc" "${flags[@]}" "${def[@]}" -c "$src" -o "$tmp/$side-$who.o" \
                        >"$tmp/cc.log" 2>&1; then
                        fail "-m$bits $opt $shape: $who failed to compile the $side side"
                        cat "$tmp/cc.log" >&2
                        ok=0
                    fi
                done
            done
            [[ $ok == 1 ]] || continue
            for leg in clang:clang clang:lccc lccc:clang lccc:lccc; do
                prov=${leg%:*}
                call=${leg#*:}
                exe=$tmp/t
                if ! "$gcc_bin" -m"$bits" -no-pie "$tmp/p-$prov.o" "$tmp/c-$call.o" -o "$exe" \
                    >"$tmp/ld.log" 2>&1; then
                    fail "-m$bits $opt $shape: link $prov -> $call"
                    cat "$tmp/ld.log" >&2
                    continue
                fi
                rc=0
                timeout 10 "$exe" >"$tmp/run.log" 2>&1 || rc=$?
                if [[ $rc -ne 0 ]]; then
                    fail "-m$bits $opt $shape: $prov provider -> $call caller (exit $rc): $(tr '\n' ' ' <"$tmp/run.log")"
                fi
                checked=$((checked + 1))
            done
        done
    done
done

if [[ $fails -ne 0 ]]; then
    echo "FAIL: ext-vector ABI grid: $fails failure(s) ($checked legs run, $rejected rejection checks)" >&2
    exit 1
fi
echo "PASS: ext-vector ABI grid vs Clang $CLANG_ORACLE_VERSION: $checked legs, $rejected rejection checks"
