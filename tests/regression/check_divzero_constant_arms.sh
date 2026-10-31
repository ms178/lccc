#!/usr/bin/env bash
# GCC differential: only provably unexecuted operands suppress -Wdiv-by-zero.
# In particular, the kernel's _calc_vm_trans compares two integer constants;
# evaluating that condition as untyped i64 gets unsigned wraparound wrong.
set -euo pipefail
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
CCC=${CCC:-$root/target/fastbuild/lccc}
GCC=${GCC:-gcc-16}
command -v "$GCC" >/dev/null || { echo "missing oracle: $GCC" >&2; exit 1; }
tmp=$(mktemp -d)
trap 'rm -rf -- "$tmp"' EXIT
case_one() { # case_one name ok|error source
    local name=$1 expected=$2 source=$3 bits compiler rc
    printf '%s\n' "$source" > "$tmp/t.c"
    for bits in 32 64; do
        for compiler in "$GCC" "$CCC"; do
            rc=0
            "$compiler" -std=gnu11 -m"$bits" -Werror=div-by-zero -fsyntax-only "$tmp/t.c" >"$tmp/out" 2>&1 || rc=$?
            if [[ $expected == ok && $rc != 0 ]] || [[ $expected == error && $rc == 0 ]]; then
                echo "FAIL $name: expected $expected, $compiler -m$bits exited $rc" >&2
                cat "$tmp/out" >&2; exit 1
            fi
            if [[ $expected == error ]] && ! grep -qi 'division by zero' "$tmp/out"; then
                echo "FAIL $name: $compiler rejected for another reason" >&2
                cat "$tmp/out" >&2; exit 1
            fi
        done
    done
}
trans='#define TRANS(x,a,b) ((!(a)||!(b))?0:((a)<=(b)?((x)&(a))*((b)/(a)):((x)&(a))/((a)/(b))))'
case_one kernel-map-forward ok "$trans
unsigned long f(unsigned long x){return TRANS(x,0x080000,0x00800000);}"
case_one kernel-map-reverse ok "$trans
unsigned long f(unsigned long x){return TRANS(x,0x00800000,0x080000);}"
case_one kernel-map-zero ok "$trans
unsigned long f(unsigned long x){return TRANS(x,0,0x800000);}"
case_one lt-dead ok 'int f(int x){return (1<2)?x:1/0;}'
case_one ge-dead ok 'int f(int x){return (1>=2)?1/0:x;}'
case_one short-and ok 'int f(void){return (1>2) && (1/0);}'
case_one short-or ok 'int f(void){return (1<2) || (1/0);}'
case_one gnu-dead ok 'int f(void){return 1 ?: 1/0;}'
case_one unsigned-wrap-dead ok 'int f(int x){return (0u-1u < 1u)?1/0:x;}'
case_one unsigned-wrap-live error 'int f(int x){return (0u-1u < 1u)?x:1/0;}'
case_one unsigned-zero-dead ok 'int f(int x){return (0xffffffffu+1u)?1/0:x;}'
case_one unsigned-zero-divisor error 'int f(int x){return x/(0xffffffffu+1u);}'
case_one narrowed-zero-divisor error 'int f(int x){return x/(unsigned char)256;}'
case_one runtime-condition error 'int f(int x){return x?1/0:1;}'
case_one const-object-condition error 'int f(void){const int x=0;return x?1/0:1;}'
case_one if-zero-still-warns error 'int f(void){if(0)return 1/0;return 1;}'
case_one gnu-live error 'int f(void){return 0 ?: 1/0;}'
case_one nested-live error 'int f(void){return (1/0)?1:1/0;}'
case_one floating-condition error 'int f(void){return 1.0?1:1/0;}'
# GCC folds a constant operand of `&&`/`||` on EITHER side, and folds through
# `!`, integer casts and conditionals, so the divisor's *value* decides the
# warning rather than its constant-expression status (measured on 16.2: Clang
# 23.1.4 folds only the left operand, which is why LCCC used to miss these).
case_one and-right-zero error 'int f(int x){return x/(x && 0);}'
case_one and-chain-zero error 'int f(int x){return x/(0 && x && x);}'
case_one and-nested-zero error 'int f(int x){return x/((x && 0) && 0);}'
case_one or-of-zeros error 'int f(int x){return x/((x && 0) || (1 && 0));}'
case_one not-and-zero error 'int f(int x){return x/(!x && 0);}'
case_one cast-of-and-zero error 'int f(int x){return x/((int)(0 && x));}'
case_one all-zero-or error 'int f(int x){return x/((0 || 0) && x);}'
case_one gnu-conditional-zero error 'int f(int x){return x/(0 ?: (0 && x));}'
case_one folded-conditional-zero error 'int f(int x){return x/((1 || x) ? 0 : x);}'
case_one and-true-silent ok 'int f(int x){return x/(x && 1);}'
case_one or-unknown-silent ok 'int f(int x){return x/(x || 0);}'
case_one or-true-silent ok 'int f(int x){return x/(x || 1);}'
case_one or-chain-silent ok 'int f(int x){return x/(0 || 0 || x);}'
case_one and-plus-one-silent ok 'int f(int x){return x/((0 && x) + 1);}'
case_one cond-unknown-arms-silent ok 'int f(int x){return x/(1 ? x : 0);}'
# Truth is not preserved by narrowing conversions. These expressions are not
# syntactic ICEs because their untaken branches refer to x, but the selected
# value is known and its converted value must decide the diagnostic.
case_one narrow-selected-zero error 'int f(int x){return x/(unsigned char)(1 ? 256 : x);}'
case_one narrow-selected-zero-neg error 'int f(int x){return x/(unsigned char)(1 ? -256 : x);}'
case_one narrow-selected-nonzero ok 'int f(int x){return x/(unsigned char)(1 ? 257 : x);}'
case_one narrow-int-zero error 'int f(int x){return x/(int)(1 ? 4294967296LL : x);}'
case_one narrow-short-zero error 'int f(int x){return x/(unsigned short)(0 ? x : 65536);}'
case_one cast-boolean-zero error 'int f(int x){return x/(unsigned char)(x && 0);}'
case_one cast-boolean-one-silent ok 'int f(int x){return x/(unsigned char)(x || 1);}'
case_one not-cast-boolean-one error 'int f(int x){return x/!(unsigned char)(x || 1);}'
case_one cast-bool-selected-one ok 'int f(int x){return x/(_Bool)(1 ? 256 : x);}'
case_one cast-bool-selected-zero error 'int f(int x){return x/(_Bool)(1 ? 0 : x);}'
case_one selected-ordinary-const-silent ok 'int f(int x){const int c=0;return x/(1 ? c : x);}'

