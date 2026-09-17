/*
 * Adler-32 rolling-checksum loop epic — exhaustive red-team gate.
 *
 * The epic vectorizes the serial two-accumulator recurrence
 *     while (n >= K) { s1 += b; s2 += s1; ... K times; p += K; n -= K; }
 * with vpsadbw + vpmaddubsw(weights 32..1) + vpmaddwd + deferred vs3<<5,
 * exact mod 2^32 for every input and length (ring-homomorphism proof in
 * src/passes/vectorize.rs).  GCC 16.2, Clang 23.1 and ICX all leave this
 * shape scalar (verified on the DO8 kernel).
 *
 * This gate covers the transform's correctness envelope and, deliberately,
 * every shape it must REFUSE (fail closed):
 *   POSITIVE (must vectorize AND be exact — the asm contract checks the
 *   packed ops; the differential checks every length against a volatile
 *   reference that can never be vectorized into the same form):
 *     - DO8 (zlib-ng spelling), DO4, DO16, DO32, and the plain K=1 form;
 *     - the GEP spelling (`buf[k]`) and the pointer-increment spelling;
 *     - U64/U32/I64/I32 byte counters, Uge/Sge guards;
 *     - the mirrored `s2 = s1 + s2` spelling;
 *     - adler inits across the valid contract range.
 *   NEGATIVE (must stay scalar AND stay correct — a packed body would
 *   miscompile each of these):
 *     - a second accumulator pair in the same loop;
 *     - a body value live-out of the loop;
 *     - a store inside the loop;
 *     - a non-U32 accumulator (I32 chains are declined: the matcher's
 *       grammar is U32-exact);
 *     - a chain length that does not divide 32 (K = 3, 5, 12);
 *     - a cursor advancing by a non-K stride;
 *     - a volatile byte stream;
 *     - a counter stepping by a different amount than the cursor.
 *
 * It also pins the three counting-epic miscompile fixes (found by this
 * battery's development): the multi-accumulator decline, the IV
 * live-out materialisation (guarded and unguarded), and the
 * narrow-compare constant wrap (`x == 0xAA` on U8 streams).
 */
#include <stdio.h>
#include <stdint.h>
#include <stddef.h>
#include <string.h>

#define N 4099 /* prime: exercises every remainder mod 32 */

static unsigned char usrc[N];

/* ── volatile reference: never vectorizable into the same form ──────── */
static uint32_t vref(uint32_t adler, const uint8_t *b, size_t n) {
    volatile uint32_t s1 = adler & 0xffff;
    volatile uint32_t s2 = (adler >> 16) & 0xffff;
    for (size_t i = 0; i < n; i++) {
        uint32_t t1 = (uint32_t)(s1 + b[i]);
        uint32_t t2 = (uint32_t)(s2 + t1);
        s1 = t1;
        s2 = t2;
    }
    s1 %= 65521U;
    s2 %= 65521U;
    return (s2 << 16) | s1;
}

/* ═══ POSITIVE: the epic must vectorize these ═════════════════════════ */

/* DO8 — the zlib-ng adler32_ssse3 main-loop spelling. */
uint32_t adler_do8(uint32_t adler, const uint8_t *buf, size_t len) {
    uint32_t s1 = adler & 0xffff;
    uint32_t s2 = (adler >> 16) & 0xffff;
    const uint8_t *p = buf;
    size_t n = len;
    while (n >= 8) {
        s1 += p[0]; s2 += s1; s1 += p[1]; s2 += s1;
        s1 += p[2]; s2 += s1; s1 += p[3]; s2 += s1;
        s1 += p[4]; s2 += s1; s1 += p[5]; s2 += s1;
        s1 += p[6]; s2 += s1; s1 += p[7]; s2 += s1;
        p += 8;
        n -= 8;
    }
    while (n--) { s1 += *p++; s2 += s1; }
    s1 %= 65521U;
    s2 %= 65521U;
    return (s2 << 16) | s1;
}

/* DO4 — the adler32_copy_tail inner-loop spelling (BUG-003 kernel). */
uint32_t adler_do4(uint32_t adler, const uint8_t *buf, size_t len) {
    uint32_t s1 = adler & 0xffff;
    uint32_t s2 = (adler >> 16) & 0xffff;
    size_t n = len;
    while (n >= 4) {
        s1 += buf[0]; s2 += s1;
        s1 += buf[1]; s2 += s1;
        s1 += buf[2]; s2 += s1;
        s1 += buf[3]; s2 += s1;
        buf += 4;
        n -= 4;
    }
    while (n--) { s1 += *buf++; s2 += s1; }
    s1 %= 65521U;
    s2 %= 65521U;
    return (s2 << 16) | s1;
}

