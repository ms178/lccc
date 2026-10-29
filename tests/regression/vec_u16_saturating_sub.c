/* Unsigned 16-bit saturation at both vector widths and in scalar tails.
 * Keep the functions externally visible so emission and fallback can be
 * checked independently. The reference updates volatile scalar values and
 * checks the whole surrounding array, including untouched guards. */
#include <stdint.h>
#include <stdio.h>
#include <string.h>

typedef uint16_t word;
#define CAP 137
#define GUARD 4
#define TOTAL (CAP + 2 * GUARD)

__attribute__((noinline))
void sat_ge(word *restrict d, const word *restrict s, unsigned n, word k) {
    for (unsigned i = 0; i < n; ++i) {
        word x = s[i];
        d[i] = (word)(x >= k ? x - k : 0);
    }
}

__attribute__((noinline))
void sat_gt(word *restrict d, const word *restrict s, unsigned n, word k) {
    for (unsigned i = 0; i < n; ++i) {
        word x = s[i];
        d[i] = (word)(x > k ? x - k : 0);
    }
}

__attribute__((noinline))
void sat_lt(word *restrict d, const word *restrict s, unsigned n, word k) {
    for (unsigned i = 0; i < n; ++i) {
        word x = s[i];
        d[i] = (word)(x < k ? 0 : x - k);
    }
}

__attribute__((noinline))
void sat_le(word *restrict d, const word *restrict s, unsigned n, word k) {
    for (unsigned i = 0; i < n; ++i) {
        word x = s[i];
        d[i] = (word)(x <= k ? 0 : x - k);
    }
}

__attribute__((noinline))
void sat_two(word *restrict d, const word *restrict a, const word *restrict b, unsigned n) {
    for (unsigned i = 0; i < n; ++i) {
        word x = a[i], y = b[i];
        d[i] = (word)(x >= y ? x - y : 0);
    }
}

/* Neither a different subtrahend nor a nonzero fallback is saturation. */
__attribute__((noinline))
void different_subtrahend(word *restrict d, const word *restrict s, unsigned n, word k) {
    word other = (word)(k + 1U);
    for (unsigned i = 0; i < n; ++i) {
        word x = s[i];
        d[i] = (word)(x >= k ? x - other : 0);
    }
}

__attribute__((noinline))
void nonzero_fallback(word *restrict d, const word *restrict s, unsigned n, word k) {
    for (unsigned i = 0; i < n; ++i) {
        word x = s[i];
        d[i] = (word)(x >= k ? x - k : 1);
    }
}

__attribute__((noinline))
void signed_compare(word *restrict d, const int16_t *restrict s, unsigned n, int16_t k) {
    for (unsigned i = 0; i < n; ++i) {
        int x = s[i];
        d[i] = (word)(x >= k ? x - k : 0);
    }
}

/* The comparison is wider than the lanes: neither bound may wrap to u16. */
__attribute__((noinline))
void wide_upper_bound(word *restrict d, const word *restrict s, unsigned n) {
    for (unsigned i = 0; i < n; ++i) {
        unsigned x = s[i];
        d[i] = (word)(x >= 65536U ? x - 65536U : 0);
    }
}

__attribute__((noinline))
void wide_negative_bound(word *restrict d, const word *restrict s, unsigned n) {
    for (unsigned i = 0; i < n; ++i) {
        int x = s[i];
        d[i] = (word)(x < -1 ? x + 1 : 42);
    }
}

__attribute__((noinline))
void byte_wide_upper(uint8_t *restrict d, const uint8_t *restrict s, unsigned n) {
    for (unsigned i = 0; i < n; ++i) {
        unsigned x = s[i];
        d[i] = (uint8_t)(x >= 256U ? x - 256U : 0);
    }
}

__attribute__((noinline))
void byte_wide_negative(uint8_t *restrict d, const uint8_t *restrict s, unsigned n) {
    for (unsigned i = 0; i < n; ++i) {
        int x = s[i];
        d[i] = (uint8_t)(x < -1 ? x + 1 : 42);
    }
}

/* Potentially overlapping ranges exercise the vectorizer's alias guard. */
__attribute__((noinline))
void sat_may_alias(word *d, const word *s, unsigned n, word k) {
    for (unsigned i = 0; i < n; ++i) {
        word x = s[i];
        d[i] = (word)(x >= k ? x - k : 0);
    }
}

static word input[TOTAL], other[TOTAL], output[TOTAL], expected[TOTAL];
static int16_t signed_input[TOTAL];
static uint8_t byte_input[TOTAL], byte_output[TOTAL];
static const word edges[] = {0, 1, 2, 16383, 16384, 32767, 32768, 32769, 65534, 65535};
static const int16_t signed_keys[] = {-32768, -32767, -1, 0, 1, 32767};

static void initialize(void) {
    uint32_t state = 0x9e3779b9U;
    for (unsigned i = 0; i < TOTAL; ++i) {
        state = state * 1664525U + 1013904223U;
        input[i] = i < sizeof edges / sizeof edges[0] ? edges[i] : (word)(state >> 8);
        other[i] = (word)(state ^ (state >> 16));
        signed_input[i] = (int16_t)input[i];
        byte_input[i] = (uint8_t)input[i];
    }
}

