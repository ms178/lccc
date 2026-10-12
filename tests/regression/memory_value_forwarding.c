/* Displaced aliases, typed store/load chains, and observable reads.
 * The volatile reference is deliberately independent of the optimizable path. */
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#define NI __attribute__((noinline))

static NI uint64_t displaced(uint64_t *p, uint64_t *q, uint64_t x) {
    *p = x;
    uint64_t observed = q[1];
    *p = x + 1;
    return observed;
}
static NI uint64_t displaced_ref(volatile uint64_t *p,
                                 volatile uint64_t *q, uint64_t x) {
    *p = x;
    uint64_t observed = q[1];
    *p = x + 1;
    return observed;
}
struct fields { uint64_t a, b, c; };
static NI uint64_t fields(struct fields *p, uint64_t x) {
    p->a += x; p->b ^= p->a; p->a += p->b;
    p->c += p->a; p->a ^= p->c; p->b += p->a;
    return p->a ^ p->b ^ p->c;
}
static NI uint64_t fields_ref(volatile struct fields *p, uint64_t x) {
    p->a += x; p->b ^= p->a; p->a += p->b;
    p->c += p->a; p->a ^= p->c; p->b += p->a;
    return p->a ^ p->b ^ p->c;
}
static NI unsigned narrow(uint8_t *p, uint64_t x) { *p = x; return *p; }
static NI unsigned narrow_ref(volatile uint8_t *p, uint64_t x) { *p = x; return *p; }
static NI float round_store(float *p, double x) { *p = x; return *p; }
static NI float round_ref(volatile float *p, double x) { *p = x; return *p; }

int main(void) {
    uint64_t a[16], b[16], seed = UINT64_C(0x123456789abcdef0), checksum = 0;
    for (unsigned round = 0; round < 64; ++round) {
        seed ^= seed << 13; seed ^= seed >> 7; seed ^= seed << 17;
        for (unsigned pi = 1; pi < 14; ++pi) {
            for (unsigned qi = 0; qi < 14; ++qi) {
                for (unsigned i = 0; i < 16; ++i) a[i] = b[i] = seed + i * 37u;
                uint64_t x = displaced(a + pi, a + qi, seed ^ UINT64_C(0xdeadbeef));
                uint64_t y = displaced_ref(b + pi, b + qi, seed ^ UINT64_C(0xdeadbeef));
                if (x != y || memcmp(a, b, sizeof a)) return 1;
                checksum ^= x;
            }
        }
        struct fields f = { seed, seed + 1, ~seed }, g = f;
        uint64_t x = fields(&f, seed), y = fields_ref(&g, seed);
        if (x != y || memcmp(&f, &g, sizeof f)) return 2;
        checksum ^= x;
        uint8_t u, v;
        if (narrow(&u, seed) != narrow_ref(&v, seed) || u != v) return 3;
        float p, q;
        double d = (double)(round * 1234567u) / 7.0;
        if (round_store(&p, d) != round_ref(&q, d) || p != q) return 4;
    }
    printf("memory-value %016llx\n", (unsigned long long)checksum);
    return 0;
}
