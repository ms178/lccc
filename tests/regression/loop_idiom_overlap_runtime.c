/* A forward byte-copy loop is NOT memmove when dst is strictly inside the
 * source range: earlier writes become later reads. Exercise the guarded
 * library-copy fast edge, the scalar overlap edge, zero trips (including
 * null pointers), pointer/IV live-outs, and ELF aliases of one global.
 * The scalar reference is deliberately independent of the compiler pass. */
#include <stdio.h>
#include <string.h>

static unsigned char global_bytes[160];
extern unsigned char published_bytes[160] __attribute__((alias("global_bytes")));

__attribute__((noinline)) static unsigned copy_indexed(
    unsigned char *dst, const unsigned char *src, unsigned n) {
    unsigned i = 0;
    for (; i < n; ++i) dst[i] = src[i];
    return i;
}

__attribute__((noinline)) static unsigned char *copy_bump(
    unsigned char *dst, const unsigned char *src, unsigned n) {
    unsigned char *end = dst;
    for (unsigned i = 0; i < n; ++i) *end++ = src[i];
    return end;
}

__attribute__((noinline)) static void copy_alias(unsigned n) {
    unsigned char *dst = published_bytes + 1;
    for (unsigned i = 0; i < n; ++i) *dst++ = global_bytes[i];
}

__attribute__((noinline)) static void copy_global_to_param(unsigned char *dst,
                                                            unsigned n) {
    for (unsigned i = 0; i < n; ++i) dst[i] = global_bytes[i];
}

__attribute__((noinline)) static void copy_param_to_global(const unsigned char *src,
                                                            unsigned n) {
    for (unsigned i = 0; i < n; ++i) global_bytes[i] = src[i];
}

static unsigned state = 0x251bc7du;
static unsigned rnd(void) {
    state = state * 1664525u + 1013904223u;
    return state ^ (state >> 13);
}

static void fill(unsigned char *dst, unsigned char *ref, unsigned n) {
    for (unsigned i = 0; i < n; i++) dst[i] = ref[i] = (unsigned char)rnd();
}

static void scalar_copy(unsigned char *dst, const unsigned char *src, unsigned n) {
    for (unsigned i = 0; i < n; i++) dst[i] = src[i];
}

static unsigned digest(unsigned acc, const unsigned char *bytes, unsigned n) {
    for (unsigned i = 0; i < n; i++) acc = (acc ^ bytes[i]) * 16777619u;
    return acc;
}

int main(void) {
    unsigned acc = 2166136261u;
    unsigned char actual[160], expected[160];
    /* Every source/destination relation occurs, including exact aliasing,
     * forward overlap, backward overlap, and disjoint arrays. Vary n and
     * stride deterministically to cover zero, one, and non-multiples of W. */
    for (unsigned t = 0; t < 8192; t++) {
        unsigned n = (t % 71u);
        unsigned src = (t / 71u * 13u + t * 17u) % 89u;
        unsigned dst = (t / 19u * 29u + t * 7u) % 89u;
        if ((t & 7u) == 0) dst = src + 1; /* forward smear */
        if ((t & 15u) == 1) dst = src;      /* identical pointers */
        if ((t & 15u) == 2) dst = src ? src - 1 : 0;
        fill(actual, expected, sizeof(actual));
        scalar_copy(expected + dst, expected + src, n);
        if (t & 1u) {
            unsigned char *end = copy_bump(actual + dst, actual + src, n);
            if (end != actual + dst + n) {
                fprintf(stderr, "copy_bump live-out failure: t=%u\n", t);
                return 2;
            }
        } else if (copy_indexed(actual + dst, actual + src, n) != n) {
            fprintf(stderr, "copy_indexed IV live-out failure: t=%u\n", t);
            return 3;
        }
        if (memcmp(actual, expected, sizeof(actual)) != 0) {
            fprintf(stderr, "copy mismatch: t=%u n=%u src=%u dst=%u\n",
                    t, n, src, dst);
            return 4;
        }
        acc = digest(acc, actual, sizeof(actual));
    }

    /* A skipped loop must not invoke a libc function with invalid pointers. */
    if (copy_indexed(NULL, NULL, 0) != 0 || copy_bump(NULL, NULL, 0) != NULL)
        return 5;

    /* Different GlobalAddr names can name the *same* object. Both directions
     * with a global-versus-parameter pointer are also legal without restrict. */
    for (unsigned t = 0; t < 96; ++t) {
        unsigned n = t % 40u;
        fill(global_bytes, expected, sizeof(expected));
        scalar_copy(expected + 1, expected, n);
        copy_alias(n);
        if (memcmp(global_bytes, expected, sizeof(expected)) != 0) {
            fprintf(stderr, "global alias smear mismatch: t=%u\n", t);
            return 6;
        }
        acc = digest(acc, global_bytes, sizeof(global_bytes));

        fill(global_bytes, expected, sizeof(expected));
        scalar_copy(expected + 1, expected, n);
        copy_global_to_param(global_bytes + 1, n);
        if (memcmp(global_bytes, expected, sizeof(expected)) != 0) {
            fprintf(stderr, "global-to-param smear mismatch: t=%u\n", t);
            return 7;
        }
        acc = digest(acc, global_bytes, sizeof(global_bytes));

        fill(global_bytes, expected, sizeof(expected));
        scalar_copy(expected, expected + 1, n);
        copy_param_to_global(global_bytes + 1, n);
        if (memcmp(global_bytes, expected, sizeof(expected)) != 0) {
            fprintf(stderr, "param-to-global backward overlap mismatch: t=%u\n", t);
            return 8;
        }
        acc = digest(acc, global_bytes, sizeof(global_bytes));
    }
    /* Pin the independent scalar result so a miscompile cannot pass merely
     * by printing a different digest without a diagnostic. */
    if (acc != 0x4238867du) {
        fprintf(stderr, "scalar-reference digest mismatch: %08x\n", acc);
        return 9;
    }
    printf("loop-idiom overlap: %08x (8192 randomized + 288 global cases)\n", acc);
    return 0;
}
