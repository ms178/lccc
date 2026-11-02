/* Bit-idiom soundness and recognition: rotate, SWAR popcount, CLZ select
 * chain, byte-swap/bit-reverse networks.
 *
 * Two things are locked per idiom:
 *  1. RECOGNITION: the canonical portable spelling folds to the native op.
 *  2. SOUNDNESS: near-miss shapes that differ only through integer CASTS
 *     must either keep the portable semantics or fold to something equal —
 *     never change the value.  The cast attacks below are all shapes where
 *     a matcher that peels through casts to compare "the same root value"
 *     would miscompile: zext/sext halves of one narrow source, truncating
 *     casts feeding a 32-bit network, widening casts feeding a 64-bit one.
 *
 * Output is a fixed set of hex/decimal lines, diffed against a reference
 * compiler build by the regression suite.
 */
#include <stdio.h>
#include <stdint.h>

#define ROTL_L(v, n) (((v) << (n)) | ((v) >> (sizeof(v) * 8 - (n))))
#define ROTL_R(v, n) (((v) >> (sizeof(v) * 8 - (n))) | ((v) << (n))) /* swapped order */

static uint32_t next(uint32_t *s) {
    *s ^= *s << 13; *s ^= *s >> 17; *s ^= *s << 5;
    return *s;
}

/* Portable spellings the recognizer must fold. */
static uint32_t swar_popcount(uint32_t v) {
    v = v - ((v >> 1) & 0x55555555u);
    v = (v & 0x33333333u) + ((v >> 2) & 0x33333333u);
    v = (v + (v >> 4)) & 0x0f0f0f0fu;
    return (v * 0x01010101u) >> 24;
}
static int clz32_net(uint32_t x) {
    int n = 0;
    if (x <= 0x0000FFFFu) { n += 16; x <<= 16; }
    if (x <= 0x00FFFFFFu) { n += 8;  x <<= 8;  }
    if (x <= 0x0FFFFFFFu) { n += 4;  x <<= 4;  }
    if (x <= 0x3FFFFFFFu) { n += 2;  x <<= 2;  }
    if (x <= 0x7FFFFFFFu) { n += 1;  }
    return n;
}
static uint32_t bswap_net(uint32_t x) {
    x = ((x >> 1) & 0x55555555u) | ((x & 0x55555555u) << 1);
    x = ((x >> 2) & 0x33333333u) | ((x & 0x33333333u) << 2);
    x = ((x >> 4) & 0x0f0f0f0fu) | ((x & 0x0f0f0f0fu) << 4);
    x = ((x >> 8) & 0x00ff00ffu) | ((x & 0x00ff00ffu) << 8);
    x = (x >> 16) | (x << 16);
    return x;
}
static uint32_t rotl32_rt(uint32_t x, unsigned n) {
    return (x << n) | (x >> (32 - n));
}

