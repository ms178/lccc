#!/usr/bin/env bash
# Pre-fix stand-in compiler for the lane-mask-select gate.
#
# `check_blendv_lane_semantics.sh --expect-defect` must go RED.  A gate that
# cannot fail is decoration, but the repository has no way to check out a
# pre-fix LCCC, so this wrapper manufactures the exact defect SA-10 describes
# and nothing else: on a NON-AVX target it redefines the FP lane-mask select
# to compute the PER-BIT bitwise reading (`(a & ~m) | (b & m)`), which is what
# `andps/andnps/orps` does; on an AVX target it forwards untouched, so the
# same source computes the PER-LANE sign-bit reading.  The gate's invariance
# check therefore sees two different answers for the same row and reports red.
#
# That is deliberately a *single-line, single-behaviour* change: if the gate
# stays green against this wrapper, the gate is not testing the lane
# semantics at all.  It never runs as part of a normal build.
#
# Not a compiler in its own right -- it delegates to $REAL_CC (default `cc`).
set -euo pipefail

REAL_CC="${REAL_CC:-cc}"

# Forward the compiler mode probes (`--version`, `-dumpmachine`, ...) so a
# caller that asks the wrapper about itself gets a real answer.
if [ $# -eq 0 ]; then
    exec "$REAL_CC"
fi
for arg in "$@"; do
    case "$arg" in
        -mavx*|-march=*) exec "$REAL_CC" "$@" ;;
    esac
done

hdr="$(mktemp --suffix=.h)"
trap 'rm -f "$hdr"' EXIT
cat >"$hdr" <<'HEADER'
/* Pre-fix SA-10 semantics: a PER-BIT select, not a per-lane sign select. */
#include <immintrin.h>
static __inline__ __m128 sa10_prefix_ps(__m128 a, __m128 b, __m128 m) {
    __m128i ai = _mm_castps_si128(a);
    __m128i bi = _mm_castps_si128(b);
    __m128i mi = _mm_castps_si128(m);
    __m128i inv = _mm_xor_si128(mi, _mm_set1_epi32(-1));
    return _mm_castsi128_ps(_mm_or_si128(_mm_and_si128(ai, inv), _mm_and_si128(bi, mi)));
}
static __inline__ __m128d sa10_prefix_pd(__m128d a, __m128d b, __m128d m) {
    __m128i ai = _mm_castpd_si128(a);
    __m128i bi = _mm_castpd_si128(b);
    __m128i mi = _mm_castpd_si128(m);
    __m128i inv = _mm_xor_si128(mi, _mm_set1_epi32(-1));
    return _mm_castsi128_pd(_mm_or_si128(_mm_and_si128(ai, inv), _mm_and_si128(bi, mi)));
}
#define _mm_blendv_ps(a, b, m) sa10_prefix_ps((a), (b), (m))
#define _mm_blendv_pd(a, b, m) sa10_prefix_pd((a), (b), (m))
HEADER

exec "$REAL_CC" "$@" -include "$hdr"
