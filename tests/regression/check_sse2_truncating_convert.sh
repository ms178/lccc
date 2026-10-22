#!/usr/bin/env bash
# `_mm_cvttsd_si32` / `_mm_cvttss_si32` must TRUNCATE regardless of MXCSR.
#
# WHY THIS EXISTS
# ---------------
# The headers used to route both truncating conversions through the *rounding*
# forms:
#
#     _mm_cvttsd_si32(a)  ->  cvtsd2si   (rounds per MXCSR.RC)
#     _mm_cvttss_si32(a)  ->  cvtss2si   (rounds per MXCSR.RC)
#
# Under the default round-to-nearest MXCSR the two agree, so a naive test sees
# nothing wrong.  Under any other rounding mode they disagree in the low bit:
#
#     MXCSR.RC = 1 (round down),   2.7 -> cvtt* gives 2, cvt* gives 2
#     MXCSR.RC = 2 (round up),     2.7 -> cvtt* gives 2, cvt* gives 3   <-- bug
#     MXCSR.RC = 3 (round toward 0), same as default for positive values
#
# C says nothing about MXCSR, but the intrinsic's contract does: `cvttsd2si`
# truncates by definition, and a program that sets the rounding mode (every
# `fesetround` user) must not see the intrinsic change behaviour.  GCC and
# Clang both emit `cvttsd2si`; this gate pins that LCCC does too, by running
# the conversion under all four rounding modes and by checking the encoding.
set -u -o pipefail

repo=$(CDPATH= cd -- "$(dirname "$0")/../.." && pwd)
ccc=${CCC:-$repo/target/fastbuild/lccc}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

fail=0
fail() { echo "FAIL: $*" >&2; fail=1; }

cat >"$tmp/trunc.c" <<'EOF'
#include <emmintrin.h>
#include <stdio.h>

/* Set MXCSR.RC (bits 13:14) to `mode` and return the previous value. */
static unsigned set_rc(unsigned mode)
{
    unsigned old;
    __asm__ volatile("stmxcsr %0" : "=m"(old));
    unsigned updated = (old & ~(3u << 13)) | ((mode & 3u) << 13);
    __asm__ volatile("ldmxcsr %0" : : "m"(updated));
    return old;
}

static void restore(unsigned v)
{
    __asm__ volatile("ldmxcsr %0" : : "m"(v));
}