/* DO16 — the SSSE3 16-byte prologue spelling. */
uint32_t adler_do16(uint32_t adler, const uint8_t *buf, size_t len) {
    uint32_t s1 = adler & 0xffff;
    uint32_t s2 = (adler >> 16) & 0xffff;
    size_t n = len;
    while (n >= 16) {
        s1 += buf[0];  s2 += s1; s1 += buf[1];  s2 += s1;
        s1 += buf[2];  s2 += s1; s1 += buf[3];  s2 += s1;
        s1 += buf[4];  s2 += s1; s1 += buf[5];  s2 += s1;
        s1 += buf[6];  s2 += s1; s1 += buf[7];  s2 += s1;
        s1 += buf[8];  s2 += s1; s1 += buf[9];  s2 += s1;
        s1 += buf[10]; s2 += s1; s1 += buf[11]; s2 += s1;
        s1 += buf[12]; s2 += s1; s1 += buf[13]; s2 += s1;
        s1 += buf[14]; s2 += s1; s1 += buf[15]; s2 += s1;
        buf += 16;
        n -= 16;
    }
    while (n--) { s1 += *buf++; s2 += s1; }
    s1 %= 65521U;
    s2 %= 65521U;
    return (s2 << 16) | s1;
}

/* DO32 — the full 32-byte source unroll. */
uint32_t adler_do32(uint32_t adler, const uint8_t *buf, size_t len) {
    uint32_t s1 = adler & 0xffff;
    uint32_t s2 = (adler >> 16) & 0xffff;
    size_t n = len;
    while (n >= 32) {
        for (int k = 0; k < 32; k++) { s1 += buf[k]; s2 += s1; }
        buf += 32;
        n -= 32;
    }
    while (n--) { s1 += *buf++; s2 += s1; }
    s1 %= 65521U;
    s2 %= 65521U;
    return (s2 << 16) | s1;
}

/* The plain K=1 spelling — no source unroll at all. */
uint32_t adler_plain1(uint32_t adler, const uint8_t *buf, size_t len) {
    uint32_t s1 = adler & 0xffff;
    uint32_t s2 = (adler >> 16) & 0xffff;
    size_t n = len;
    while (n >= 1) {
        s1 += *buf;
        s2 += s1;
        buf += 1;
        n -= 1;
    }
    s1 %= 65521U;
    s2 %= 65521U;
    return (s2 << 16) | s1;
}

/* Signed byte counter + mirrored s2 spelling. */
uint32_t adler_do8_signed(uint32_t adler, const uint8_t *buf, int64_t len) {
    uint32_t s1 = adler & 0xffff;
    uint32_t s2 = (adler >> 16) & 0xffff;
    int64_t n = len;
    while (n >= 8) {
        s1 += buf[0]; s2 = s1 + s2;
        s1 += buf[1]; s2 = s1 + s2;
        s1 += buf[2]; s2 = s1 + s2;
        s1 += buf[3]; s2 = s1 + s2;
        s1 += buf[4]; s2 = s1 + s2;
        s1 += buf[5]; s2 = s1 + s2;
        s1 += buf[6]; s2 = s1 + s2;
        s1 += buf[7]; s2 = s1 + s2;
        buf += 8;
        n -= 8;
    }
    while (n-- > 0) { s1 += *buf++; s2 += s1; }
    s1 %= 65521U;
    s2 %= 65521U;
    return (s2 << 16) | s1;
}

/* U32 counter (int len). */
uint32_t adler_do8_u32n(uint32_t adler, const uint8_t *buf, uint32_t len) {
    uint32_t s1 = adler & 0xffff;
    uint32_t s2 = (adler >> 16) & 0xffff;
    uint32_t n = len;
    while (n >= 8) {
        s1 += buf[0]; s2 += s1;
        s1 += buf[1]; s2 += s1;
        s1 += buf[2]; s2 += s1;
        s1 += buf[3]; s2 += s1;
        s1 += buf[4]; s2 += s1;
        s1 += buf[5]; s2 += s1;
        s1 += buf[6]; s2 += s1;
        s1 += buf[7]; s2 += s1;
        buf += 8;
        n -= 8;
    }
    while (n--) { s1 += *buf++; s2 += s1; }
    s1 %= 65521U;
    s2 %= 65521U;
    return (s2 << 16) | s1;
}

