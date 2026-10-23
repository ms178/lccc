#!/usr/bin/env bash
# __attribute__((target("..."))) contract on x86-64 (GCC semantics):
#   1. the attributed function is compiled with the named ISA, and the ISA
#      is scoped to it: plain functions before AND after it in the same TU
#      keep the TU's baseline (the per-function state is restored);
#   2. a caller lacking the features keeps the CALL (GCC refuses to inline:
#      the clone would execute BMI2 on the dispatcher's baseline path);
#   3. a caller whose own target covers the callee may inline it;
#   4. the program computes the same result as GCC at run time.
# zstd relies on all four: its *_bmi2 clones are dispatched at run time
# from plain code (DYNAMIC_BMI2).
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
GCC_ORACLE=${GCC_ORACLE:-$(command -v gcc-16 || true)}
tmp=${TMPDIR:-/tmp}/lccc-targetattr.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"
fail() { echo "FAIL: $* (${BASH_SOURCE[0]##*/}:${BASH_LINENO[0]})" >&2; exit 1; }
cat >"$tmp/t.c" <<'C'
typedef unsigned long long u64;
u64 shl_before(u64 x, unsigned n) { return x << (n & 63); }
__attribute__((target("bmi2"))) u64 shl_bmi2(u64 x, unsigned n) { return x << (n & 63); }
u64 shl_after(u64 x, unsigned n) { return x << (n & 63); }
/* Two independent variable shifts (not a rotate, whose optimal form is a
 * plain `rol %cl`): the clone's optimal lowering is shlx + shrx. */
__attribute__((noinline, target("bmi,bmi2"))) static u64 rot_body(u64 x, unsigned n) {
    return (x << (n & 63)) + (x >> ((n >> 3) & 63));
}
/* The UB-free masked rotate is one rotate instruction at any ISA. */
u64 rotl_masked(u64 x, unsigned n) { return (x << (n & 63)) | (x >> (-n & 63)); }
/* Plain dispatcher: must CALL the BMI2 clone, never absorb it. */
__attribute__((noinline)) u64 dispatch(u64 x, unsigned n) { return rot_body(x, n) ^ 1; }
static inline u64 mix(u64 x, unsigned n) { return (x >> (n & 63)) + 7; }
/* Covering caller: a plain static inline callee may be absorbed. */
__attribute__((target("bmi,bmi2"))) u64 covered(u64 x, unsigned n) { return mix(x, n) * 3; }
int main(void) {
    volatile unsigned n = 13;
    u64 acc = 0x0123456789abcdefull;
    for (unsigned i = 0; i < 64; i++)
        acc = dispatch(acc, n + i) + covered(acc, i) + shl_before(acc, i) - shl_after(acc, n)
            + rotl_masked(acc, n + 3 * i);
    __builtin_printf("%#llx\n", acc);
    return 0;
}
C
"$CCC" -O2 -S "$tmp/t.c" -o "$tmp/t.s"
# body <function> -> its instruction lines
body() { awk -v f="$1:" '$1==f{on=1;next} on&&/^\t?\.cfi_endproc|^\t?\.size/{exit} on' "$tmp/t.s"; }
# has <function> <ERE>: the body is captured first and matched from a here-string,
# so no pipeline consumer can exit early and SIGPIPE the producer under pipefail.
has() { local b; b=$(body "$1"); grep -qE -- "$2" <<<"$b"; }

# 1. ISA scoped to the attributed function.
has shl_bmi2 'shlx' || fail "target(\"bmi2\") function did not use shlx"
for f in shl_before shl_after dispatch; do
    has "$f" 'shlx|shrx|sarx|rorx' && fail "plain $f leaked BMI2 forms"
done
has rot_body 'shlx' && has rot_body 'shrx' \
    || fail "target(\"bmi,bmi2\") rot_body lacks its shlx/shrx forms"
has rotl_masked '\b(rolq|rorq)\b' || fail "masked rotate idiom is not one rotate"
has rotl_masked '\b(shlq|shrq|orq)\b' && fail "masked rotate left as shifts"

# 2. Plain caller keeps the call.
has dispatch 'call\s+rot_body' || fail "dispatch inlined/omitted the target clone"

# 3. Covering caller absorbs the plain static inline callee.
has covered 'call\s+mix' && fail "covered failed to inline a plain callee"

# 4. Runtime result equals GCC's (on hosts that can run BMI2 code).
if grep -qw bmi2 /proc/cpuinfo; then
    "$CCC" -O2 "$tmp/t.c" -o "$tmp/t"
    got=$("$tmp/t")
    if [[ -n "$GCC_ORACLE" ]]; then
        "$GCC_ORACLE" -O2 "$tmp/t.c" -o "$tmp/g"
        want=$("$tmp/g")
        [ "$got" = "$want" ] || fail "runtime $got != GCC $want"
        echo "oracle: $got matches $("$GCC_ORACLE" -dumpfullversion)"
    else
        echo "SKIP-ORACLE: no gcc-16; runtime ran ($got)"
    fi
else
    echo "SKIP-RUN: host CPU lacks BMI2; assembly checks passed"
fi
echo "target-attribute ISA gate: PASS"
