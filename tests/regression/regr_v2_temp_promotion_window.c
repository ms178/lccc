/* Regression (v2): vector_temp_promotion redirected an intrinsic's result
 * store from a temp slot into the memcpy destination slot and dropped the
 * copy — WITHOUT checking whether that destination slot is read or written
 * in between (the "window"). The narrow-loop sequence
 *   vs1 = sad + vs1; ...; vs3 = vs1_0 + vs3; vs1_0 = vs1
 * redirected the vs1 update into vs1_0's slot; vs3 then read the NEW value
 * too early -> wrong adler32 (vector_defer_multidef_slot regression).
 * Fix: no_var_access_between(intrinsic, memcpy, dest) window check. */
#include <immintrin.h>
#include <stdint.h>

static uint32_t narrow_chain(uint32_t adler, const uint8_t *buf) {
    uint32_t sum2 = 0;
    __m128i vs1 = _mm_cvtsi32_si128((int)adler);
    __m128i vs1_0 = vs1;
    __m128i vs3 = _mm_setzero_si128();
    __m128i zero = _mm_setzero_si128();

    for (int k = 0; k < 32; k += 16) {
        __m128i vbuf = _mm_loadu_si128((const __m128i *)(buf + k));
        __m128i sad = _mm_sad_epu8(vbuf, zero);
        vs1 = _mm_add_epi32(sad, vs1);      /* def of new vs1 (temp) */
        vs3 = _mm_add_epi32(vs1_0, vs3);    /* reads vs1_0 in the window */
        vs1_0 = vs1;                        /* memcpy vs1_0 <- tmp(vs1) */
    }
    /* Reference: vs3 = old_vs1_0 (adler) + new_vs1_0 each iteration. With
     * correct ordering (vs3 reads vs1_0 BEFORE vs1_0 = vs1), the first
     * iteration adds adler, the second adds adler + sad(buf[0..15]).
     * The pre-fix promotion redirect wrote the new vs1 into vs1_0's slot
     * early, so the second iteration read adler + sad -> different result. */
    uint32_t sad_sum = 0;
    for (int i = 0; i < 16; i++) sad_sum += buf[i];
    uint32_t want = adler + (adler + sad_sum);
    uint32_t got = (uint32_t)_mm_cvtsi128_si32(_mm_add_epi32(vs3, _mm_srli_si128(vs3, 8)));
    return got == want ? 0 : 1;
}

int main(void) {
    uint8_t buf[32];
    for (int i = 0; i < 32; i++) buf[i] = (uint8_t)(i * 7);
    return narrow_chain(1000, buf);
}
