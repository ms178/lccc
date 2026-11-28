/* The names of the AVX-512 integer compares that write a mask.
 *
 * `vpcmp{,u}{b,w,d,q}` takes its predicate as an immediate. The headers
 * name six of the eight predicates (`cmpeq`, `cmpneq`, `cmplt`, `cmple`,
 * `cmpgt`, `cmpge`), for eight element types and three vector widths, and
 * give each a form whose result is ANDed with a mask; lccc had `cmpeq`
 * and the one that takes the predicate as a number. This file calls every
 * one of the 24 * (6 + 6 + 8 + 8) = 672 forms and checks the mask against
 * a compare of the elements one at a time.
 */
#include <immintrin.h>
#include <stdio.h>
#include <string.h>

static int failures;
static unsigned long long calls;

static void check(const char *name, unsigned long long got, unsigned long long want)
{
    calls++;
    if (got != want) {
        if (failures < 20)
            printf("FAIL %s: %#llx, expected %#llx\n", name, got, want);
        failures++;
    }
}

/* The mask of `bytes / size` lanes: `predicate` is a `_MM_CMPINT_*`. */
static unsigned long long model(const unsigned char *a, const unsigned char *b, int bytes, int size,
                                int is_signed, int predicate)
{
    unsigned long long mask = 0;
    for (int lane = 0; lane < bytes / size; lane++) {
        unsigned long long x = 0, y = 0;
        memcpy(&x, a + lane * size, (size_t)size);
        memcpy(&y, b + lane * size, (size_t)size);
        if (is_signed && size < 8) {
            unsigned long long sign = 1ULL << (8 * size - 1);
            x = (x ^ sign) - sign;
            y = (y ^ sign) - sign;
        }
        int eq = x == y;
        int lt = is_signed ? (long long)x < (long long)y : x < y;
        int r;
        switch (predicate) {
        case 0: r = eq; break;
        case 1: r = lt; break;
        case 2: r = lt || eq; break;
        case 3: r = 0; break;
        case 4: r = !eq; break;
        case 5: r = !lt; break;
        case 6: r = !lt && !eq; break;
        default: r = 1; break;
        }
        mask |= (unsigned long long)r << lane;
    }
    return mask;
}

/* X(element, bytes in one, 1 if signed) */
#define ELEMENTS(X) \
    X(epi8, 1, 1) X(epu8, 1, 0) X(epi16, 2, 1) X(epu16, 2, 0) \
    X(epi32, 4, 1) X(epu32, 4, 0) X(epi64, 8, 1) X(epu64, 8, 0)

/* One width: `prefix` is `_mm512_`, `_mm256_` or `_mm_`. */
#define NAMED(prefix, bytes, element, size, sgn, name, number)                                   \
    check(#prefix #name "_" #element "_mask", prefix##name##_##element##_mask(va, vb),          \
          model(a, b, bytes, size, sgn, number));                                                \
    check(#prefix "mask_" #name "_" #element "_mask",                                            \
          prefix##mask_##name##_##element##_mask(k, va, vb),                                     \
          k & model(a, b, bytes, size, sgn, number));

#define NUMBERED(prefix, bytes, element, size, sgn, number)                                      \
    check(#prefix "cmp_" #element "_mask " #number, prefix##cmp_##element##_mask(va, vb, number), \
          model(a, b, bytes, size, sgn, number));                                                \
    check(#prefix "mask_cmp_" #element "_mask " #number,                                         \
          prefix##mask_cmp_##element##_mask(k, va, vb, number),                                  \
          k & model(a, b, bytes, size, sgn, number));

