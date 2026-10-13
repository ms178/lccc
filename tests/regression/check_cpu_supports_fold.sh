#!/usr/bin/env bash
# AUD-CPU-1 — `__builtin_cpu_supports` must respect -march, -mno-*, and the
# target architecture.
#
# Before the fix the builtin folded against a hard-coded "Raptor Lake" allowlist
# that ignored every one of them: `-march=x86-64` reported AVX2/BMI2/SHA as
# present, `-mno-avx2` changed nothing, and the aarch64 driver answered x86
# feature names too. Runtime-dispatch libraries (zlib-ng, zstd, xz, OpenSSL,
# FFmpeg) therefore always took their widest path — a guaranteed SIGILL on
# hardware without the feature.
#
# The contract pinned here is "fold to 1 iff the effective code-generation
# permission of this invocation guarantees the feature":
#
#   * no -march=  -> the project code-generation baseline, x86-64-v3
#                    (documented in src/backend/x86/isa.rs)
#   * -march=x86-64 / -v2 / -v3 / <cpu>  -> that profile's set
#   * -mno-<feat> -> that rung folds to 0, sticky against the baseline
#   * non-x86     -> a diagnostic, never a silent constant
#
# Features with no codegen permission bit (sha, adx, rdseed, gfni, avx512*,
# amx*, ...) fold to 0 even for a Raptor Lake profile: reporting a feature the
# backend cannot promise is exactly the SIGILL bug. The runtime
# CPUID/libgcc-`__cpu_model` form is tracked separately as AUD-CPU-2.
#
# vaes/vpclmulqdq ARE tracked, as the 256-bit permission they are: the fold
# grants them only when the 256-bit vector class is permitted AND the
# extension's own flag was given, which is the same conjunction the intrinsic
# gate enforces (tests/regression/check_intrinsic_feature_gate.sh, GCC-checked)
# and the same one GCC uses.
set -euo pipefail

CCC=${CCC:-./target/fastbuild/lccc}
CCC_ARM=${CCC_ARM:-./target/fastbuild/lccc-arm}
CCC_I686=${CCC_I686:-./target/fastbuild/lccc-i686}
if [[ ! -x $CCC ]]; then
    echo "SKIP: $CCC not built" >&2
    exit 1
fi
abs() {
    local dir
    dir=$(cd "$(dirname -- "$1")" && pwd)
    printf '%s/%s\n' "$dir" "$(basename -- "$1")"
}
CCC=$(abs "$CCC")
for var in CCC_ARM CCC_I686; do
    if [[ -x ${!var} ]]; then printf -v "$var" '%s' "$(abs "${!var}")"; fi
done