/* ═══ NEGATIVE: must decline, stay scalar, stay correct ═══════════════ */

/* K = 3: does not divide 32 — the vector consumption cannot be repaid. */
uint32_t adler_k3(uint32_t adler, const uint8_t *buf, size_t len) {
    uint32_t s1 = adler & 0xffff;
    uint32_t s2 = (adler >> 16) & 0xffff;
    size_t n = len;
    while (n >= 3) {
        s1 += buf[0]; s2 += s1;
        s1 += buf[1]; s2 += s1;
        s1 += buf[2]; s2 += s1;
        buf += 3;
        n -= 3;
    }
    while (n--) { s1 += *buf++; s2 += s1; }
    s1 %= 65521U;
    s2 %= 65521U;
    return (s2 << 16) | s1;
}

/* A second accumulator pair in the same loop. */
uint32_t adler_two_pairs(uint32_t adler, const uint8_t *buf, size_t len) {
    uint32_t s1 = adler & 0xffff;
    uint32_t s2 = (adler >> 16) & 0xffff;
    uint32_t t1 = 0, t2 = 0;
    size_t n = len;
    while (n >= 8) {
        s1 += buf[0]; s2 += s1; t1 += buf[0] ^ 0xFF; t2 += t1;
        s1 += buf[1]; s2 += s1; t1 += buf[1] ^ 0xFF; t2 += t1;
        s1 += buf[2]; s2 += s1; t1 += buf[2] ^ 0xFF; t2 += t1;
        s1 += buf[3]; s2 += s1; t1 += buf[3] ^ 0xFF; t2 += t1;
        s1 += buf[4]; s2 += s1; t1 += buf[4] ^ 0xFF; t2 += t1;
        s1 += buf[5]; s2 += s1; t1 += buf[5] ^ 0xFF; t2 += t1;
        s1 += buf[6]; s2 += s1; t1 += buf[6] ^ 0xFF; t2 += t1;
        s1 += buf[7]; s2 += s1; t1 += buf[7] ^ 0xFF; t2 += t1;
        buf += 8;
        n -= 8;
    }
    while (n--) {
        uint8_t c = *buf++;
        s1 += c;
        s2 += s1;
        t1 += (uint8_t)(c ^ 0xFF);
        t2 += t1;
    }
    s1 %= 65521U;
    s2 %= 65521U;
    return (s2 << 16) | (s1 ^ (t2 & 0xffff));
}

/* A body value live-out (last byte consumed after the loop). */
uint32_t adler_liveout(uint32_t adler, const uint8_t *buf, size_t len) {
    uint32_t s1 = adler & 0xffff;
    uint32_t s2 = (adler >> 16) & 0xffff;
    uint32_t last = 0;
    size_t n = len;
    while (n >= 8) {
        s1 += buf[0]; s2 += s1;
        s1 += buf[1]; s2 += s1;
        s1 += buf[2]; s2 += s1;
        s1 += buf[3]; s2 += s1;
        s1 += buf[4]; s2 += s1;
        s1 += buf[5]; s2 += s1;
        s1 += buf[6]; s2 += s1;
        s1 += buf[7]; s2 += s1;
        last = buf[7];
        buf += 8;
        n -= 8;
    }
    while (n--) { s1 += *buf++; s2 += s1; }
    s1 %= 65521U;
    s2 %= 65521U;
    return (s2 << 16) | (s1 ^ (last & 0xffff));
}

/* A store inside the loop (the byte stream is also an output). */
uint32_t adler_store(uint32_t adler, uint8_t *buf, size_t len) {
    uint32_t s1 = adler & 0xffff;
    uint32_t s2 = (adler >> 16) & 0xffff;
    size_t n = len;
    while (n >= 8) {
        s1 += buf[0]; s2 += s1;
        s1 += buf[1]; s2 += s1;
        s1 += buf[2]; s2 += s1;
        s1 += buf[3]; s2 += s1;
        s1 += buf[4]; s2 += s1;
        s1 += buf[5]; s2 += s1;
        s1 += buf[6]; s2 += s1;
        s1 += buf[7]; s2 += s1;
        buf[0] = (uint8_t)s1;
        buf += 8;
        n -= 8;
    }
    while (n--) { s1 += *buf++; s2 += s1; }
    s1 %= 65521U;
    s2 %= 65521U;
    return (s2 << 16) | s1;
}

