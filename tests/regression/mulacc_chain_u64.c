/* mul-accumulate chain fusion (i686): the parser hot shape
 *   res = res * base + (u64)digit
 * with kstrtoull-style overflow gates. The fused head computes the whole
 * expression at the Mul point (3-term zero-high-half product + addend)
 * and dual-stores only the tail; feeder zexts are virtual. Any error in
 * the high-half accounting, feeder liveness, or the add paths shows up
 * as a wrong digest here. Values include wraparound and the 2^32/2^63
 * boundaries the plan resolver reasons about. */
#include <stdio.h>
#include <stdint.h>

static uint64_t parse10(const char *s) {
    uint64_t res = 0;
    while (*s >= '0' && *s <= '9') {
        uint64_t d = (uint64_t)(*s - '0');
        int ov = res >= (~0ULL - d) / 10 + 1 && (res > (~0ULL - d) / 10 || d > (~0ULL - res * 10));
        res = res * 10 + d;
        if (ov) res ^= 0xA5A5A5A5A5A5A5A5ULL; /* perturb, keep going */
        s++;
    }
    return res;
}

/* digit feeder through a byte cast between head and tail (after_head) */
static uint64_t parse16(const char *s) {
    uint64_t res = 0;
    for (;;) {
        char c = *s;
        unsigned v;
        if (c >= '0' && c <= '9') v = (unsigned)(c - '0');
        else if (c >= 'a' && c <= 'f') v = (unsigned)(c - 'a' + 10);
        else break;
        res = res * 16 + (uint64_t)v;
        s++;
    }
    return res;
}

/* constant base at the hi-zero boundary and a u32 feeder */
static uint64_t mix(uint32_t seed) {
    uint64_t h = seed;
    for (int i = 0; i < 8; i++) {
        h = h * 0xFFFFFFFFu + (uint64_t)(seed + (uint32_t)i * 2654435761u);
        h ^= h >> 13;
    }
    return h;
}

int main(void) {
    printf("A=%016llx\n", (unsigned long long)parse10("0"));
    printf("B=%016llx\n", (unsigned long long)parse10("18446744073709551615"));
    printf("C=%016llx\n", (unsigned long long)parse10("18446744073709551616"));
    printf("D=%016llx\n", (unsigned long long)parse10("99999999999999999999"));
    printf("E=%016llx\n", (unsigned long long)parse16("deadbeefcafebabe"));
    printf("F=%016llx\n", (unsigned long long)parse16("0000000000000000"));
    printf("G=%016llx\n", (unsigned long long)mix(0u));
    printf("H=%016llx\n", (unsigned long long)mix(0xFFFFFFFFu));
    printf("I=%016llx\n", (unsigned long long)mix(0x80000000u));
    /* overflow-gate shape: (res & mask) != 0 drives a branch (setCC fusion) */
    uint64_t r = 1;
    for (int i = 0; i < 40; i++) {
        r = r * 10 + (uint64_t)i;
        if ((r & 0x8000000000000000ULL) != 0) r &= 0x7FFFFFFFFFFFFFFFULL;
        if (r >= 1844674407370955161ULL) r /= 10;
    }
    printf("J=%016llx\n", (unsigned long long)r);
    return 0;
}
