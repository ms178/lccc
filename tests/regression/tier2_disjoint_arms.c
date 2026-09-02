#include <stdio.h>
#include <stdint.h>

__attribute__((noinline)) static uint64_t op(uint64_t x) {
    return x * 0x9E3779B97F4A7C15ULL + 0x12345ULL;
}

/* Two mutually exclusive paths (no join -> no phi, so none of these values is
 * "location sensitive"). Each path defines several single-def uint64 values
 * that must survive a long run of opaque calls (compiler cannot keep them all
 * in registers). The paths can never execute together, so their values' live
 * ranges are disjoint across the CFG -> the Tier-2 colorer may share the
 * slots between the two paths' temporaries. */
__attribute__((noinline)) static uint64_t arms(uint64_t seed, int path, int n) {
    uint64_t r;
    if (path == 0) {
        uint64_t a = seed ^ 0xAAAA;
        uint64_t b = seed ^ 0xBBBB;
        uint64_t c = seed ^ 0xCCCC;
        uint64_t d = seed ^ 0xDDDD;
        uint64_t e = seed ^ 0xEEEE;
        uint64_t f = seed ^ 0xFFFF;
        uint64_t g = seed ^ 0x1111;
        uint64_t h = seed ^ 0x2222;
        for (int i = 0; i < n; i++) {
            a = op(a); b = op(b); c = op(c); d = op(d);
            e = op(e); f = op(f); g = op(g); h = op(h);
            if (i % 7 == 0) { a ^= op(b ^ c); }
            if (i % 11 == 0) { e ^= op(f ^ g); }
        }
        r = a ^ b ^ c ^ d ^ e ^ f ^ g ^ h;
    } else {
        uint64_t p = seed ^ 0x1357;
        uint64_t q = seed ^ 0x2468;
        uint64_t s = seed ^ 0x3691;
        uint64_t t = seed ^ 0x4812;
        for (int i = 0; i < n; i++) {
            p = op(p); q = op(q); s = op(s); t = op(t);
            if (i % 5 == 0) { p ^= op(q); }
            if (i % 13 == 0) { s ^= op(t); }
        }
        r = p ^ q ^ s ^ t;
    }
    return r;
}

int main(void) {
    uint64_t h = 0;
    for (int p = 0; p < 2; p++)
        for (int s = 0; s < 8; s++)
            h ^= arms(0xFEEDFACE00000000ULL + s, p, 500);
    printf("arms h=%016llx\n", (unsigned long long)h);
    return 0;
}
