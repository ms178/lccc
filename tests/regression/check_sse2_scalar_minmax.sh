#!/usr/bin/env bash
# End-to-end gate for scalar SSE2 min/max: `_mm_max_sd` / `_mm_min_sd` must
# lower to the single hardware instructions `maxsd` / `minsd`, with the
# hardware semantics bit-for-bit.
#
# WHY THIS EXISTS
# ---------------
# These two intrinsics spent a long time as the largest per-row gap in the
# SSE2 codegen matrix (84 instructions vs the reference compiler's 11)
# because they were spelled as compare+select in a header.  The native
# intrinsic path fixed the lowering, and this gate exists so the fix can
# never silently regress: it asserts on the *assembly* (one `maxsd` /
# `minsd` in each kernel body, no calls) and on the *bits*.
#
# SOUNDNESS CONTRACT PINNED HERE (Intel SDM, MAXSD/MINSD)
#   * low lane: MAXSD returns the second (source) operand unless the first
#     is strictly greater -- so NaN in either operand yields the second
#     operand, equal operands yield the second, and signed-zero ties take
#     the second operand's sign.  MINSD is the mirror image.  C's ternary
#     `(a > b) ? a : b` reproduces exactly this, because every comparison
#     with a NaN is false; the test program uses it as the oracle and
#     compares bit patterns, not values, so -0.0 vs +0.0 is observable.
#   * high lane of the destination is preserved from the first operand.
#   * the `set_sd` / `cvtsd` scalar round-trip agrees with the vector form.
set -u -o pipefail

repo=$(CDPATH= cd -- "$(dirname "$0")/../.." && pwd)
ccc=${CCC:-$repo/target/fastbuild/lccc}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

fail=0
fail() { echo "FAIL: $*" >&2; fail=1; }

cat > "$tmp/minmax.c" <<'EOF'
#include <emmintrin.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

__attribute__((noinline)) __m128d mx(__m128d a, __m128d b) { return _mm_max_sd(a, b); }
__attribute__((noinline)) __m128d mn(__m128d a, __m128d b) { return _mm_min_sd(a, b); }

__attribute__((noinline)) double smax(double x, double y)
{
    return _mm_cvtsd_f64(_mm_max_sd(_mm_set_sd(x), _mm_set_sd(y)));
}
__attribute__((noinline)) double smin(double x, double y)
{
    return _mm_cvtsd_f64(_mm_min_sd(_mm_set_sd(x), _mm_set_sd(y)));
}

static double dfrom(uint64_t u) { double d; memcpy(&d, &u, 8); return d; }
static uint64_t dbits(double d) { uint64_t u; memcpy(&u, &d, 8); return u; }

