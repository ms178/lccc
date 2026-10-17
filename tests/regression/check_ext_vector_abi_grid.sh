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
#
# Verdict: the set of failing checks per leg is compared EXACTLY with
# ext_vector_abi/grid_expectations.txt, which records (a) checks on which
# Clang 23.1.3 disagrees with itself (no reference exists) and (b) open lccc
# defects, each with a tracking id. An unlisted failure is red; so is a
# listed failure that stops happening (remove its line with the fix). The
# summary line reports both classes so the open-defect count is never
# invisible in a green run.
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
expectations=$root/tests/regression/ext_vector_abi/grid_expectations.txt
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# expected[<bits> <opt> <shape> <prov:call>] = "check check ..."; class per check.
declare -A expected expected_class
while read -r e_bits e_opt e_shape e_leg e_check e_class e_id _; do
    [[ -z $e_bits || $e_bits == \#* ]] && continue
    case $e_class in
        oracle|lccc|flaky) ;;
        *) echo "FAIL: $expectations: unknown class '$e_class' for $e_bits $e_opt $e_shape $e_leg $e_check" >&2; exit 1 ;;
    esac
    [[ -n $e_id ]] || { echo "FAIL: $expectations: missing tracking id for $e_bits $e_opt $e_shape $e_leg $e_check" >&2; exit 1; }
    expected["$e_bits $e_opt $e_shape $e_leg"]+="$e_check "
    expected_class["$e_bits $e_opt $e_shape $e_leg $e_check"]=$e_class
done < "$expectations"
xfail_oracle=0
xfail_lccc=0
flaky_seen=0
flaky_total=0
declare -A seen_expected

# leg_verdict <key> <rc> <run.log>: compare the checks the probe reported as
# failing with the expectation for this leg. Returns 0 when they agree.
leg_verdict() {
    local key=$1 rc=$2 log=$3
    local actual want extra missing chk flaky judged
    actual=$(awk '/^FAIL /{print $2}' "$log" | sort -u | tr '\n' ' ')
    want=$(tr ' ' '\n' <<<"${expected[$key]:-}" | sed '/^$/d' | sort -u | tr '\n' ' ')
    seen_expected[$key]=1
    if [[ $rc -ne 0 && -z $actual ]]; then
        fail "$key: probe exited $rc without a verdict: $(tr '\n' ' ' <"$log")"
        return 1
    fi
    # A `flaky` check is neither required nor forbidden: take it out of both
    # sets before the exact comparison, and count the occurrences.
    flaky=
    judged=
    for chk in $want; do
        if [[ ${expected_class["$key $chk"]} == flaky ]]; then flaky+="$chk "; else judged+="$chk "; fi
    done
    want=$judged
    judged=
    for chk in $actual; do
        if [[ " $flaky " == *" $chk "* ]]; then flaky_seen=$((flaky_seen + 1)); else judged+="$chk "; fi
    done
    actual=$judged
    for chk in $flaky; do flaky_total=$((flaky_total + 1)); done
    extra=
    missing=
    for chk in $actual; do [[ " $want " == *" $chk "* ]] || extra+="$chk "; done
    for chk in $want; do [[ " $actual " == *" $chk "* ]] || missing+="$chk "; done
    if [[ -n $extra ]]; then
        fail "$key: unexpected failure(s): $extra"
    fi
    if [[ -n $missing ]]; then
        fail "$key: expected failure(s) did not occur: $missing-- remove the line(s) from $(basename "$expectations") in the commit that fixes them"
    fi
    [[ -z $extra && -z $missing ]] || return 1
    for chk in $actual; do
        case ${expected_class["$key $chk"]} in
            oracle) xfail_oracle=$((xfail_oracle + 1)) ;;
            lccc) xfail_lccc=$((xfail_lccc + 1)) ;;
        esac
    done
    return 0
}

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
ran_m32=0
for bits in 64 32; do
    if [[ $bits == 32 ]] && ! i386_run_or_skip "$gcc_bin"; then
        if [[ ${LCCC_REQUIRE_I686:-0} == 1 ]]; then exit 1; fi
        continue
    fi
    [[ $bits == 32 ]] && ran_m32=1
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
                leg_verdict "$bits $opt $shape $leg" "$rc" "$tmp/run.log" || true
                checked=$((checked + 1))
            done
        done
    done
done

# Every expectation must belong to a leg this run actually exercised:
# a stale key (a shape or leg that no longer runs) would never be retired.
for key in "${!expected[@]}"; do
    [[ ${seen_expected[$key]:-0} == 1 ]] && continue
    if [[ $key == 32\ * ]] && [[ ${ran_m32:-0} == 0 ]]; then continue; fi
    fail "$key: listed in $(basename "$expectations") but no such leg ran"
done

if [[ $fails -ne 0 ]]; then
    echo "FAIL: ext-vector ABI grid: $fails failure(s) ($checked legs run, $rejected rejection checks)" >&2
    exit 1
fi
echo "PASS: ext-vector ABI grid vs Clang $CLANG_ORACLE_VERSION: $checked legs, $rejected rejection checks;" \
    "expected failures held: $xfail_oracle oracle-inconsistent checks, $xfail_lccc open lccc defects," \
    "$flaky_seen of $flaky_total flaky checks failed this run" \
    "(tests/regression/ext_vector_abi/grid_expectations.txt)"
