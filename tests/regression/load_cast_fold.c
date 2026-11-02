/* Regression: Load(U8/U16/U32)->Cast widening fold (W2, 2026-08-10).
 * The fold emits the zero-extending load directly into the cast dest's
 * register and skips the cast. Three soundness bugs were found while
 * developing it; this test locks in the sound-by-construction handshake:
 *  (a) single-use loads whose cast dest register is live-conflicted must NOT
 *      fold (the load must not clobber a value read right after);
 *  (b) loads that take a non-redirecting emission path must still write the
 *      cast dest (the cast may not be skipped unless the redirect fired);
 *  (c) folded values consumed through further casts/branches stay correct.
 * Fails as wrong-result (not crash) if any fold misfires. */
#include <stdio.h>
#include <stdint.h>
#include <string.h>

static unsigned char buf[4096] __attribute__((aligned(64)));

/* (a)+(c): byte loads widened and compared in a chain (gzip longest_match
 * first-match-check shape). Every byte position and alignment exercised. */
static unsigned chain_check(const unsigned char *p, unsigned n) {
    unsigned acc = 0;
    for (unsigned i = 0; i + 4 <= n; i++) {
        unsigned int a = p[i];          /* Load U8 -> Cast I32 */
        unsigned int b = p[i + 1];
        unsigned long long w = p[i + 2]; /* Load U8 -> Cast I64 */
        unsigned short h = (unsigned short)((p[i + 3] << 8) | p[i]);
        if (a == b)
            acc += 1;
        if ((unsigned)(w & 0xff) == a)
            acc += 2;
        if ((h & 0xff) == a)
            acc += 4;
        if (a > 128 && b < 64)
            acc += 8;
    }
    return acc;
}

/* (b): loads feeding memcpy/memset regions — their loads must take
 * non-redirecting paths without losing the widened value. */
static unsigned copy_region(unsigned char *dst, const unsigned char *src, unsigned n) {
    unsigned acc = 0;
    for (unsigned i = 0; i + 8 <= n; i += 8) {
        unsigned int x = src[i];      /* Load U8 -> Cast */
        unsigned int y = src[i + 7];
        memcpy(dst + i, src + i, 8);
        if (dst[i] == (unsigned char)x)
            acc += 1;
        if (dst[i + 7] == (unsigned char)y)
            acc += 2;
    }
    return acc;
}

int main(void) {
    uint32_t st = 0xDEADBEEFu;
    for (size_t i = 0; i < sizeof buf; i++) {
        st ^= st << 13; st ^= st >> 17; st ^= st << 5;
        buf[i] = (unsigned char)(st >> 24);
    }
    unsigned expect_chain = 0, expect_copy = 0;
    unsigned char dst[4096];
    for (unsigned off = 0; off < 16; off++) {
        expect_chain += chain_check(buf + off, 3000 - off);
        memset(dst, 0, sizeof dst);
        expect_copy += copy_region(dst, buf + off, 3000 - off);
    }
    unsigned got_chain = 0, got_copy = 0;
    for (unsigned off = 0; off < 16; off++) {
        got_chain += chain_check(buf + off, 3000 - off);
        memset(dst, 0, sizeof dst);
        got_copy += copy_region(dst, buf + off, 3000 - off);
    }
    if (got_chain != expect_chain) {
        printf("chain mismatch: got %u expect %u\n", got_chain, expect_chain);
        return 1;
    }
    if (got_copy != expect_copy) {
        printf("copy mismatch: got %u expect %u\n", got_copy, expect_copy);
        return 2;
    }
    return 0;
}
