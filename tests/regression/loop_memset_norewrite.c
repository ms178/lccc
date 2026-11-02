// Loop-memset near-misses: these shapes must KEEP their loop (the pass
// refuses them) and stay correct. Differential vs GCC.
#include <stdio.h>

unsigned char G[4096];
unsigned int W[1024];
unsigned LEN;
unsigned char OTHER[4096];

static unsigned long sum8(const unsigned char *p, unsigned n) {
    unsigned long s = 0;
    for (unsigned i = 0; i < n; i++) s += p[i];
    return s;
}

// stored value is computed, not constant: not a fill
static void computed(unsigned n) {
    for (unsigned i = 0; i < n; i++) G[i] = (unsigned char)(i * 7);
}

// stored value is a load: copy loop (loop_idiom's memcpy job, not ours)
static void copyloop(unsigned n) {
    for (unsigned i = 0; i < n; i++) G[i] = OTHER[i];
}

// strided fill: GEP scale != store width, gaps are never written
static unsigned int stride(void) {
    for (unsigned i = 0; i < 256; i++) W[i * 2] = 0;
    return W[1];
}

int main(void) {
    for (int i = 0; i < 4096; i++) G[i] = (unsigned char)(i ^ 0x3C);
    for (int i = 0; i < 4096; i++) OTHER[i] = (unsigned char)(i * 5 + 1);
    for (int i = 0; i < 1024; i++) W[i] = 0x12345678u;

    LEN = 4000;
    computed(LEN);
    printf("computed %lu\n", sum8(G, 4000));

    copyloop(LEN);
    printf("copy %lu\n", sum8(G, 4000));

    printf("stride %u\n", stride());
    return 0;
}
