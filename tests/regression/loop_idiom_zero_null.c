/* Even for provably disjoint parameter+fresh-alloca roots, an unconditional
 * memcpy(dst, NULL, 0) would introduce undefined behavior not present in a
 * skipped scalar loop. The guarded memcpy edge must only run for n > 0. */
#include <stdio.h>

static unsigned char result[64];

__attribute__((noinline)) static void from_local(unsigned char *dst, unsigned n) {
    unsigned char local[64];
    for (unsigned i = 0; i < n; ++i) local[i] = (unsigned char)(i + 1);
    for (unsigned i = 0; i < n; ++i) dst[i] = local[i];
}

__attribute__((noinline)) static unsigned into_local(const unsigned char *src,
                                                       unsigned n) {
    unsigned char local[64];
    for (unsigned i = 0; i < n; ++i) local[i] = src[i];
    return n ? local[n - 1] : 0;
}

int main(void) {
    from_local(NULL, 0);
    if (into_local(NULL, 0)) return 1;
    from_local(result, 16);
    unsigned last = into_local(result, 16);
    if (result[0] != 1 || result[15] != 16 || last != 16) return 2;
    printf("zero-trip null pointers: %u %u %u\n", result[0], result[15], last);
    return 0;
}