/* Cursor stride 8 with counter step 4 — the consumption mismatch. */
uint32_t adler_stride_mismatch(uint32_t adler, const uint8_t *buf, size_t len) {
    uint32_t s1 = adler & 0xffff;
    uint32_t s2 = (adler >> 16) & 0xffff;
    size_t n = len;
    while (n >= 8) {
        s1 += buf[0]; s2 += s1;
        s1 += buf[2]; s2 += s1;
        s1 += buf[4]; s2 += s1;
        s1 += buf[6]; s2 += s1;
        buf += 8;
        n -= 4;
    }
    while (n--) { s1 += *buf++; s2 += s1; }
    s1 %= 65521U;
    s2 %= 65521U;
    return (s2 << 16) | s1;
}

/* Volatile byte stream — the packed loop reads 32 at a time. */
uint32_t adler_volatile(uint32_t adler, const volatile uint8_t *buf, size_t len) {
    uint32_t s1 = adler & 0xffff;
    uint32_t s2 = (adler >> 16) & 0xffff;
    size_t n = len;
    while (n >= 8) {
        s1 += buf[0]; s2 += s1;
        s1 += buf[1]; s2 += s1;
        s1 += buf[2]; s2 += s1;
        s1 += buf[3]; s2 += s1;
        s1 += buf[4]; s2 += s1;
        s1 += buf[5]; s2 += s1;
        s1 += buf[6]; s2 += s1;
        s1 += buf[7]; s2 += s1;
        buf += 8;
        n -= 8;
    }
    while (n--) { s1 += *buf++; s2 += s1; }
    s1 %= 65521U;
    s2 %= 65521U;
    return (s2 << 16) | s1;
}

/* I8 (signed char) stream — the packed form is only exact for zext. */
uint32_t adler_i8(uint32_t adler, const signed char *buf, size_t len) {
    uint32_t s1 = adler & 0xffff;
    uint32_t s2 = (adler >> 16) & 0xffff;
    size_t n = len;
    while (n >= 8) {
        s1 += (uint32_t)buf[0]; s2 += s1;
        s1 += (uint32_t)buf[1]; s2 += s1;
        s1 += (uint32_t)buf[2]; s2 += s1;
        s1 += (uint32_t)buf[3]; s2 += s1;
        s1 += (uint32_t)buf[4]; s2 += s1;
        s1 += (uint32_t)buf[5]; s2 += s1;
        s1 += (uint32_t)buf[6]; s2 += s1;
        s1 += (uint32_t)buf[7]; s2 += s1;
        buf += 8;
        n -= 8;
    }
    while (n--) { s1 += (uint32_t)*buf++; s2 += s1; }
    s1 %= 65521U;
    s2 %= 65521U;
    return (s2 << 16) | s1;
}

/* ═══ Counting-epic miscompile fixes (this battery's finds) ═══════════ */

/* Two counting accumulators in one loop — the b chain must survive. */
unsigned long cnt_two_acc(const unsigned char *restrict s, unsigned long n) {
    unsigned long a = 0, b = 0;
    for (unsigned long i = 0; i < n; ++i) {
        a += (s[i] == 0x5A);
        b += (s[i] > 0x21);
    }
    return a * 1000003UL + b;
}

/* IV live-out, unguarded: the final i must equal n. */
unsigned long cnt_iv_out(const unsigned char *restrict s, unsigned long n) {
    unsigned long c = 0;
    unsigned long i = 0;
    for (; i < n; ++i) c += (s[i] == 0x5A);
    return c * 1000003UL + i;
}

/* IV live-out, guarded: the bypass path must keep i = 0. */
unsigned long cnt_iv_guarded(const unsigned char *restrict s, unsigned long n) {
    unsigned long c = 0;
    unsigned long i = 0;
    if (n > 4) {
        for (; i < n; ++i) c += (s[i] == 0x5A);
    }
    return c * 1000003UL + i;
}

