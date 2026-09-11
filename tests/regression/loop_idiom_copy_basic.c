// Loop-idiom recognition: byte-copy loops become `memcpy` (CCC_LOOP_IDIOM=1).
// Differential test: every checksum must match GCC bit-for-bit.
#include <stdio.h>

unsigned char G1[4096], G2[4096];
unsigned LEN;

static void copy_g2g(void) {
    for (unsigned i = 0; i < LEN; i++) G2[i] = G1[i];
}

unsigned char A[1024], B[1024];
unsigned N;

static void bump(void) {
    unsigned char *d = B, *s = A;
    for (unsigned i = 0; i < N; i++) *d++ = *s++;
}

unsigned char *after;

static void exituse(void) {
    unsigned char *d = B;
    for (unsigned i = 0; i < N; i++) *d++ = A[i];
    after = d;
}

static void cbound(void) {
    for (unsigned i = 0; i < 512; i++) G2[i] = G1[i];
}

unsigned NN[4] = {100, 200, 300, 400};

static void nested(void) {
    for (unsigned k = 0; k < 4; k++) {
        unsigned n = NN[k];
        for (unsigned i = 0; i < n; i++) G2[i] = G1[i];
    }
}

static void wide(unsigned long n) {
    for (unsigned long i = 0; i < n; i++) G2[i] = G1[i];
}

static unsigned wsum;
static void wideret(unsigned n) {
    unsigned i = 0;
    for (; i < n; i++) G2[i] = G1[i];
    wsum = i;
}

static unsigned long sum(unsigned char *p, unsigned n) {
    unsigned long s = 0;
    for (unsigned i = 0; i < n; i++) s = s * 31 + p[i];
    return s;
}

int main(void) {
    for (int i = 0; i < 4096; i++) G1[i] = (unsigned char)(i * 7 + 3);
    for (int i = 0; i < 1024; i++) A[i] = (unsigned char)(i * 13 + 1);

    LEN = 4000;
    copy_g2g();
    printf("g2g %lu %lu\n", sum(G2, 4000), sum(G1, 4000));

    for (int i = 0; i < 4096; i++) G2[i] = 0xAA;
    LEN = 0;
    copy_g2g();
    printf("zero %lu\n", sum(G2, 4096));

    N = 1000;
    bump();
    printf("bump %lu %lu\n", sum(B, 1000), sum(A, 1000));

    N = 777;
    exituse();
    printf("exituse %lu %ld\n", sum(B, 777), (long)(after - B));

    N = 0;
    exituse();
    printf("exitzero %ld\n", (long)(after - B));

    for (int i = 0; i < 512; i++) G2[i] = 0;
    cbound();
    printf("cbound %lu\n", sum(G2, 512));

    nested();
    printf("nested %lu\n", sum(G2, 400));

    for (int i = 0; i < 512; i++) G2[i] = 0;
    wide(500);
    printf("wide %lu\n", sum(G2, 500));

    wideret(511);
    printf("wideret %u %lu\n", wsum, sum(G2, 511));
    return 0;
}