int main(void)
{
    const double nanv = dfrom(0x7ff8000000000000ull);
    const double infv = dfrom(0x7ff0000000000000ull);
    const struct { double a, b; } cases[] = {
        {2.5, 1.5}, {1.5, 2.5}, {2.5, 2.5}, {-3.25, -7.5}, {-7.5, -3.25},
    };
    const double extra[][2] = {
        {-0.0, 0.0}, {0.0, -0.0},
    };
    int bad = 0;
    const double hi_sentinel = 1234.5;

    for (unsigned i = 0; i < sizeof(cases) / sizeof(cases[0]); i++) {
        double a = cases[i].a, b = cases[i].b, out[2];
        _mm_storeu_pd(out, mx(_mm_set_pd(hi_sentinel, a), _mm_set_pd(-987.25, b)));
        double want = (a > b) ? a : b;
        if (dbits(out[0]) != dbits(want) || dbits(out[1]) != dbits(hi_sentinel)) {
            printf("FAIL max_sd %g %g -> %g/%g want %g\n", a, b, out[0], out[1], want);
            bad = 1;
        }
        _mm_storeu_pd(out, mn(_mm_set_pd(hi_sentinel, a), _mm_set_pd(-987.25, b)));
        want = (a < b) ? a : b;
        if (dbits(out[0]) != dbits(want) || dbits(out[1]) != dbits(hi_sentinel)) {
            printf("FAIL min_sd %g %g -> %g/%g want %g\n", a, b, out[0], out[1], want);
            bad = 1;
        }
    }
    for (unsigned i = 0; i < sizeof(extra) / sizeof(extra[0]); i++) {
        double a = extra[i][0], b = extra[i][1], out[2];
        _mm_storeu_pd(out, mx(_mm_set_sd(a), _mm_set_sd(b)));
        if (dbits(out[0]) != dbits(b)) { /* tie: second operand wins, sign included */
            printf("FAIL max_sd zero-tie bits %llx want %llx\n",
                   (unsigned long long)dbits(out[0]), (unsigned long long)dbits(b));
            bad = 1;
        }
        _mm_storeu_pd(out, mn(_mm_set_sd(a), _mm_set_sd(b)));
        if (dbits(out[0]) != dbits(b)) {
            printf("FAIL min_sd zero-tie bits %llx want %llx\n",
                   (unsigned long long)dbits(out[0]), (unsigned long long)dbits(b));
            bad = 1;
        }
    }
    /* NaN and infinity, via the scalar set_sd/cvtsd round-trip. */
    const double av[6] = {7.0, nanv, nanv, infv, -infv, 1e308};
    const double bv[6] = {nanv, 7.0, nanv, 1e308, -1e308, infv};
    for (unsigned i = 0; i < 6; i++) {
        double a = av[i], b = bv[i];
        double want_max = (a > b) ? a : b; /* false under NaN -> b, per MAXSD */
        double want_min = (a < b) ? a : b;
        if (dbits(smax(a, b)) != dbits(want_max)) {
            printf("FAIL smax bits %llx want %llx\n",
                   (unsigned long long)dbits(smax(a, b)),
                   (unsigned long long)dbits(want_max));
            bad = 1;
        }
        if (dbits(smin(a, b)) != dbits(want_min)) {
            printf("FAIL smin bits %llx want %llx\n",
                   (unsigned long long)dbits(smin(a, b)),
                   (unsigned long long)dbits(want_min));
            bad = 1;
        }
    }
    if (!bad)
        printf("scalar-minmax OK\n");
    return bad;
}
EOF

# --- instruction selection: exactly the hardware op in each kernel body ---
if ! "$ccc" -O2 -S "$tmp/minmax.c" -o "$tmp/minmax.s" 2>"$tmp/err"; then
    fail "lccc -S failed: $(cat "$tmp/err")"
else
    for spec in "mx maxsd" "mn minsd"; do
        set -- $spec
        body=$(awk "/^$1:/{f=1} f{print} f&&/\\.size/{exit}" "$tmp/minmax.s")
        [ -n "$body" ] || { fail "function $1 not found in assembly"; continue; }
        printf '%s\n' "$body" | grep -q "$2" || fail "$1 body has no $2 instruction"
        printf '%s\n' "$body" | grep -q "call" && fail "$1 body contains a call"
    done
fi

# --- runtime semantics, bit-exact ---
if ! "$ccc" -O2 "$tmp/minmax.c" -o "$tmp/lccc.exe" 2>"$tmp/err"; then
    fail "lccc compile failed: $(cat "$tmp/err")"
elif ! out=$("$tmp/lccc.exe"); then
    fail "lccc binary failed: $out"
else
    [ "$out" = "scalar-minmax OK" ] || fail "lccc output: $out"
    gcc_bin=${GCC:-gcc-16}
    if command -v "$gcc_bin" >/dev/null 2>&1; then
        if ! "$gcc_bin" -O2 "$tmp/minmax.c" -o "$tmp/gcc.exe" 2>"$tmp/err"; then
            fail "oracle compile failed: $(cat "$tmp/err")"
        elif ! want=$("$tmp/gcc.exe"); then
            fail "oracle execution failed: $want"
        elif [ "$want" != "$out" ]; then
            fail "differential mismatch against $gcc_bin: lccc='$out' gcc='$want'"
        fi
    fi
fi

[ "$fail" -ne 0 ] && exit 1
printf 'sse2 scalar minmax: ok\n'
