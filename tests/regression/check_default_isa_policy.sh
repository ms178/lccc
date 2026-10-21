#!/usr/bin/env bash
# Default ISA policy: a build with no `-march=` targets exactly x86-64 (v1).
# No post-x86-64 instruction may be emitted, and no post-baseline feature
# macro may be defined. POPCNT, BMI, LZCNT, MOVBE, AVX, FMA and the rest are
# opt-in: an explicit flag or an explicit `-march=` profile (x86-64-v3 here)
# grants them. The contract is checked in both directions, so the gate cannot
# pass by refusing too much.
#
# The v1 baseline is also what GCC 14 defaults to, so the default-build
# feature-macro set is cross-checked against GCC for the same invocation.
set -euo pipefail

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
ccc=${CCC:-$repo/target/fastbuild/lccc}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

[ -x "$ccc" ] || { echo "SKIP: $ccc not built" >&2; exit 0; }

fail() { echo "FAIL: $*" >&2; exit 1; }

# A source that would tempt every post-baseline class: a popcount, trailing and
# leading zero counts (bsf/bsr are x86-64 baseline and stay allowed), a shift-and-mask that BMI would turn into one insn, a
# double multiply-add that FMA would fuse, and a loop that AVX2 would widen.
cat >"$tmp/probe.c" <<'EOF'
unsigned pc(unsigned x) { return __builtin_popcount(x); }
unsigned ctz(unsigned x) { return x ? __builtin_ctz(x) : 32u; }
unsigned clz(unsigned x) { return x ? __builtin_clz(x) : 32u; }
unsigned long long sh(unsigned long long x, unsigned n) { return (x >> n) & 0xffu; }
double fma3(double a, double b, double c) { return a * b + c; }
int sum(const int *a, int n) {
    int s = 0;
    for (int i = 0; i < n; i++) s += a[i];
    return s;
}
EOF

# 1. Default feature macros: none of the post-x86-64 set.
default_macros=$("$ccc" -dM -E "$tmp/probe.c" 2>/dev/null) || fail "lccc -dM -E failed on the default build"
for m in __SSE3__ __SSSE3__ __SSE4_1__ __SSE4_2__ __POPCNT__ __AVX__ __AVX2__ \
         __BMI__ __BMI2__ __LZCNT__ __FMA__ __F16C__ __MOVBE__ __AES__ __PCLMUL__; do
    if grep -q "^#define $m " <<<"$default_macros"; then
        fail "default build defines $m (post-x86-64; must be opt-in)"
    fi
done
echo "ok: default build defines no post-x86-64 feature macro"

# 2. Default emitted code: no post-baseline instruction, no VEX encoding.
"$ccc" -O2 -S "$tmp/probe.c" -o "$tmp/default.s" || fail "default -S failed"
if grep -Eq '^[[:space:]]+(popcnt|tzcnt|lzcnt|andn|blsr|shlx|shrx|sarx|rorx|bzhi|pdep|pext|movbe|crc32)[bwlqd]?[[:space:]]' "$tmp/default.s"; then
    grep -E '^[[:space:]]+(popcnt|tzcnt|lzcnt|andn|blsr|shlx|shrx|sarx|rorx|bzhi|pdep|pext|movbe|crc32)' "$tmp/default.s" >&2
    fail "default build emitted a post-x86-64 instruction"
fi
if grep -Eq '^[[:space:]]+v[a-z0-9]+[[:space:]]' "$tmp/default.s"; then
    sed -n '/^[[:space:]]*v[a-z0-9]*[[:space:]]/{p;q;}' "$tmp/default.s" >&2
    fail "default build emitted a VEX-encoded instruction"
fi
if grep -Eq '%ymm|%zmm' "$tmp/default.s"; then
    fail "default build used a 256/512-bit register"
fi
if grep -Eq 'vfmadd|vfmsub|vfnmadd|fmaf' "$tmp/default.s"; then
    fail "default build fused a multiply-add"
fi
echo "ok: default code has no post-x86-64 instruction, VEX encoding, or FMA"

# 3. Opt-in direction: the explicit v3 profile and the explicit POPCNT flag
#    each grant their class (so the gate also fails if the grants vanish).
"$ccc" -dM -E -march=x86-64-v3 "$tmp/probe.c" >"$tmp/v3.macros" 2>/dev/null
grep -q "^#define __POPCNT__ " "$tmp/v3.macros" \
    || fail "-march=x86-64-v3 does not define __POPCNT__"
grep -q "^#define __AVX2__ " "$tmp/v3.macros" \
    || fail "-march=x86-64-v3 does not define __AVX2__"
"$ccc" -dM -E -mpopcnt "$tmp/probe.c" >"$tmp/pop.macros" 2>/dev/null
grep -q "^#define __POPCNT__ " "$tmp/pop.macros" \
    || fail "-mpopcnt does not define __POPCNT__"
"$ccc" -O2 -S -mpopcnt "$tmp/probe.c" -o "$tmp/popcnt.s" || fail "-mpopcnt -S failed"
grep -Eq '^[[:space:]]+popcnt[lwq]?[[:space:]]' "$tmp/popcnt.s" \
    || fail "-mpopcnt did not emit popcnt for __builtin_popcount"
"$ccc" -O2 -S -march=x86-64-v3 "$tmp/probe.c" -o "$tmp/v3.s" || fail "v3 -S failed"
grep -Eq '^[[:space:]]+popcnt[lwq]?[[:space:]]' "$tmp/v3.s" \
    || fail "-march=x86-64-v3 did not emit popcnt for __builtin_popcount"
echo "ok: -march=x86-64-v3 and -mpopcnt grant their post-baseline classes"

# 4. GCC 14 oracle for the default feature set: the post-baseline macros GCC
#    leaves undefined must be undefined here too (checked only when present).
gcc_bin=${GCC:-gcc}
if command -v "$gcc_bin" >/dev/null 2>&1; then
    gcc_macros=$("$gcc_bin" -dM -E "$tmp/probe.c" 2>/dev/null)
    for m in __SSE3__ __SSSE3__ __SSE4_1__ __SSE4_2__ __POPCNT__ __AVX__ __AVX2__ __BMI__ __BMI2__ __LZCNT__ __FMA__; do
        if grep -q "^#define $m " <<<"$gcc_macros"; then
            fail "oracle inconsistency: $gcc_bin default defines $m"
        fi
    done
    echo "ok: GCC default feature macros agree (none of the post-x86-64 set)"
else
    echo "SKIP: oracle unavailable ($gcc_bin not found); the contract above still ran" >&2
fi

echo "PASS: default ISA policy (x86-64 default emits no post-baseline instruction; opt-in classes granted)"