tmp=$(mktemp -d "${TMPDIR:-/tmp}/lccc-cpu-support.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
cd "$tmp"

fail() { echo "FAIL: $*" >&2; exit 1; }

# The probe prints the granted set (one feature name per hit, in a fixed order)
# and exits with the same bitmask, so a mismatch names the offending feature
# instead of a mystery integer.
cat >probe.c <<'C'
#include <stdio.h>
#define P(n, b) do { if (__builtin_cpu_supports(n)) { fputs(n " ", stdout); v |= 1 << (b); } } while (0)
int main(void) {
    int v = 0;
    P("cmov", 0);   P("fxsr", 1);   P("mmx", 2);    P("sse", 3);
    P("sse2", 4);   P("sse3", 5);   P("ssse3", 6);  P("sse4.1", 7);
    P("sse4.2", 8); P("popcnt", 9); P("cx16", 10);  P("avx", 11);
    P("avx2", 12);  P("fma", 13);   P("f16c", 14);  P("bmi", 15);
    P("bmi2", 16);  P("lzcnt", 17); P("movbe", 18); P("aes", 19);
    P("pclmul", 20); P("rdrnd", 21);
    P("vaes", 22);   P("vpclmulqdq", 23);
    printf("| 0x%x\n", v);
    return 0;
}
C

V3="cmov fxsr mmx sse sse2 sse3 ssse3 sse4.1 sse4.2 popcnt cx16 avx avx2 fma f16c bmi bmi2 lzcnt movbe"
# The two denials the 256-bit crypto rows are built on: no AVX at all, and the
# AVX1-class target `-mno-avx2` names.
V3_NO_AVX="cmov fxsr mmx sse sse2 sse3 ssse3 sse4.1 sse4.2 popcnt cx16 bmi bmi2 lzcnt movbe"
V3_NO_AVX2="cmov fxsr mmx sse sse2 sse3 ssse3 sse4.1 sse4.2 popcnt cx16 avx fma f16c bmi bmi2 lzcnt movbe"

# expect_set <expected-names|-> <flags...>
expect_set() {
    local expected=$1; shift
    local out
    rm -f probe.bin
    "$CCC" "$@" probe.c -o probe.bin 2>probe.err \
        || { cat probe.err >&2; fail "lccc $* failed"; }
    out=$(./probe.bin)
    [[ $out == "$expected | "* ]] \
        || fail "lccc $*: expected [$expected], got [$out]"
    # "Folded" means no CPUID/XGETBV and no runtime CPU-model helper: the whole
    # probe is a chain of constants.
    "$CCC" "$@" -O2 -S probe.c -o probe.s || fail "lccc $* -S failed"
    ! grep -qE 'cpuid|xgetbv|__cpu_model|__cpu_indicator_init' probe.s \
        || fail "lccc $*: probe is not a compile-time fold (cpuid/__cpu_model present)"
    echo "ok: [$*] -> $out"
}

# 1. No -march: the project baseline is x86-64-v3, and the explicit profile must
#    agree with it. This is the regression that matters: the old table produced
#    the same constants for *every* row below.
expect_set "$V3" -O2
expect_set "$V3" -O2 -march=x86-64-v3
# 2. Explicit levels. v1 is the x86-64 architectural baseline; v2 adds the
#    SSE4.2/popcnt/cx16 rung; neither may leak AVX2/BMI2.
expect_set "cmov fxsr mmx sse sse2" -O2 -march=x86-64
expect_set "cmov fxsr mmx sse sse2 sse3 ssse3 sse4.1 sse4.2 popcnt cx16" \
    -O2 -march=x86-64-v2
# 3. A named CPU profile: Raptor Lake also brings AES/PCLMUL/RDRAND. SHA is
#    deliberately absent — lccc has no SHA codegen permission (`-msha` is
#    rejected), so a dispatcher must not be told otherwise.
expect_set "$V3 aes pclmul rdrnd vaes vpclmulqdq" -O2 -march=raptorlake
# 4. Denials are sticky against the baseline and against -march.
expect_set "$V3_NO_AVX" -O2 -march=x86-64-v3 -mno-avx
expect_set "$V3_NO_AVX2" -O2 -march=x86-64-v3 -mno-avx2
expect_set "cmov fxsr mmx sse sse2 sse3 ssse3 sse4.1 popcnt bmi bmi2 lzcnt movbe" \
    -O2 -march=x86-64-v3 -mno-sse4.2
expect_set "cmov fxsr mmx sse sse2 popcnt bmi bmi2 lzcnt movbe" \
    -O2 -march=x86-64-v3 -mno-sse4.1
expect_set "cmov fxsr popcnt cx16 bmi bmi2 lzcnt movbe" -O2 -mno-sse
expect_set "cmov fxsr mmx sse sse2 sse3 ssse3 sse4.1 sse4.2 popcnt cx16 avx avx2 fma f16c bmi lzcnt movbe" \
    -O2 -mno-bmi2
expect_set "cmov fxsr mmx sse sse2 sse3 ssse3 sse4.1 popcnt cx16 bmi bmi2 lzcnt movbe" \
    -O2 -mno-sse4.2
# 5. `-mno-sse4.1 -msse3`: the later positive flag re-enables the rung it names
#    and nothing above it (the baseline grant must not resurrect the denied
#    SSE4.1/SSE4.2 surface — the pre-rewrite fold reported SSE4.1 here).
expect_set "cmov fxsr mmx sse sse2 sse3 popcnt cx16 bmi bmi2 lzcnt movbe" \
    -O2 -mno-sse4.1 -msse3
# 6. Positive flags on a low -march: -mavx2 carries the SSE ladder and POPCNT
#    (GCC's implication chain) but not BMI/BMI2/LZCNT/MOVBE/FMA/F16C.
expect_set "cmov fxsr mmx sse sse2 sse3 ssse3 sse4.1 sse4.2 popcnt cx16 avx avx2" \
    -O2 -march=x86-64 -mavx2
# 7. Requests the baseline does not grant.
expect_set "$V3 aes" -O2 -maes
expect_set "$V3 rdrnd" -O2 -mrdrnd
expect_set "$V3 aes pclmul" -O2 -maes -mpclmul
# 7b. The 256-bit crypto extensions: granted only by the CONJUNCTION of the
#     class and the flag. The negative rows are the ones that matter -- a
#     dispatcher told `vaes = 1` on a target whose 256-bit class is denied
#     would take a VAES path into a SIGILL.
expect_set "$V3 vaes" -O2 -mvaes
expect_set "$V3 vpclmulqdq" -O2 -mvpclmulqdq
expect_set "$V3 vaes vpclmulqdq" -O2 -mvaes -mvpclmulqdq
expect_set "$V3_NO_AVX2" -O2 -mvaes -mno-avx2
# (`-march=x86-64-v3` is named here so the denial is the only difference from
# the row above it: without an explicit profile, `-mno-avx` also suppresses the
# baseline grant of the SSE4.2-era surface, which would make the row test two
# things at once.)
expect_set "$V3_NO_AVX" -O2 -march=x86-64-v3 -mvpclmulqdq -mno-avx
expect_set "$V3" -O2 -mno-vaes -mno-vpclmulqdq
expect_set "$V3 aes pclmul rdrnd vaes vpclmulqdq" -O2 -march=alderlake
expect_set "$V3 aes pclmul rdrnd" -O2 -march=alderlake -mno-vaes -mno-vpclmulqdq
# 8. Aliases and case folding must resolve (GCC/Clang spellings real code uses).
cat >alias.c <<'C'
int main(void) {
    return __builtin_cpu_supports("pni") != __builtin_cpu_supports("SSE3")
        || __builtin_cpu_supports("sse4_1") != __builtin_cpu_supports("sse4.1")
        || __builtin_cpu_supports("fma3") != __builtin_cpu_supports("fma")
        || __builtin_cpu_supports("pclmulqdq") != __builtin_cpu_supports("pclmul")
        || __builtin_cpu_supports("rdrand") != __builtin_cpu_supports("rdrnd")
        || __builtin_cpu_supports("vpclmul") != __builtin_cpu_supports("vpclmulqdq")
        || __builtin_cpu_supports("avx2") != 1
        || __builtin_cpu_supports("vaes") != 0
        || __builtin_cpu_supports("sha") != 0;
}
C
"$CCC" -O2 alias.c -o alias.bin || fail "alias probe failed to compile"
./alias.bin || fail "aliases disagree with their canonical spellings"

# 9. Unknown target-affecting flags stay errors instead of being silently
#    accepted: `-msha` is how a user would ask for the one feature class the
#    fold deliberately never promises.
if "$CCC" -O2 -msha -S probe.c -o /dev/null 2>msha.err; then
    fail "-msha was accepted; unknown target-affecting flags must be diagnosed"
fi
grep -q "unsupported machine option" msha.err \
    || { cat msha.err >&2; fail "unclear -msha diagnostic"; }

# 10. A non-literal feature name cannot be resolved at compile time. Folding 0
#     in silence would pick the generic path with no explanation, so the
#     compiler diagnoses it with the offending expression's span.
cat >nonconst.c <<'C'
extern const char *feat;
int f(void) { return __builtin_cpu_supports(feat); }
C
if "$CCC" -O0 -S nonconst.c -o nonconst.s 2>nonconst.err; then
    fail "non-literal feature argument was accepted"
fi
grep -q "requires a string literal feature name" nonconst.err \
    || { cat nonconst.err >&2; fail "non-literal argument diagnostic is missing"; }
grep -q "nonconst.c:2:" nonconst.err \
    || { cat nonconst.err >&2; fail "non-literal diagnostic has no source span"; }
! grep -q "internal error" nonconst.err \
    || fail "non-literal argument path panicked"

# 11. Other targets. aarch64/riscv have no x86 feature names at all (GCC does
#     not define the builtin there either), so they must diagnose; i686 has the
#     builtin but none of the x86-64 guarantees, so every tracked feature folds
#     to 0 — the old build answered 111 to a `avx2 + 10*bmi2 + 100*sha` probe
#     on this driver.
# These rows use a header-free probe: a cross target has no aarch64/i686
# sysroot on this host, so `<stdio.h>` (and the host's multiarch `bits/*.h`)
# is the wrong thing to test here — the subject is the builtin, not libc.
cat >cross.c <<'C'
int f(void) {
    int v = 0;
    if (__builtin_cpu_supports("avx2"))  v |= 1;
    if (__builtin_cpu_supports("bmi2"))  v |= 2;
    if (__builtin_cpu_supports("sha"))   v |= 4;
    if (__builtin_cpu_supports("sse2"))  v |= 8;
    return v;
}
C
# The audit's own reproducer shape: `avx2 + 10*bmi2 + 100*sha` folded to 111
# (all three, SHA included) on every target. The truthful answer for a v3-class
# target is 11 — AVX2 and BMI2 are guaranteed, SHA is not.
"$CCC" -O2 -march=raptorlake -S cross.c -o cross_raptor.s \
    || fail "raptorlake cross probe failed"
grep -qE 'movl?[[:space:]]+\$11, %eax' cross_raptor.s \
    || { sed -n '1,40p' cross_raptor.s >&2; fail "audit probe: expected the folded constant 11 (avx2|bmi2|sse2)"; }
echo "ok: audit probe avx2+bmi2+sha -> 11 (sha correctly excluded)"
if [[ -x $CCC_ARM ]]; then
    if "$CCC_ARM" -O2 -S cross.c -o cross_arm.s 2>arm.err; then
        fail "aarch64 target accepted __builtin_cpu_supports (expected a diagnostic)"
    fi
    grep -q "not available on this target architecture" arm.err \
        || { cat arm.err >&2; fail "aarch64 diagnostic is missing or unclear"; }
    grep -q "cross.c:3:" arm.err \
        || { cat arm.err >&2; fail "aarch64 diagnostic has no source span"; }
    ! grep -q "internal error" arm.err || { cat arm.err >&2; fail "aarch64 path panicked"; }
    echo "ok: aarch64 -> diagnostic"
fi
if [[ -x $CCC_I686 ]]; then
    # i686 has the builtin but none of the x86-64 guarantees. There is no 32-bit
    # libgcc on this host, so assert on the folded constant in the assembly: a
    # nonzero bit would have to appear as an immediate and the audit's build
    # emitted every one of them.
    "$CCC_I686" -O2 -S cross.c -o cross_i686.s || fail "i686 probe failed to compile"
    grep -qE '(xorl[[:space:]]+%eax, %eax|movl[[:space:]]+\$0, %eax)' cross_i686.s \
        || { sed -n '1,40p' cross_i686.s >&2; fail "i686 did not fold every x86-64 feature to 0"; }
    ! grep -qE 'cpuid|xgetbv|__cpu_model|__cpu_indicator_init' cross_i686.s \
        || fail "i686 fold is not a compile-time constant"
    ! grep -qE '\$[1-9][0-9]*' cross_i686.s \
        || { sed -n '1,40p' cross_i686.s >&2; fail "i686 claimed an x86-64 feature"; }
    echo "ok: i686 -> no x86-64 guarantees"
fi

echo "PASS: __builtin_cpu_supports folds from the effective codegen permission"