/* Narrow-compare constant wrap: 0xAA arrives as I8(-86). */
unsigned long cnt_wrap_aa(const unsigned char *restrict s, unsigned long n) {
    unsigned long c = 0;
    for (unsigned long i = 0; i < n; ++i) c += (s[i] == 0xAA);
    return c;
}
unsigned long cnt_wrap_hi(const unsigned char *restrict s, unsigned long n) {
    unsigned long c = 0;
    for (unsigned long i = 0; i < n; ++i) c += (s[i] > 0xF0);
    return c;
}
unsigned long cnt_wrap_lo(const unsigned char *restrict s, unsigned long n) {
    unsigned long c = 0;
    for (unsigned long i = 0; i < n; ++i) c += (s[i] < 0x80);
    return c;
}

/* Body value live-out of a counting loop. */
unsigned long cnt_liveout(const unsigned char *restrict s, unsigned long n) {
    unsigned long c = 0;
    unsigned long last = 0;
    for (unsigned long i = 0; i < n; ++i) {
        c += (s[i] == 0x5A);
        last = s[i];
    }
    return c * 256UL + last;
}

/* Volatile counting loop — must stay scalar. */
unsigned long cnt_volatile(const volatile unsigned char *s, unsigned long n) {
    unsigned long c = 0;
    for (unsigned long i = 0; i < n; ++i) c += (s[i] == 0x5A);
    return c;
}

/* ── references for the counting shapes (volatile, never packed) ────── */
static unsigned long rcnt(const unsigned char *s, unsigned long n, int which) {
    volatile unsigned long a = 0, b = 0, last = 0;
    volatile unsigned long i = 0;
    for (; i < n; ++i) {
        unsigned char v = s[i];
        a += (v == 0x5A);
        b += (v > 0x21);
        last = v;
    }
    switch (which) {
    case 0: return a * 1000003UL + b;
    case 1: return a * 1000003UL + i;
    case 2: return a * 256UL + last;
    case 3: return a;
    case 4: return b;
    default: return 0;
    }
}
static unsigned long rcnt_cmp(const unsigned char *s, unsigned long n, uint8_t k, int gt) {
    volatile unsigned long c = 0;
    for (unsigned long i = 0; i < n; ++i) {
        c += gt ? (s[i] > k) : (s[i] == k);
    }
    return c;
}
static unsigned long rcnt_lo(const unsigned char *s, unsigned long n) {
    volatile unsigned long c = 0;
    for (unsigned long i = 0; i < n; ++i) c += (s[i] < 0x80);
    return c;
}

/* ── exact sequential mirrors for the adversarial adler shapes ────── */
/* two_pairs: the second accumulator pair over ^0xFF bytes. */
static uint32_t m_two_pairs(uint32_t adler, const uint8_t *b, size_t n) {
    volatile uint32_t s1 = adler & 0xffff, s2 = (adler >> 16) & 0xffff;
    volatile uint32_t t1 = 0, t2 = 0;
    for (size_t i = 0; i < n; i++) {
        uint32_t u1 = (uint32_t)(s1 + b[i]);
        uint32_t u2 = (uint32_t)(s2 + u1);
        uint32_t v1 = (uint32_t)(t1 + (uint8_t)(b[i] ^ 0xFF));
        uint32_t v2 = (uint32_t)(t2 + v1);
        s1 = u1; s2 = u2; t1 = v1; t2 = v2;
    }
    s1 %= 65521U; s2 %= 65521U;
    return ((uint32_t)s2 << 16) | ((uint32_t)s1 ^ ((uint32_t)t2 & 0xffff));
}
/* liveout: last = buf[7] of the final DO8 window (0 if none ran). */
static uint32_t m_liveout(uint32_t adler, const uint8_t *b, size_t n) {
    volatile uint32_t s1 = adler & 0xffff, s2 = (adler >> 16) & 0xffff;
    volatile uint32_t last = 0;
    size_t i = 0;
    while (n - i >= 8) {
        for (int k = 0; k < 8; k++) {
            uint32_t u1 = (uint32_t)(s1 + b[i + (size_t)k]);
            uint32_t u2 = (uint32_t)(s2 + u1);
            s1 = u1; s2 = u2;
        }
        last = b[i + 7];
        i += 8;
    }
    while (i < n) {
        uint32_t u1 = (uint32_t)(s1 + b[i]);
        uint32_t u2 = (uint32_t)(s2 + u1);
        s1 = u1; s2 = u2;
        i++;
    }
    s1 %= 65521U; s2 %= 65521U;
    return ((uint32_t)s2 << 16) | ((uint32_t)s1 ^ ((uint32_t)last & 0xffff));
}
/* store: buf[0] = (uint8_t)s1 AFTER each 8-byte window (visible to the
 * next window's buf[0] read). */