int main(void)
{
    const double dv[4] = {2.7, -2.7, 0.9, -0.9};
    const float fv[4] = {2.7f, -2.7f, 0.9f, -0.9f};
    int bad = 0;
    for (unsigned mode = 0; mode < 4; mode++) {
        unsigned old = set_rc(mode);
        for (int i = 0; i < 4; i++) {
            __m128d d = _mm_setzero_pd();
            d = _mm_loadu_pd(&dv[i & 1 ? i - 1 : i]);
            int got = _mm_cvttsd_si32(d);
            int want = (int)dv[i & 1 ? i - 1 : i];
            if (got != want) {
                printf("FAIL sd rc=%u %g -> %d (want %d)\n", mode, dv[i & 1 ? i - 1 : i], got, want);
                bad = 1;
            }
            __m128 f = _mm_loadu_ps(&fv[i & 1 ? i - 1 : i]);
            got = _mm_cvttss_si32(f);
            want = (int)fv[i & 1 ? i - 1 : i];
            if (got != want) {
                printf("FAIL ss rc=%u %g -> %d (want %d)\n", mode, (double)fv[i & 1 ? i - 1 : i], got, want);
                bad = 1;
            }
        }
        restore(old);
    }
    /* And the ROUNDING forms must still round: this is what proves the two
       families were not merged into one implementation. */
    unsigned old = set_rc(2); /* round up */
    __m128d d = _mm_loadu_pd((const double[]){2.7, 0.0});
    int rounded = _mm_cvtsd_si32(d);
    restore(old);
    if (rounded != 3) {
        printf("FAIL _mm_cvtsd_si32 must round per MXCSR (round-up 2.7 -> %d)\n", rounded);
        bad = 1;
    }
    /* 64-bit forms: the ROUNDING conversions must round per MXCSR and must
       keep the full range. A tie (`2.5`) rounds to even under the default
       mode, which is what distinguishes `cvtsd2si` from the old
       `d +/- 0.5` composition (that gave 3). */
    {
        __m128d t = _mm_set_sd(2.5);
        __m128d n = _mm_set_sd(-2.5);
        __m128d big = _mm_set_sd(4503599627370496.5); /* 2^52 + 0.5 */
        long long a = _mm_cvtsd_si64(t);
        long long b = _mm_cvtsd_si64(n);
        long long c = _mm_cvtsd_si64(big);
        if (a != 2 || b != -2 || c != 4503599627370496LL) {
            printf("FAIL _mm_cvtsd_si64 round-per-MXCSR: %lld %lld %lld\n", a, b, c);
            bad = 1;
        }
        __m128 fs = _mm_set_ss(2.7f);
        if (_mm_cvtss_si64(fs) != 3) {
            printf("FAIL _mm_cvtss_si64 must round (2.7f -> %lld)\n", _mm_cvtss_si64(fs));
            bad = 1;
        }
        /* ...and the truncating 64-bit forms must NOT round. */
        if (_mm_cvttsd_si64(_mm_set_sd(2.7)) != 2) {
            printf("FAIL _mm_cvttsd_si64 must truncate\n");
            bad = 1;
        }
        if (_mm_cvttss_si64(_mm_set_ss(2.7f)) != 2) {
            printf("FAIL _mm_cvttss_si64 must truncate\n");
            bad = 1;
        }
    }
    if (!bad) {
        puts("trunc-ok");
    }
    return bad;
}
EOF

# 1. Encoding: the truncating forms must be `cvtt*2si`, not `cvt*2si`.
if ! "$ccc" -O2 -S -o "$tmp/trunc.s" "$tmp/trunc.c" 2>"$tmp/err"; then
    fail "lccc -S failed: $(cat "$tmp/err")"
else
    for mn in cvttsd2si cvttss2si cvtsd2si cvtss2si; do
        if ! grep -qE "^\s+$mn\b" "$tmp/trunc.s"; then
            fail "the emitted assembly has no '$mn' — the truncating form was not used"
        fi
    done
    # Both families must be present: the source deliberately exercises the
    # rounding forms too, so a count ratio is not the invariant -- the
    # per-mode runtime checks below are.
    if ! grep -qE '^\s+cvt(sd|ss)2si\b' "$tmp/trunc.s"; then
        fail "the emitted assembly has no rounding conversion (cvt*2si) at all"
    fi
fi

# 2. Behaviour under every rounding mode.
if ! "$ccc" -O2 -o "$tmp/lccc.exe" "$tmp/trunc.c" 2>"$tmp/err"; then
    fail "lccc link failed: $(cat "$tmp/err")"
else
    out=$("$tmp/lccc.exe") || fail "binary failed: $out"
    case "$out" in
        *trunc-ok*) ;;
        *) fail "runtime check failed:
$out" ;;
    esac
    gcc_bin=${GCC:-gcc-16}
    if command -v "$gcc_bin" >/dev/null 2>&1; then
        "$gcc_bin" -O2 -o "$tmp/gcc.exe" "$tmp/trunc.c" 2>"$tmp/err" &&
            want=$("$tmp/gcc.exe") || want=""
        if [ -n "$want" ] && [ "$want" != "$out" ]; then
            fail "differential mismatch against $gcc_bin:
  lccc:  $out
  $gcc_bin: $want"
        fi
    fi
fi

if [ "$fail" -ne 0 ]; then
    echo "sse2 truncating-conversion gate: FAIL" >&2
    exit 1
fi
echo "sse2 truncating-conversion gate: PASS"
