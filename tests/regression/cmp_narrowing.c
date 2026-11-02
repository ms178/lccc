/*
 * comparison narrowing + the U32 operand cast-type fix.
 *
 * `Cmp(op, Cast(x, T->W), C, W)` narrows to `Cmp(op, x, C', T)` when the
 * extension preserves op's ordering and C fits in T, so a promoted byte is
 * compared at its native width (cmpb/testb) instead of widening — GCC/ICX's
 * byte-compare shape. The cast-type fix removes the identity Cast(U32->U32)
 * that was mislabeled I64->U32 and emitted a spurious `mov` per operand.
 *
 * Differential vs GCC across all byte values and INT/LLONG edges.
 */
#include <stdio.h>
#include <limits.h>

static int lt80(unsigned char c) { return c < 0x80U; }
static int az(unsigned char c) { return c >= 'a' && c <= 'z'; }
static int azAZ(unsigned char c) { return (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z'); }
static int digit(unsigned char c) { return c >= '0' && c <= '9'; }
static int u16lt(unsigned short s) { return s < 500u; }
static int u16ge(unsigned short s) { return s >= 60000u; }
static int sc_pos(signed char c) { return c < 5; }
static int sc_range(signed char c) { return c >= -5 && c <= 5; }
static int eq200(unsigned char c) { return c == 200u; }
static int ne7(unsigned char c) { return c != 7u; }
static int ge200(unsigned char c) { return c >= 200u; }
static int u32lt(unsigned int x) { return x < 0x80u; }      /* NOT narrowable past U32 */
static int u32big(unsigned int x) { return x < 0x80000000u; }
static int short_pos(short s) { return s > 100; }
static int int_cmp(int x) { return x < 128; }               /* I32: no narrowing */

static unsigned long long mix(unsigned long long h, unsigned long long v) {
    return (h ^ v) * 0x9e3779b97f4a7c15ULL + 0xdeadbeefULL;
}

int main(void) {
    unsigned long long h = 1469598103934665603ULL;
    int c;
    for (c = 0; c <= 255; c++) {
        h = mix(h, (unsigned)lt80((unsigned char)c));
        h = mix(h, (unsigned)az((unsigned char)c));
        h = mix(h, (unsigned)azAZ((unsigned char)c));
        h = mix(h, (unsigned)digit((unsigned char)c));
        h = mix(h, (unsigned)sc_pos((signed char)(c - 128)));
        h = mix(h, (unsigned)sc_range((signed char)(c - 128)));
        h = mix(h, (unsigned)eq200((unsigned char)c));
        h = mix(h, (unsigned)ne7((unsigned char)c));
        h = mix(h, (unsigned)ge200((unsigned char)c));
    }
    unsigned short s;
    for (s = 0; s < 4096; s += 97) {
        h = mix(h, (unsigned)u16lt(s));
        h = mix(h, (unsigned)u16ge(s));
        h = mix(h, (unsigned)short_pos((short)(s - 2048)));
    }
    {
        static const unsigned int xs[] = { 0, 1, 127, 128, 255, 0x7fffffffu, 0x80000000u, 0xffffffffu };
        unsigned n;
        for (n = 0; n < sizeof(xs) / sizeof(xs[0]); n++) {
            h = mix(h, (unsigned)u32lt(xs[n]));
            h = mix(h, (unsigned)u32big(xs[n]));
        }
    }
    {
        static const int is[] = { INT_MIN, -129, -128, -1, 0, 127, 128, 129, INT_MAX };
        unsigned n;
        for (n = 0; n < sizeof(is) / sizeof(is[0]); n++)
            h = mix(h, (unsigned)int_cmp(is[n]));
    }
    printf("%llu\n", (unsigned long long)h);
    return 0;
}