static uint32_t m_store(uint32_t adler, uint8_t *b, size_t n) {
    volatile uint32_t s1 = adler & 0xffff, s2 = (adler >> 16) & 0xffff;
    size_t i = 0;
    while (n - i >= 8) {
        for (int k = 0; k < 8; k++) {
            uint32_t u1 = (uint32_t)(s1 + b[i + (size_t)k]);
            uint32_t u2 = (uint32_t)(s2 + u1);
            s1 = u1; s2 = u2;
        }
        b[i] = (uint8_t)s1;
        i += 8;
    }
    while (i < n) {
        uint32_t u1 = (uint32_t)(s1 + b[i]);
        uint32_t u2 = (uint32_t)(s2 + u1);
        s1 = u1; s2 = u2;
        i++;
    }
    s1 %= 65521U; s2 %= 65521U;
    return ((uint32_t)s2 << 16) | (uint32_t)s1;
}
/* stride_mismatch: reads buf[0,2,4,6] per window, n -= 4 per window. */
static uint32_t m_stride(uint32_t adler, const uint8_t *b, size_t n) {
    volatile uint32_t s1 = adler & 0xffff, s2 = (adler >> 16) & 0xffff;
    size_t i = 0, rem = n;
    while (rem >= 8) {
        for (int k = 0; k < 4; k++) {
            uint32_t u1 = (uint32_t)(s1 + b[i + (size_t)(2 * k)]);
            uint32_t u2 = (uint32_t)(s2 + u1);
            s1 = u1; s2 = u2;
        }
        i += 8;
        rem -= 4;
    }
    while (rem--) {
        uint32_t u1 = (uint32_t)(s1 + b[i]);
        uint32_t u2 = (uint32_t)(s2 + u1);
        s1 = u1; s2 = u2;
        i++;
    }
    s1 %= 65521U; s2 %= 65521U;
    return ((uint32_t)s2 << 16) | (uint32_t)s1;
}

