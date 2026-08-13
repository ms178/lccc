/*
 * v8 regression: widen-then-narrow cast chain folding in simplify.
 *
 * Cast(Cast(x, A->B), B->C) with B strictly widest collapses to Cast(x, A->C).
 * Covers every signedness mix and the extend-then-truncate semantics across
 * byte values and INT edges; differential vs GCC.
 */
#include <stdio.h>
#include <limits.h>

static int f1(unsigned char x)  { return (int)(long long)(int)x; }
static int f2(char x)           { return (int)(unsigned short)(int)x; }
static long long f3(int x)      { return (long long)(int)(long long)x; }
static unsigned f4(signed char x) { return (unsigned)(int)(signed char)x; }
static int f5(unsigned char x)  { return (int)(unsigned int)(unsigned long)x; }
static short f6(int x)          { return (short)(long long)(int)x; }
static unsigned long long f7(signed char x) { return (unsigned long long)(long long)(int)x; }
static int f8(unsigned short x) { return (int)(long long)(unsigned short)x; }

static unsigned long long mix(unsigned long long h, unsigned long long v) {
    return (h ^ v) * 0x9e3779b97f4a7c15ULL + 0xdeadbeefULL;
}

int main(void) {
    unsigned long long h = 1469598103934665603ULL;
    int c;
    for (c = 0; c <= 255; c++) {
        h = mix(h, (unsigned)f1((unsigned char)c));
        h = mix(h, (unsigned)f2((char)(c - 128)));
        h = mix(h, (unsigned long long)f4((signed char)(c - 128)));
        h = mix(h, (unsigned)f5((unsigned char)c));
        h = mix(h, (unsigned short)f6(c * 65537 - 1));
        h = mix(h, (unsigned long long)f7((signed char)(c - 128)));
        h = mix(h, (unsigned)f8((unsigned short)(c * 257)));
    }
    {
        static const int vals[] = { INT_MIN, INT_MIN + 1, -1, 0, 1, INT_MAX - 1, INT_MAX };
        unsigned i;
        for (i = 0; i < sizeof(vals) / sizeof(vals[0]); i++)
            h = mix(h, (unsigned long long)f3(vals[i]));
    }
    printf("%llu\n", (unsigned long long)h);
    return 0;
}
