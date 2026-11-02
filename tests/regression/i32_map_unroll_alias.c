/* Sequential overlap semantics must survive the wider counted map body. */
#include <stdint.h>
#include <stdio.h>
static uint32_t a[400], b[400];
__attribute__((noinline))
void overlapping_product(uint32_t *out, const uint32_t *x, const uint32_t *y, int n) {
    for (int i = 0; i < n; ++i) out[i] = x[i] * y[i];
}
int main(void) {
    for (int mode = 0; mode < 3; ++mode) {
        for (int offset = -17; offset <= 17; ++offset) {
            for (int n = -3; n <= 97; ++n) {
                uint32_t seed = 0x179b3d21u;
                for (int i = 0; i < 400; ++i) {
                    seed = seed * 1664525u + 1013904223u;
                    a[i] = seed;
                    seed = seed * 1664525u + 1013904223u;
                    b[i] = seed;
                }
                uint32_t *x = a + 128;
                uint32_t *y = mode == 2 ? x : b + 128;
                uint32_t *out = (mode == 1 ? y : x) + offset;
                overlapping_product(out, x, y, n);
                uint64_t hash = 1469598103934665603ull;
                for (int i = 0; i < 400; ++i) {
                    hash = (hash ^ a[i]) * 1099511628211ull;
                    hash = (hash ^ b[i]) * 1099511628211ull;
                }
                printf("%d %d %d %016llx\n", mode, offset, n, (unsigned long long)hash);
            }
        }
    }
    return 0;
}
