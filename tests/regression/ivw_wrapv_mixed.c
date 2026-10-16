/* -fwrapv: retain overflowing arithmetic at 32 bits while independently
   widening safe addresses. Volatile stores must keep their width and value. */
#include <stdio.h>
#define NOINLINE __attribute__((noinline))
NOINLINE static unsigned add_up(const unsigned *p, volatile int *out, int bias) {
    unsigned sum = 0;
    for (int i = (-31); i <= (31); i += (1)) {
        int j = i + bias;
        out[i & 63] = i;
        sum = sum * 33U + p[(unsigned)j & 63U] + p[(i & 63) + 1];
    }
    return sum;
}
NOINLINE static unsigned sub_left_up(const unsigned *p, volatile int *out, int bias) {
    unsigned sum = 0;
    for (int i = (-31); i <= (31); i += (1)) {
        int j = bias - i;
        out[i & 63] = i;
        sum = sum * 33U + p[(unsigned)j & 63U] + p[(i & 63) + 1];
    }
    return sum;
}
NOINLINE static unsigned sub_right_up(const unsigned *p, volatile int *out, int bias) {
    unsigned sum = 0;
    for (int i = (-31); i <= (31); i += (1)) {
        int j = i - bias;
        out[i & 63] = i;
        sum = sum * 33U + p[(unsigned)j & 63U] + p[(i & 63) + 1];
    }
    return sum;
}
NOINLINE static unsigned mul_up(const unsigned *p, volatile int *out, int bias) {
    unsigned sum = 0;
    for (int i = (-31); i <= (31); i += (1)) {
        int j = i * bias;
        out[i & 63] = i;
        sum = sum * 33U + p[(unsigned)j & 63U] + p[(i & 63) + 1];
    }
    return sum;
}
NOINLINE static unsigned add_down(const unsigned *p, volatile int *out, int bias) {
    unsigned sum = 0;
    for (int i = (31); i >= (-31); i += (-1)) {
        int j = i + bias;
        out[i & 63] = i;
        sum = sum * 33U + p[(unsigned)j & 63U] + p[(i & 63) + 1];
    }
    return sum;
}
NOINLINE static unsigned sub_left_down(const unsigned *p, volatile int *out, int bias) {
    unsigned sum = 0;
    for (int i = (31); i >= (-31); i += (-1)) {
        int j = bias - i;
        out[i & 63] = i;
        sum = sum * 33U + p[(unsigned)j & 63U] + p[(i & 63) + 1];
    }
    return sum;
}
NOINLINE static unsigned sub_right_down(const unsigned *p, volatile int *out, int bias) {
    unsigned sum = 0;
    for (int i = (31); i >= (-31); i += (-1)) {
        int j = i - bias;
        out[i & 63] = i;
        sum = sum * 33U + p[(unsigned)j & 63U] + p[(i & 63) + 1];
    }
    return sum;
}
NOINLINE static unsigned mul_down(const unsigned *p, volatile int *out, int bias) {
    unsigned sum = 0;
    for (int i = (31); i >= (-31); i += (-1)) {
        int j = i * bias;
        out[i & 63] = i;
        sum = sum * 33U + p[(unsigned)j & 63U] + p[(i & 63) + 1];
    }
    return sum;
}
static int check_stores(volatile int *out) {
    for (int x = 0; x < 64; ++x) {
        int want = x == 32 ? 0x5a5a5a5a : x < 32 ? x : x - 64;
        if (out[x] != want) return 1;
    }
    return 0;
}
int main(void) {
    unsigned p[65], h = 2166136261U;
    volatile int out[64];
    const int biases[] = { (-2147483647 - 1), -1, 0, 1, 2147483647 };
    for (unsigned i = 0; i < 65; ++i) p[i] = i * 37U + 11U;
    for (unsigned b = 0; b < sizeof(biases) / sizeof(biases[0]); ++b) {
        for (int x = 0; x < 64; ++x) out[x] = 0x5a5a5a5a;
        h = (h ^ add_up(p, out, biases[b])) * 16777619U;
        if (check_stores(out)) return 2;
        for (int x = 0; x < 64; ++x) out[x] = 0x5a5a5a5a;
        h = (h ^ sub_left_up(p, out, biases[b])) * 16777619U;
        if (check_stores(out)) return 2;
        for (int x = 0; x < 64; ++x) out[x] = 0x5a5a5a5a;
        h = (h ^ sub_right_up(p, out, biases[b])) * 16777619U;
        if (check_stores(out)) return 2;
        for (int x = 0; x < 64; ++x) out[x] = 0x5a5a5a5a;
        h = (h ^ mul_up(p, out, biases[b])) * 16777619U;
        if (check_stores(out)) return 2;
        for (int x = 0; x < 64; ++x) out[x] = 0x5a5a5a5a;
        h = (h ^ add_down(p, out, biases[b])) * 16777619U;
        if (check_stores(out)) return 2;
        for (int x = 0; x < 64; ++x) out[x] = 0x5a5a5a5a;
        h = (h ^ sub_left_down(p, out, biases[b])) * 16777619U;
        if (check_stores(out)) return 2;
        for (int x = 0; x < 64; ++x) out[x] = 0x5a5a5a5a;
        h = (h ^ sub_right_down(p, out, biases[b])) * 16777619U;
        if (check_stores(out)) return 2;
        for (int x = 0; x < 64; ++x) out[x] = 0x5a5a5a5a;
        h = (h ^ mul_down(p, out, biases[b])) * 16777619U;
        if (check_stores(out)) return 2;
    }
    if (h != 889073641U) { printf("ivw_wrapv_mixed: FAIL %u\n", h); return 1; }
    puts("ivw_wrapv_mixed: PASS");
    return 0;
}