int main(void) {
    uint32_t s = 0x12345678u;
    uint32_t acc = 0;
    uint64_t acc64 = 0;
    uint16_t acc16 = 0;
    uint32_t p = 0, b = 0, cl = 0;
    uint8_t acc8 = 0;
    int c = 0;

    for (int i = 0; i < 2000; i++) {
        uint32_t x = next(&s);
        uint64_t x64 = ((uint64_t)x << 32) ^ (x * 0x9e3779b97f4a7c15ull);
        uint16_t x16 = (uint16_t)(x >> 7);

        /* --- rotate: both operand orders, extreme amounts ------------- */
        acc ^= ROTL_L(x, 16);
        acc ^= ROTL_R(x, 12);
        acc ^= ROTL_L(x, 8);
        acc ^= ROTL_R(x, 7);
        acc64 ^= ROTL_L(x64, 33);
        acc64 ^= ROTL_R(x64, 40);
        acc64 ^= ROTL_L(x64, 1);
        acc64 ^= ROTL_R(x64, 63);
        /* rotate-right spelling with the small count on the shr side */
        acc ^= (x >> 7) | (x << 25);
        /* run-time amount, both macro spellings */
        acc ^= rotl32_rt(x, i & 31);
        acc ^= (x >> (i & 31)) | (x << (32 - (i & 31)));

        /* --- negative controls: NOT rotates --------------------------- */
        /* non-complementary counts (5 + 20 != 32) */
        acc ^= (x << 5) | (x >> 20);
        /* same counts as a rotate by 5, but the halves read DIFFERENT
         * values — folding this would be a miscompile */
        acc ^= (x << 5) | (next(&s) >> 27);
        /* arithmetic shift half: not a rotate */
        acc ^= (uint32_t)(((int32_t)x << 5) | ((int32_t)x >> 27));

        /* --- u16 rotate spelled through promotions -------------------- */
        /* counts complementary at the promoted width: a genuine u32
         * rotate of the extended value, then truncated — must stay
         * correct (folded or not) */
        {
            uint32_t w = (uint32_t)x16;
            acc16 ^= (uint16_t)((w << 8) | (w >> 24));
        }
        /* counts complementary only at 16: the narrow-rotate pattern —
         * now RECOGNIZED as rol16 (both halves shift the same zext of
         * x16; the promoted >> arrives as an arithmetic shift whose sign
         * bit is provably zero) */
        acc16 ^= (uint16_t)(((uint32_t)x16 << 8) | ((uint32_t)x16 >> 8));
        /* ...variable count, complement is the live `16 - c`: rol16 %cl */
        {
            unsigned c = (x >> 7) & 15u; /* 0..15 */
            acc16 += (uint16_t)(((uint32_t)x16 << c) | ((uint32_t)x16 >> (16 - c)));
        }
        /* ...variable count including the degenerate 16 (identity) */
        {
            unsigned c = (x >> 3) & 17u; /* 0..17 hitting 16; 17 makes the
                                          * portable form's `16 - c` shift
                                          * negative, so clamp it the way
                                          * real code does */
            if (c > 16u) c = 16u;
            acc16 += (uint16_t)(((uint32_t)x16 << c) | ((uint32_t)x16 >> (16 - c)));
        }
        /* ...masked source: the And carries the zero-extension proof the
         * same way the zext does */
        acc16 ^= (uint16_t)((((x & 0xffffu) << 4) | ((x & 0xffffu) >> 12)));
        /* --- u8 rotates ------------------------------------------------ */
        {
            uint8_t b8 = (uint8_t)x;
            unsigned c = (x >> 9) & 7u; /* 0..7 */
            acc8 ^= (uint8_t)((b8 << 4) | (b8 >> 4));              /* const */
            acc8 += (uint8_t)((b8 << c) | (b8 >> (8 - c)));        /* var  */
            acc8 ^= (uint8_t)(((uint32_t)b8 << 5) | ((uint32_t)b8 << 3)); /* not complementary at 8: 5+5!=8 — stays portable */
        }
        /* --- narrow-rotate CAST ATTACKS (soundness) ------------------- */
        /* (e) sext source: `(int16_t)sv << 8 | (int16_t)sv >> 8` — the
         *     halves read the SIGN-extended promotion, so bits above 16
         *     are replicated sign, not zeros.  For negative sv the
         *     portable value differs from rol16(sv, 8): folding through
         *     the cast would change the result. */
        {
            int16_t sv = (int16_t)x16;
            acc16 ^= (uint16_t)(((int32_t)sv << 8) | ((int32_t)sv >> 8));
        }
        /* (f) halves through different extensions of one u16 root (zext
         *     vs and-masked): both are zero above 16, so the VALUES are
         *     equal — but the identity check must not match them through
         *     the cast; either way the value must be exact. */
        {
            uint32_t z = (uint32_t)x16;
            uint32_t m = x & 0xffffu;
            acc16 ^= (uint16_t)((z << 7) | (m >> 9));
        }
        /* (g) multi-consumer Or: the wide Or feeds a second reader, so
         *     the fold must not delete the chain — the truncation's own
         *     value must still be exact. */
        {
            uint32_t w = (uint32_t)x16;
            uint32_t orv = (w << 8) | (w >> 8);
            acc16 ^= (uint16_t)orv;
            acc ^= orv; /* the second reader */
        }

        /* --- popcount / clz / bswap networks -------------------------- */
        p += swar_popcount(x);
        cl += (uint32_t)clz32_net(x);
        b += bswap_net(x);

        /* --- CAST ATTACKS: soundness of the matchers ------------------ */
        /* (a) truncating cast into the 32-bit networks: the network sees
         *     only the low 16 bits.  A matcher that peels through the
         *     cast to the wide root would compute the wrong value. */
        p += swar_popcount((uint32_t)(uint16_t)x);
        cl += (uint32_t)clz32_net((uint32_t)(uint16_t)x);
        b += bswap_net((uint32_t)(uint16_t)x);
        /* (b) sign-extending narrow source: zext and sext of one byte
         *     are different 32-bit values. */
        {
            int8_t sb = (int8_t)x;
            p += swar_popcount((uint32_t)(int32_t)sb);
            b += bswap_net((uint32_t)(int32_t)sb);
        }
        /* (c) rotate halves through DIFFERENT casts of one narrow source:
         *     zext on the shl side, sext on the shr side.  Not a rotate
         *     of anything; folding it drops the sign half. */
        {
            uint32_t z = (uint32_t)(uint8_t)x;
            int32_t sg = (int32_t)(int8_t)x;
            acc += (z << 16) | ((uint32_t)sg >> 16);
            /* same-cast control: this one IS a rotate of the extended
             * value and must stay correct */
            acc ^= (z << 16) | (z >> 16);
        }
        /* (d) widening cast into the 64-bit rotate: the shifts run at
         *     the promoted width on the extended value. */
        acc64 ^= ROTL_L((uint64_t)(uint32_t)x, 17);
    }

    printf("%08x %016llx %04x %02x %u %u %u\n",
           acc, (unsigned long long)acc64, acc16, acc8, p, cl, b);
    return 0;
}
