/* GVN cross-signedness load CSE: an I8 sign-test load that dominates U8
 * reloads of the same address must forward the loaded register instead of
 * reloading memory (sqlite_get_varint chain shape). Also covers
 * GEP(param+i) address canonicalization across blocks: p[1]/p[2] loads in
 * later arms reuse the sign-test loads from earlier blocks.
 *
 * Tripwires baked in:
 * - high-bit bytes (0x80..0xff): a sext/zext confusion forwards the wrong
 *   value (0xffffff80 vs 0x80) and corrupts the checksum;
 * - stores to *out in every exit arm: sibling-subtree stores must not kill
 *   the dominator-scoped entries (rollback check);
 * - single-use rule: each reloaded byte feeds exactly one integer widening
 *   cast (if the shape drifts multi-use, the optimization simply stops
 *   firing and the test still passes functionally).
 */
#include <stdio.h>

static unsigned t(const unsigned char *p, unsigned *out) {
    signed char c0 = *(const signed char *)p;
    if (c0 >= 0) {
        *out = (unsigned)p[0];
        return 1;
    }
    signed char c1 = *(const signed char *)(p + 1);
    if (c1 >= 0) {
        *out = ((unsigned)p[0] << 7) | (unsigned)p[1];
        return 2;
    }
    *out = ((unsigned)p[0] << 14) | ((unsigned)p[1] << 7) | (unsigned)p[2];
    return 3;
}

int main(void) {
    static const unsigned char vals[][4] = {
        {0x00, 0x00, 0x00, 0x00},
        {0x01, 0x7f, 0x00, 0x00},
        {0x7f, 0x01, 0x7e, 0x00},
        {0x80, 0x00, 0x00, 0x00},
        {0x80, 0x7f, 0x00, 0x00},
        {0x81, 0x80, 0x00, 0x00},
        {0xff, 0x00, 0x00, 0x00},
        {0xff, 0x7f, 0x00, 0x00},
        {0xff, 0xff, 0x00, 0x00},
        {0xff, 0xff, 0x7f, 0x00},
        {0xff, 0xff, 0xff, 0x00},
        {0x80, 0x80, 0x80, 0x00},
    };
    unsigned checksum = 0;
    unsigned i;
    for (i = 0; i < sizeof(vals) / sizeof(vals[0]); i++) {
        unsigned out = 0;
        unsigned n = t(vals[i], &out);
        checksum += out ^ (n * 0x9e3779b1u) ^ (unsigned)vals[i][0];
    }
    printf("OK gvn_xsign_load_cse %u\n", checksum);
    return 0;
}
