/* An independent SSE1 grant cannot cancel an explicit SSE2 denial. Keep
   scalar memory-to-memory F32 arithmetic usable without packed I32 code. */
#ifndef __SSE__
#error The independent SSE1 grant must be preserved
#endif
#ifdef REQUIRE_SSE2
#ifndef __SSE2__
#error The explicit SSE2 regrant must be preserved
#endif
#else
#ifdef __SSE2__
#error An SSE1 grant must not cancel an explicit SSE2 denial
#endif
#endif
#include <stdint.h>
#include <stdio.h>
#define N 40
#define VALUES 1.f,2.f,3.f,4.f,5.f,6.f,7.f,8.f
#define HALVES .5f,.5f,.5f,.5f,.5f,.5f,.5f,.5f
static const float x[N] = {VALUES,VALUES,VALUES,VALUES,VALUES};
static const float y[N] = {HALVES,HALVES,HALVES,HALVES,HALVES};
static const float expected[8] = {1.5f,2.5f,3.5f,4.5f,5.5f,6.5f,7.5f,8.5f};
static float z[N];
static uint32_t a[N], b[N], out[N];

__attribute__((noinline))
void denied_product_map(uint32_t *restrict d, const uint32_t *restrict p,
                        const uint32_t *restrict q, int n) {
    for (int i = 0; i < n; ++i) d[i] = p[i] * q[i];
}

__attribute__((noinline))
uint32_t denied_product_dot(const uint32_t *p, const uint32_t *q, int n) {
    uint32_t sum = 0;
    for (int i = 0; i < n; ++i) sum += p[i] * q[i];
    return sum;
}

__attribute__((noinline))
void sse1_add(float *restrict d, const float *restrict p, const float *restrict q, int n) {
    for (int i = 0; i < n; ++i) d[i] = p[i] + q[i];
}

int main(void) {
    uint32_t state = 0x179b3d21u;
    for (int i = 0; i < N; ++i) {
        state = state * 1664525u + 1013904223u;
        a[i] = state;
        state = state * 1664525u + 1013904223u;
        b[i] = state;
    }
    for (int n = -3; n <= N; ++n) {
        denied_product_map(out, a, b, n);
        sse1_add(z, x, y, n);
        uint32_t hash = 2166136261u;
        for (int i = 0; i < n; ++i) {
            hash = (hash ^ out[i]) * 16777619u;
            if (z[i] != expected[i & 7]) return 7;
        }
        printf("%d %08x %08x\n", n, hash, denied_product_dot(a, b, n));
    }
    return 0;
}
