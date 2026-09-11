// Loop-memset recognition: constant-byte fill loops become `memset`
// (default-on at -O2; kill switch CCC_NO_MEMSET_LOOP).
// Differential test: every checksum must match GCC bit-for-bit. The transform
// must be exact for byte/word/dword wide fills, uniform nonzero patterns,
// advancing-pointer forms, exit-value reconstruction, zero-trip loops,
// dynamically bounded loops and fills nested in an enclosing loop.
#include <stdio.h>

unsigned char G[4096];
unsigned int W[1024];
unsigned LEN;

static unsigned long sum8(const unsigned char *p, unsigned n) {
    unsigned long s = 0;
    for (unsigned i = 0; i < n; i++) s += p[i];
    return s;
}

static unsigned long sum32(const unsigned int *p, unsigned n) {
    unsigned long s = 0;
    for (unsigned i = 0; i < n; i++) s += p[i];
    return s;
}

// byte fill, indexed form
static void fill_byte(unsigned char c, unsigned n) {
    for (unsigned i = 0; i < n; i++) G[i] = c;
}

// dword zero fill, indexed form (wide store, uniform pattern 0)
static void fill_wide_zero(unsigned n) {
    for (unsigned i = 0; i < n; i++) W[i] = 0;
}

// dword uniform nonzero pattern (0xAAAAAAAA fills every byte with 0xAA)
static void fill_wide_pattern(unsigned n) {
    for (unsigned i = 0; i < n; i++) W[i] = 0xAAAAAAAAu;
}

// advancing-pointer form
static void fill_bump(unsigned char c, unsigned n) {
    unsigned char *d = G;
    for (unsigned i = 0; i < n; i++) *d++ = c;
}

// the loop iv is used after the loop: exit-value reconstruction
static unsigned char *fill_exituse(unsigned char c, unsigned n) {
    unsigned char *d = G;
    for (unsigned i = 0; i < n; i++) *d++ = c;
    return d;
}

// zero-trip: the n != 0 guard must skip the call entirely. The buffer is
// zero-initialized first so every read byte is defined.
static unsigned fill_zerotrip(unsigned n) {
    unsigned char d[64];
    for (unsigned i = 0; i < 64; i++) d[i] = 0;
    for (unsigned i = 0; i < n && i < 64; i++) d[i] = 0x5A;
    unsigned long s = 0;
    for (unsigned i = 0; i < 64; i++) s += d[i];
    return (unsigned)s;
}

// fill nested in an enclosing loop
static void fill_nested(void) {
    unsigned ns[4] = {100, 200, 300, 400};
    for (unsigned k = 0; k < 4; k++) {
        unsigned n = ns[k];
        for (unsigned i = 0; i < n; i++) G[i] = (unsigned char)k;
    }
}

// counter used past the loop (init != 0: n = bound - init reconstruction)
static unsigned fill_offset_init(void) {
    unsigned i;
    for (i = 3; i < 259; i++) G[i - 3] = 0x11;
    return i;
}

int main(void) {
    fill_byte(0, 4000);
    printf("b0 %lu\n", sum8(G, 4000));

    fill_byte(0xFF, 4000);
    printf("bff %lu\n", sum8(G, 4000));

    fill_wide_zero(1024);
    printf("wz %lu\n", sum32(W, 1024));

    fill_wide_pattern(1000);
    printf("wp %lu\n", sum32(W, 1000));

    fill_bump(0x5A, 777);
    printf("bump %lu\n", sum8(G, 777));

    unsigned char *e = fill_exituse(0x33, 100);
    printf("exituse %ld %lu\n", (long)(e - G), sum8(G, 100));

    printf("zt %u %u\n", fill_zerotrip(0), fill_zerotrip(30));

    fill_nested();
    printf("nested %lu\n", sum8(G, 400));

    // Sequence explicitly: printf argument evaluation order is unspecified.
    unsigned off_i = fill_offset_init();
    printf("offinit %u %lu\n", off_i, sum8(G, 256));
    return 0;
}