# GCC keeps the warning after an evaluated signed overflow, even with
# -fwrapv. An overflowing *unevaluated* subexpression must not poison it.
case_one signed-add-overflow error 'int f(void){return (2147483647+2147483647)==2 ? 1/0 : 1;}'
case_one signed-sub-overflow error 'int f(void){return (-2147483647-2)<0 ? 1/0 : 1;}'
case_one signed-mul-overflow error 'int f(void){return (100000*100000)==2 ? 1/0 : 1;}'
case_one unsigned-mixed-valid ok 'int f(void){return ((1LL+4294967295u)==4294967296LL) ? 1 : 1/0;}'
case_one skipped-overflow ok 'int f(void){return (1 || (2147483647+1)) ? 1 : 1/0;}'
case_one selected-overflow error 'int f(void){return ((2147483647+1) || 1) ? 1 : 1/0;}'
# GCC uses the wrapped value of an overflowing ICE to diagnose a divisor,
# but will not generally suppress a branch warning based on overflow. A
# conditional selects an arm and carries that arm's overflow flag, not the
# condition's flag; exercise both the direct and nested diagnostic paths.
case_one overflow-zero-divisor error 'int f(int x){return x/((2147483647+2147483647)==2);}'
case_one overflow-condition-selected-zero-divisor error 'int f(int x){return x/((2147483647+1)?0:0);}'
case_one overflow-condition-selected-dead ok 'int f(void){return ((2147483647+1)?0:0) ? 1/0 : 1;}'
case_one overflow-condition-selected-live error 'int f(void){return ((2147483647+1)?0:0) ? 1 : 1/0;}'
case_one overflow-selected-arm-still-warns error 'int f(void){return (1 ? 2147483647+1 : 0) ? 1/0 : 1;}'
python3 "$root/tests/regression/check_divzero_constant_matrix.py" --lccc "$CCC" --gcc "$GCC"
echo 'div-zero constant arms: PASS (GCC differential, 32/64-bit)'
