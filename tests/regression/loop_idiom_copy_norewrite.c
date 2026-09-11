// Loop-idiom near misses: these loops must stay loops (CCC_LOOP_IDIOM=1)
// yet remain correct. Differential test vs GCC; the no-rewrite decision
// itself is pinned by tests/regression/check_loop_idiom.sh.
#include <stdio.h>

unsigned char G1[512], G2[512];
unsigned N;

// Same-address copy: same object root, must not match.
static void selfsame(unsigned n) {
    for (unsigned i = 0; i < n; i++) G1[i] = G1[i];
}

// Forward overlap (dst > src): the loop smears bytes; neither memcpy
// nor memmove semantics. Must not match.
static void overlap(unsigned n) {
    for (unsigned i = 0; i < n; i++) G1[i + 1] = G1[i];
}

unsigned sum;
// Extra loop-carried integer state: must not match.
static void copysum(void) {
    unsigned s = 0;
    for (unsigned i = 0; i < N; i++) {
        G2[i] = G1[i];
        s += G1[i];
    }
    sum = s;
}

// Parameter roots: v1 has no cross-call noalias proof, must not match.
static void copy_p2p(unsigned char *d, unsigned char *s, unsigned n) {
    for (unsigned i = 0; i < n; i++) d[i] = s[i];
}

// Two-condition test: not a single `Ult(iv, bound)`, must not match.
static void twocond(unsigned n) {
    for (unsigned i = 0; i < n && i < 512; i++) G2[i] = G1[i];
}

static unsigned long sum8(unsigned char *p, unsigned n) {
    unsigned long s = 0;
    for (unsigned i = 0; i < n; i++) s = s * 31 + p[i];
    return s;
}

int main(void) {
    for (int i = 0; i < 512; i++) G1[i] = (unsigned char)(i * 3 + 5);
    N = 500;

    selfsame(512);
    printf("selfsame %lu\n", sum8(G1, 512));

    for (int i = 0; i < 512; i++) G1[i] = (unsigned char)i;
    overlap(100);
    printf("overlap %lu %u %u\n", sum8(G1, 110), G1[0], G1[100]);

    for (int i = 0; i < 512; i++) {
        G1[i] = (unsigned char)(i + 1);
        G2[i] = 0;
    }
    copysum();
    printf("copysum %u %lu\n", sum, sum8(G2, 500));

    for (int i = 0; i < 512; i++) G2[i] = 0;
    copy_p2p(G2, G1, 512);
    printf("p2p %lu\n", sum8(G2, 512));

    for (int i = 0; i < 512; i++) G2[i] = 0;
    twocond(600);
    printf("twocond %lu\n", sum8(G2, 512));
    return 0;
}
