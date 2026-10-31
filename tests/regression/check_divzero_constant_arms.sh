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

echo 'div-zero constant arms: PASS (GCC differential, 32/64-bit)'
