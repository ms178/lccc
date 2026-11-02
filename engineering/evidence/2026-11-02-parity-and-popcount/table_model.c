#include <stdio.h>
#include <stdint.h>
/* Models of the emitted RISC-V sequences, transcribed operation for operation. */
static unsigned rv_parity32(unsigned x) {
    unsigned t0 = x, t1, t2;
    t1 = t0 >> 16; t1 = t0 ^ t1;
    t2 = t1 >> 8;  t1 = t1 ^ t2;
    t2 = t1 >> 4;  t1 = t1 ^ t2;
    t1 = t1 & 0xf;
    t2 = 0x6996u;
    t2 = t2 >> t1;
    return t2 & 1;
}
static unsigned rv_parity64(unsigned long x) {
    unsigned long t0 = x, t1, t2;
    t1 = t0 >> 32; t1 = t0 ^ t1;
    t2 = t1 >> 16; t1 = t1 ^ t2;
    t2 = t1 >> 8;  t1 = t1 ^ t2;
    t2 = t1 >> 4;  t1 = t1 ^ t2;
    t1 = t1 & 0xf;
    t2 = 0x6996u;
    t2 = t2 >> t1;
    return (unsigned)(t2 & 1);
}
static unsigned arm_parity32(unsigned x) {
    uint32_t w1 = x ^ (x >> 16);
    w1 = w1 ^ (w1 >> 8);
    w1 = w1 ^ (w1 >> 4);
    w1 &= 0xf;
    uint32_t w2 = 0x6996u;
    w2 = w2 >> w1;
    return w2 & 1;
}
static unsigned rv_popcount32(unsigned x) {
    uint32_t t0 = x, t1, t2, t3;
    t1 = t0 >> 1; t2 = 0x55555555u; t1 &= t2;
    t1 = t0 - t1;
    t2 = 0x33333333u; t3 = t1 >> 2; t3 &= t2; t1 &= t2; t1 += t3;
    t3 = t1 >> 4; t1 += t3;
    t2 = 0x0f0f0f0fu; t1 &= t2;
    t3 = t1 >> 8; t1 += t3;
    t3 = t1 >> 16; t1 += t3;
    return t1 & 0x7f;
}
static unsigned rv_popcount64(unsigned long x) {
    uint64_t t0 = x, t1, t2, t3;
    t1 = t0 >> 1; t2 = 0x5555555555555555ull; t1 &= t2;
    t1 = t0 - t1;
    t2 = 0x3333333333333333ull; t3 = t1 >> 2; t3 &= t2; t1 &= t2; t1 += t3;
    t3 = t1 >> 4; t1 += t3;
    t2 = 0x0f0f0f0f0f0f0f0full; t1 &= t2;
    t3 = t1 >> 8; t1 += t3;
    t3 = t1 >> 16; t1 += t3;
    t3 = t1 >> 32; t1 += t3;
    return (unsigned)(t1 & 0x7f);
}
int main(void) {
    unsigned bad = 0;
    for (unsigned x = 0; x < 65536u; x++) {
        if (rv_parity32(x) != __builtin_parity(x)) { if (bad++ < 3) printf("rvp32 %x\n", x); }
        if (arm_parity32(x) != __builtin_parity(x)) { if (bad++ < 3) printf("armp32 %x\n", x); }
        if (rv_popcount32(x) != (unsigned)__builtin_popcount(x)) { if (bad++ < 3) printf("rvpc32 %x\n", x); }
        if (rv_parity64(x) != __builtin_parityll(x)) bad++;
        if (rv_popcount64(x) != __builtin_popcountll(x)) bad++;
    }
    for (unsigned long i = 0; i < 3000000ul; i++) {
        unsigned long v = (unsigned long)i * 0x9e3779b97f4a7c15ul;
        if (rv_parity64(v) != __builtin_parityll(v)) { if (bad++ < 6) printf("rvp64 %lx\n", v); }
        if (rv_popcount64(v) != (unsigned)__builtin_popcountll(v)) { if (bad++ < 6) printf("rvpc64 %lx\n", v); }
        unsigned u = (unsigned)v ^ (unsigned)(v >> 32);
        if (rv_parity32(u) != __builtin_parity(u)) bad++;
        if (arm_parity32(u) != __builtin_parity(u)) bad++;
        if (rv_popcount32(u) != (unsigned)__builtin_popcount(u)) bad++;
    }
    unsigned long edges[] = {0, 1, ~0ul, 1ul<<63, 0xffffffff00000000ul, 0x00000000fffffffful, 0xaaaaaaaau, 0x55555555u};
    for (unsigned i = 0; i < sizeof edges/sizeof edges[0]; i++) {
        if (rv_parity64(edges[i]) != __builtin_parityll(edges[i])) bad++;
        if (rv_popcount64(edges[i]) != (unsigned)__builtin_popcountll(edges[i])) bad++;
        if (rv_parity32((unsigned)edges[i]) != __builtin_parity((unsigned)edges[i])) bad++;
        if (arm_parity32((unsigned)edges[i]) != __builtin_parity((unsigned)edges[i])) bad++;
        if (rv_popcount32((unsigned)edges[i]) != (unsigned)__builtin_popcount((unsigned)edges[i])) bad++;
    }
    printf("mismatches=%u\n", bad);
    return bad != 0;
}
