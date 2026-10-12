#include <stdint.h>
#include <stdio.h>
#define NI __attribute__((noinline))
static NI int member(int x) { if (x == 45 || x == 46) return 7; return 3; }
static NI int nonmember(int x) { if (x != 46 && x != 45) return 3; return 7; }
static NI int reversed(int x) { if (46 == x || 45 == x) return 7; return 3; }
static NI int signed_pair(int8_t x) { if (x == -1 || x == 0) return 7; return 3; }
static NI int high_pair(uint64_t x) {
    if (x == UINT64_MAX - 1 || x == UINT64_MAX) return 7;
    return 3;
}
static volatile unsigned effects;
static NI int guarded(int x) {
    if (x == 45 || (++effects && x == 46)) return 7;
    return 3;
}
static NI int guarded_range(int x) {
    return x >= 10 && (++effects, x <= 20);
}
int main(void) {
    volatile unsigned char expected[256] = {0};
    expected[45] = expected[46] = 1;
    for (int x = -512; x < 512; ++x) {
        int want = x >= 0 && x < 256 && expected[x] ? 7 : 3;
        if (member(x) != want || nonmember(x) != want || reversed(x) != want) return 1;
        effects = 0;
        if (guarded(x) != want || effects != (unsigned)(x != 45)) return 2;
    }
    for (int x = -512; x < 512; ++x) {
        effects = 0;
        if (guarded_range(x) != ((unsigned)x - 10u <= 10u)
            || effects != (unsigned)(x >= 10)) return 5;
    }
    expected[45] = expected[46] = 0;
    expected[0] = expected[255] = 1;
    for (unsigned i = 0; i < 256; ++i)
        if (signed_pair((int8_t)i) != (expected[i] ? 7 : 3)) return 3;
    const uint64_t inputs[] = {0, 1, 45, 46, UINT64_MAX - 2, UINT64_MAX - 1, UINT64_MAX,
                              UINT64_C(0x7fffffffffffffff), UINT64_C(0x8000000000000000)};
    volatile int results[] = {3, 3, 3, 3, 3, 7, 7, 3, 3};
    for (unsigned i = 0; i < sizeof inputs / sizeof inputs[0]; ++i)
        if (high_pair(inputs[i]) != results[i]) return 4;
    puts("adjacent membership: PASS");
    return 0;
}
