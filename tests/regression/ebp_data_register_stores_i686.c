/* i686 under -fomit-frame-pointer allocates %ebp like any callee-saved
 * register. The peephole read every `(%ebp)` operand as a frame slot, so in
 * main's init loop -- `&a[i]` and then `&b[i]` homed in %ebp -- the windowed
 * dead-store pass deleted `movl %edi, (%ebp)` (the a[i] store) because the
 * b[i] store "overwrote the same slot": a[] stayed zero. This is benchmark
 * i32_product_map (main at -m32 -O1 printed the zero-input hash) with the
 * pass count cut to 3; the hash is compared against the reference compiler
 * and `verify` re-derives every element independently. Its allocation is
 * pinned by the probe that the unfixed compiler fails it. */
#include <stdint.h>
#include <stdio.h>
#define PRODUCT_MAP_PASSES 3
#define N 2048
static uint32_t a[N], b[N], out[N];

__attribute__((noinline))
static void product_map(uint32_t *restrict d, const uint32_t *restrict x,
                        const uint32_t *restrict y, int n) {
    for (int i = 0; i < n; ++i) d[i] = x[i] * y[i];
}

__attribute__((noinline)) static int verify(void) {
    uint32_t state = 0x179b3d21u, a2[N], b2[N];
    for (int i = 0; i < N; ++i) {
        state = state * 1664525u + 1013904223u;
        a2[i] = state;
        state = state * 1664525u + 1013904223u;
        b2[i] = state;
    }
    for (unsigned pass = 0; pass < PRODUCT_MAP_PASSES; ++pass) a2[pass & (N - 1)] ^= pass;
    int bad = 0;
    for (int i = 0; i < N; ++i) bad += (a[i] != a2[i]) + (b[i] != b2[i]) + (out[i] != a2[i] * b2[i]);
    return bad;
}

int main(void) {
    uint32_t state = 0x179b3d21u;
    for (int i = 0; i < N; ++i) {
        state = state * 1664525u + 1013904223u;
        a[i] = state;
        state = state * 1664525u + 1013904223u;
        b[i] = state;
    }
    for (unsigned pass = 0; pass < PRODUCT_MAP_PASSES; ++pass) {
        a[pass & (N - 1)] ^= pass;
        product_map(out, a, b, N);
    }
    uint64_t hash = 1469598103934665603ull;
    for (int i = 0; i < N; ++i) hash = (hash ^ out[i]) * 1099511628211ull;
    printf("%016llx\n", (unsigned long long)hash);
    int bad = verify();
    printf("%s %d\n", bad ? "FAILED" : "ok", bad);
    return bad != 0;
}