static int same(const char *name, unsigned n, unsigned k) {
    for (unsigned i = 0; i < TOTAL; ++i) {
        if (output[i] != expected[i]) {
            printf("FAIL %s n=%u k=%u i=%u got=%u want=%u\n", name, n, k, i,
                   (unsigned)output[i], (unsigned)expected[i]);
            return 0;
        }
    }
    return 1;
}

static void blank(void) {
    for (unsigned i = 0; i < TOTAL; ++i)
        output[i] = expected[i] = (word)0xa55aU;
}

int main(void) {
    typedef void (*map_fn)(word *, const word *, unsigned, word);
    static const map_fn maps[] = {sat_ge, sat_gt, sat_lt, sat_le};
    initialize();
    for (unsigned n = 0; n <= CAP; ++n) {
        /* One-element offsets are aligned for word access but not SIMD. */
        for (unsigned off = 0; off < GUARD; ++off) {
            for (unsigned ki = 0; ki < sizeof edges / sizeof edges[0]; ++ki) {
                word k = edges[ki];
                for (unsigned m = 0; m < sizeof maps / sizeof maps[0]; ++m) {
                    blank();
                    for (unsigned i = 0; i < n; ++i) {
                        volatile word x = input[off + i];
                        expected[GUARD + off + i] = (word)(x >= k ? x - k : 0);
                    }
                    maps[m](output + GUARD + off, input + off, n, k);
                    if (!same("saturation", n, k)) return 1;
                }
                blank();
                for (unsigned i = 0; i < n; ++i) {
                    volatile word x = input[off + i];
                    word diff = (word)(k + 1U);
                    expected[GUARD + off + i] = (word)(x >= k ? x - diff : 0);
                }
                different_subtrahend(output + GUARD + off, input + off, n, k);
                if (!same("different-subtrahend", n, k)) return 1;
                blank();
                for (unsigned i = 0; i < n; ++i) {
                    volatile word x = input[off + i];
                    expected[GUARD + off + i] = (word)(x >= k ? x - k : 1);
                }
                nonzero_fallback(output + GUARD + off, input + off, n, k);
                if (!same("nonzero-fallback", n, k)) return 1;
            }
            blank();
            for (unsigned i = 0; i < n; ++i) {
                volatile word a = input[off + i], b = other[off + i];
                expected[GUARD + off + i] = (word)(a >= b ? a - b : 0);
            }
            sat_two(output + GUARD + off, input + off, other + off, n);
            if (!same("two-stream", n, 0)) return 1;
            blank();
            for (unsigned i = 0; i < n; ++i)
                expected[GUARD + off + i] = 0;
            wide_upper_bound(output + GUARD + off, input + off, n);
            if (!same("wide-upper-bound", n, 0)) return 1;
            blank();
            for (unsigned i = 0; i < n; ++i)
                expected[GUARD + off + i] = 42;
            wide_negative_bound(output + GUARD + off, input + off, n);
            if (!same("wide-negative-bound", n, 0)) return 1;
            memset(byte_output, 0xa5, sizeof byte_output);
            byte_wide_upper(byte_output + GUARD + off, byte_input + off, n);
            for (unsigned i = 0; i < TOTAL; ++i) {
                unsigned want = i >= GUARD + off && i < GUARD + off + n ? 0 : 0xa5;
                if (byte_output[i] != want) {
                    printf("FAIL byte-wide-upper n=%u off=%u i=%u\n", n, off, i);
                    return 1;
                }
            }
            memset(byte_output, 0xa5, sizeof byte_output);
            byte_wide_negative(byte_output + GUARD + off, byte_input + off, n);
            for (unsigned i = 0; i < TOTAL; ++i) {
                unsigned want = i >= GUARD + off && i < GUARD + off + n ? 42 : 0xa5;
                if (byte_output[i] != want) {
                    printf("FAIL byte-wide-negative n=%u off=%u i=%u\n", n, off, i);
                    return 1;
                }
            }
            for (unsigned ki = 0; ki < sizeof signed_keys / sizeof signed_keys[0]; ++ki) {
                int16_t k = signed_keys[ki];
                blank();
                for (unsigned i = 0; i < n; ++i) {
                    volatile int x = signed_input[off + i];
                    expected[GUARD + off + i] = (word)(x >= k ? x - k : 0);
                }
                signed_compare(output + GUARD + off, signed_input + off, n, k);
                if (!same("signed", n, (word)k)) return 1;
            }
        }
        /* Sequential reference handles both exact and shifted overlap. */
        for (unsigned mode = 0; mode < 3; ++mode) {
            unsigned src = GUARD + (mode == 2 ? 1 : 0);
            unsigned dst = GUARD + (mode == 1 ? 1 : 0);
            memcpy(output, input, sizeof output);
            memcpy(expected, input, sizeof expected);
            for (unsigned i = 0; i < n; ++i) {
                volatile word x = expected[src + i];
                expected[dst + i] = (word)(x >= 32768U ? x - 32768U : 0);
            }
            sat_may_alias(output + dst, output + src, n, 32768U);
            if (!same("overlap", n, mode)) return 1;
        }
    }
    puts("u16 saturation: PASS");
    return 0;
}