#define EVERY(prefix, bytes, element, size, sgn)             \
    NAMED(prefix, bytes, element, size, sgn, cmpeq, 0)       \
    NAMED(prefix, bytes, element, size, sgn, cmplt, 1)       \
    NAMED(prefix, bytes, element, size, sgn, cmple, 2)       \
    NAMED(prefix, bytes, element, size, sgn, cmpneq, 4)      \
    NAMED(prefix, bytes, element, size, sgn, cmpge, 5)       \
    NAMED(prefix, bytes, element, size, sgn, cmpgt, 6)       \
    NUMBERED(prefix, bytes, element, size, sgn, 0)           \
    NUMBERED(prefix, bytes, element, size, sgn, 1)           \
    NUMBERED(prefix, bytes, element, size, sgn, 2)           \
    NUMBERED(prefix, bytes, element, size, sgn, 3)           \
    NUMBERED(prefix, bytes, element, size, sgn, 4)           \
    NUMBERED(prefix, bytes, element, size, sgn, 5)           \
    NUMBERED(prefix, bytes, element, size, sgn, 6)           \
    NUMBERED(prefix, bytes, element, size, sgn, 7)

__attribute__((noinline)) static void width512(const unsigned char *a, const unsigned char *b,
                                               unsigned long long k)
{
    __m512i va = _mm512_loadu_si512((const void *)a), vb = _mm512_loadu_si512((const void *)b);
#define X(element, size, sgn) EVERY(_mm512_, 64, element, size, sgn)
    ELEMENTS(X)
#undef X
}

__attribute__((noinline)) static void width256(const unsigned char *a, const unsigned char *b,
                                               unsigned long long k)
{
    __m256i va = _mm256_loadu_si256((const void *)a), vb = _mm256_loadu_si256((const void *)b);
#define X(element, size, sgn) EVERY(_mm256_, 32, element, size, sgn)
    ELEMENTS(X)
#undef X
}

__attribute__((noinline)) static void width128(const unsigned char *a, const unsigned char *b,
                                               unsigned long long k)
{
    __m128i va = _mm_loadu_si128((const void *)a), vb = _mm_loadu_si128((const void *)b);
#define X(element, size, sgn) EVERY(_mm_, 16, element, size, sgn)
    ELEMENTS(X)
#undef X
}

int main(void)
{
    if (!__builtin_cpu_supports("avx512f") || !__builtin_cpu_supports("avx512bw")
        || !__builtin_cpu_supports("avx512dq") || !__builtin_cpu_supports("avx512vl")) {
        puts("mask compares: no AVX-512 F, BW, DQ and VL on this machine");
        return 0;
    }

    unsigned char a[64], b[64];
    static const unsigned char edge[8] = {0x00, 0x01, 0x7f, 0x80, 0x81, 0xfe, 0xff, 0x40};
    unsigned state = 0x2545f491u;
    for (int round = 0; round < 6; round++) {
        for (int i = 0; i < 64; i++) {
            state = state * 1664525u + 1013904223u;
            unsigned r = state >> 9;
            switch (round) {
            case 0: /* unrelated bytes */
                a[i] = (unsigned char)r;
                b[i] = (unsigned char)(r >> 8);
                break;
            case 1: /* the same vector twice */
                a[i] = b[i] = (unsigned char)r;
                break;
            case 2: /* the bytes where signed and unsigned order differ */
                a[i] = edge[r & 7];
                b[i] = edge[(r >> 3) & 7];
                break;
            case 3: /* equal but for the lowest byte of some elements */
                a[i] = (unsigned char)r;
                b[i] = (unsigned char)(i % 8 == 0 && (r & 0x300) ? r + 1 : r);
                break;
            case 4: /* equal but for the highest byte of some elements */
                a[i] = (unsigned char)r;
                b[i] = (unsigned char)(i % 8 == 7 && (r & 0x300) ? r ^ 0x80 : r);
                break;
            default: /* whole 64-bit elements equal, the others unrelated */
                a[i] = (unsigned char)r;
                b[i] = (unsigned char)((i / 8) % 2 ? r >> 8 : r);
                break;
            }
        }
        static const unsigned long long masks[3] = {0xffffffffffffffffULL, 0, 0xa5c3f00f96695a3cULL};
        for (int m = 0; m < 3; m++) {
            width512(a, b, masks[m]);
            width256(a + round, b + round, masks[m]);
            width128(a + 3 * round, b + 3 * round, masks[m]);
        }
    }
    printf("mask compares: %llu calls, %d wrong\n", calls, failures);
    return failures != 0;
}
