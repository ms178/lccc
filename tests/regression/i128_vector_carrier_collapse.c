/* `return f()` of a 16-byte vector aggregate must not round-trip the value
 * through a stack staging slot.
 *
 * The I128 call carrier is split into two 64-bit stores (lo at +0, hi at
 * +8) and immediately reassembled by a 16-byte vector load of the same
 * slot.  `aggregate_pair_roundtrip::collapse_carrier_reloads` forwards the
 * carrier straight to the load's users and then deletes the now-unreadable
 * staging stores, taking the measured 12-instruction epilogue down to
 * call + ret.
 *
 * The lane discipline is the trap: on little-endian x86-64 the carrier's
 * bits 63:0 are lane 0 (the +0 store) and bits 127:64 are lane 1 (the +8
 * store).  A collapse that swapped halves, or forwarded a value whose
 * staging slot was also written by something else (the zero-prefill
 * variant h0 below), produces swapped or zeroed lanes here.  Distinct
 * asymmetric values in every 32-bit lane catch both.
 */
#include <immintrin.h>
#include <stdio.h>

__attribute__((noinline)) __m128i mk4(const int *p) {
    __m128i v;
    int *vp = (int *)&v;
    vp[0] = p[0] + 1;
    vp[1] = p[1] + 2;
    vp[2] = p[2] + 3;
    vp[3] = p[3] + 4;
    return v;
}

/* Direct relay: the pure carrier round trip. */
__attribute__((noinline)) __m128i h2(const int *p) { return mk4(p); }

/* Zero-initialized local rewritten by the call result before the return:
 * the staging home sees extra stores, which must either be proven dead or
 * veto the collapse — never half-apply. */
__attribute__((noinline)) __m128i h0(const int *p) {
    __m128i v = _mm_setzero_si128();
    v = mk4(p);
    return v;
}

int main(void) {
    int src[4] = {0x01020304, 0x11223344, -100, 0x7ffffff0};
    int out[4], bad = 0;

    _mm_storeu_si128((__m128i *)out, h2(src));
    if (out[0] != 0x01020305 || out[1] != 0x11223346 || out[2] != -97 ||
        out[3] != 0x7ffffff4)
        bad |= 1;

    _mm_storeu_si128((__m128i *)out, h0(src));
    if (out[0] != 0x01020305 || out[1] != 0x11223346 || out[2] != -97 ||
        out[3] != 0x7ffffff4)
        bad |= 2;

    printf("%08x %08x %08x %08x bad=%d\n", out[0], out[1], out[2], out[3],
           bad);
    return bad;
}
