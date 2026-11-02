/* Regression: const-qualified __int128 locals must not be folded through an
 * i64-sized cache.
 *
 * The lowering cached every const-qualified local's initializer in an
 * i64-keyed table (`const_local_values`). A 128-bit initializer was
 * truncated to its low half on the way in (`IrConst::I128(v).to_i64()` =
 * `v as i64`), and every later read of the variable folded to that half:
 *
 *   const unsigned __int128 c = ((unsigned __int128)3 << 100) | 3;
 *   (unsigned long long)(c >> 64)
 *
 * folded to 3i64.wrapping_shr(64 & 63 == 0) == 3 — the HIGH half read the
 * LOW value and the shift amount masked to zero. GCC prints the full
 * high half (3 << 36 == 206158430208); the truncated fold cannot.
 *
 * The fix refuses to cache initializers whose value cannot round-trip
 * through the cache bit-exactly; reads then take the (correct) alloca
 * path. The boundary values below cover: high-only bits (1 << 64), a
 * nonzero low AND high half, all-ones halves, the top sign bit, and a
 * 128-bit value that DOES fit in 63 bits (which stays cacheable — its
 * fold must keep matching the memory path bit for bit).
 */
#include <stdio.h>

static unsigned failures;

static void check(const char *what, unsigned long long lo, unsigned long long hi,
                  unsigned long long elo, unsigned long long ehi) {
    if (lo != elo || hi != ehi) {
        printf("FAIL %s: got %016llx%016llx want %016llx%016llx\n", what, hi, lo, ehi, elo);
        failures++;
    } else {
        printf("ok %s %016llx%016llx\n", what, hi, lo);
    }
}

/* Sink so the values are observable and the compiler cannot drop them. */
static unsigned long long sink_lo, sink_hi;

static void emit(const char *what, unsigned __int128 v,
                 unsigned long long elo, unsigned long long ehi) {
    check(what, (unsigned long long)v, (unsigned long long)(v >> 64), elo, ehi);
}

int main(void) {
    /* The original repro: nonzero low AND high half, high half needs 37 bits. */
    const unsigned __int128 c = ((unsigned __int128)3 << 100) | 3;
    emit("mixed", c, 3, 3ULL << 36);

    /* High half only: the old fold returned the low half's 0 shifted... and
     * `1 << 64` itself folded through i64 as 0 — check both halves. */
    const unsigned __int128 h = (unsigned __int128)1 << 64;
    emit("high-only", h, 0, 1);

    /* All-ones low half: truncating to i64 yields -1; any sign-extending
     * widening of the cached value into the unsigned 128-bit domain gives
     * 0xFFFF...FFFF_FFFFFFFF... instead of the correct 0x00000000FFFFFFFF. */
    const unsigned __int128 lo32 = ((unsigned __int128)1 << 64) | 0xFFFFFFFFULL;
    emit("ones-lo32", lo32, 0xFFFFFFFFULL, 1);

    /* Top sign bit: i128-representable only, far outside the i64 cache. */
    const unsigned __int128 top = (unsigned __int128)1 << 127;
    emit("top-bit", top, 0, 0x8000000000000000ULL);

    /* All 128 bits set (unsigned reading of -1). */
    const unsigned __int128 ones = ~(unsigned __int128)0;
    emit("all-ones", ones, 0xFFFFFFFFFFFFFFFFULL, 0xFFFFFFFFFFFFFFFFULL);

    /* A value that FITS in 63 bits stays cacheable; its folded reads must
     * still match the C semantics exactly. (5 << 32) | 7 = 0x500000007 —
     * entirely inside the LOW half; the high half is 0. */
    const unsigned __int128 small = ((unsigned __int128)5 << 32) | 7;
    emit("small", small, 0x500000007ULL, 0);

    /* Shifts in BOTH directions over an uncached const local, including the
     * shift-amount-masking trap (>= 64 on an i64-carried fold).
     * s = 0x1234_00000000_00005678: s >> 64 = 0x1234; (s>>64) >> 32 = 0.
     * The shl trap: (u64)(s << 64) is the low half of the shifted value = 0,
     * and 0 >> 32 stays 0 — the masking bug used to produce the HIGH half's
     * value here instead. */
    const unsigned __int128 s = ((unsigned __int128)0x1234 << 96) | 0x5678;
    unsigned long long sh_lo = (unsigned long long)(s >> 64);
    unsigned long long sh_hi = (unsigned long long)((s >> 64) >> 32);
    unsigned long long sl_lo = (unsigned long long)(s << 64) >> 32;
    check("shr64", sh_lo, sh_hi, 0x123400000000ULL, 0x1234ULL);
    check("shl-trap", sl_lo, 0, 0, 0);
    sink_lo = sh_lo; sink_hi = sh_hi;

    if (failures) {
        return 1;
    }
    printf("PASS i128 const-local half extraction\n");
    return 0;
}