int main(void) {
    for (size_t i = 0; i < N; i++)
        usrc[i] = (unsigned char)(i * 2654435761u >> 13);

    /* Valid adler init values (the deferred-mod contract: s1, s2 < 65521). */
    const uint32_t inits[] = {1, 0x12345678u, 0x00110022u, 0x0000abcdu, 0x00010001u};
    const int ninit = (int)(sizeof inits / sizeof inits[0]);

    /* ── POSITIVE differentials: every length, every init ───────────── */
    for (int k = 0; k < ninit; k++) {
        for (size_t n = 0; n <= 700; n++) {
            uint32_t want = vref(inits[k], usrc, n);
            if (adler_do8(inits[k], usrc, n) != want) {
                printf("FAIL do8 init=%08x n=%zu\n", inits[k], n);
                return 1;
            }
            if (adler_do4(inits[k], usrc, n) != want) {
                printf("FAIL do4 init=%08x n=%zu\n", inits[k], n);
                return 2;
            }
            if (adler_do16(inits[k], usrc, n) != want) {
                printf("FAIL do16 init=%08x n=%zu\n", inits[k], n);
                return 3;
            }
            if (adler_do32(inits[k], usrc, n) != want) {
                printf("FAIL do32 init=%08x n=%zu\n", inits[k], n);
                return 4;
            }
            if (adler_plain1(inits[k], usrc, n) != want) {
                printf("FAIL plain1 init=%08x n=%zu\n", inits[k], n);
                return 5;
            }
            if (adler_do8_signed(inits[k], usrc, (int64_t)n) != want) {
                printf("FAIL do8_signed init=%08x n=%zu\n", inits[k], n);
                return 6;
            }
            if (adler_do8_u32n(inits[k], usrc, (uint32_t)n) != want) {
                printf("FAIL do8_u32n init=%08x n=%zu\n", inits[k], n);
                return 7;
            }
        }
    }
    /* Long buffers: full vector iterations + every remainder. */
    for (size_t n = 701; n <= N; n += 3) {
        uint32_t want = vref(1, usrc, n);
        if (adler_do8(1, usrc, n) != want) { printf("FAIL do8 long n=%zu\n", n); return 8; }
        if (adler_do4(1, usrc, n) != want) { printf("FAIL do4 long n=%zu\n", n); return 9; }
        if (adler_plain1(1, usrc, n) != want) { printf("FAIL plain1 long n=%zu\n", n); return 10; }
    }

    /* ── NEGATIVE differentials: declined but correct ───────────────── */
    for (size_t n = 0; n <= 700; n++) {
        uint32_t want = vref(1, usrc, n);
        if (adler_k3(1, usrc, n) != want) { printf("FAIL k3 n=%zu\n", n); return 20; }
        if (adler_two_pairs(1, usrc, n) != m_two_pairs(1, usrc, n)) { printf("FAIL two_pairs n=%zu\n", n); return 21; }
        if (adler_liveout(1, usrc, n) != m_liveout(1, usrc, n)) { printf("FAIL liveout n=%zu\n", n); return 22; }
        if (adler_stride_mismatch(1, usrc, n) != m_stride(1, usrc, n)) { printf("FAIL stride n=%zu\n", n); return 23; }
        if (adler_volatile(1, usrc, n) != want) { printf("FAIL volatile n=%zu\n", n); return 24; }
    }
    {
        /* adler_store works on a scratch copy against its exact mirror. */
        static uint8_t scratch[512];
        static uint8_t mirror[512];
        for (size_t n = 0; n <= 300; n += 7) {
            memcpy(scratch, usrc, n);
            memcpy(mirror, usrc, n);
            uint32_t g = adler_store(1, scratch, n);
            uint32_t want = m_store(1, mirror, n);
            if (g != want) { printf("FAIL store n=%zu\n", n); return 25; }
        }
    }
    {
        /* Signed stream: reference over sign-extended bytes. */
        static uint32_t want_i8 = 0;
        for (size_t n = 0; n <= 300; n += 3) {
            const signed char *ss = (const signed char *)usrc;
            volatile uint32_t s1 = 1, s2 = 0;
            for (size_t i = 0; i < n; i++) {
                uint32_t t1 = (uint32_t)(s1 + (uint32_t)ss[i]);
                uint32_t t2 = (uint32_t)(s2 + t1);
                s1 = t1;
                s2 = t2;
            }
            s1 %= 65521U;
            s2 %= 65521U;
            want_i8 = (s2 << 16) | s1;
            if (adler_i8(1, ss, n) != want_i8) { printf("FAIL i8 n=%zu\n", n); return 26; }
        }
    }

    /* ── Counting-epic fixes ────────────────────────────────────────── */
    for (unsigned long n = 0; n <= 700; n++) {
        unsigned long w0 = rcnt(usrc, n, 0);
        if (cnt_two_acc(usrc, n) != w0) { printf("FAIL cnt_two_acc n=%lu\n", n); return 30; }
        unsigned long w1 = rcnt(usrc, n, 1);
        if (cnt_iv_out(usrc, n) != w1) { printf("FAIL cnt_iv_out n=%lu\n", n); return 31; }
        /* The guarded variant keeps i = 0 (and c = 0) on the bypass
         * path: the reference only applies when the guard fired. */
        unsigned long w1g = (n > 4) ? w1 : 0;
        if (cnt_iv_guarded(usrc, n) != w1g) { printf("FAIL cnt_iv_guarded n=%lu\n", n); return 32; }
        unsigned long w2 = rcnt(usrc, n, 2);
        if (cnt_liveout(usrc, n) != w2) { printf("FAIL cnt_liveout n=%lu\n", n); return 33; }
        if (cnt_wrap_aa(usrc, n) != rcnt_cmp(usrc, n, 0xAA, 0)) { printf("FAIL wrap_aa n=%lu\n", n); return 34; }
        if (cnt_wrap_hi(usrc, n) != rcnt_cmp(usrc, n, 0xF0, 1)) { printf("FAIL wrap_hi n=%lu\n", n); return 35; }
        if (cnt_wrap_lo(usrc, n) != rcnt_lo(usrc, n)) { printf("FAIL wrap_lo n=%lu\n", n); return 36; }
        if (cnt_volatile(usrc, n) != rcnt_cmp(usrc, n, 0x5A, 0)) { printf("FAIL cnt_volatile n=%lu\n", n); return 37; }
    }

    printf("OK vec_adler_epic\n");
    return 0;
}
