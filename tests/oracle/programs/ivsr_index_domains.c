/* Index-domain semantics for strength reduction (IVSR-DOMAIN-1/2).
 *
 * A pointer recurrence `p += step*stride` is only equivalent to recomputing
 * `p = base + f(i)` when `f` is linear over the values `i` actually takes.
 * Three independent ways to break that linearity are pinned here, each with an
 * asymmetric answer so a compiler cannot pass by cancelling its own error:
 *
 *   1. an unsigned 32-bit counter READ THROUGH a signed index — the C17
 *      6.3.1.3p3 conversion is implementation-defined at the wrap, and every
 *      hosted two's-complement target must produce the negative subscript;
 *   2. the same counter used WITHOUT a signedness change, wrapping through
 *      UINT32_MAX, where the zero-extension is linear but the wrap is not;
 *   3. a SIGNED counter whose increment is spelled in unsigned arithmetic,
 *      where the wrap is implementation-defined rather than undefined, so the
 *      "signed overflow is UB" theorem does not license the recurrence.
 *
 * Case 2 is exercised here only over the non-wrapping prefix; the wrapping
 * form needs a 16 GiB sparse mapping and lives in
 * tests/regression/ivsr_unsigned_sparse_wrap.c.  Every case prints a checksum
 * derived from the ACTUAL subscripts visited, not from the loop trip count. */
#include <stdint.h>
#include <stdio.h>

#define ALEN 8u
static unsigned a[ALEN] = {17u, 29u, 43u, 71u, 97u, 131u, 173u, 211u};

/* (1) unsigned counter, signed subscript, crossing the 32-bit wrap. */
__attribute__((noinline)) static uint64_t signed_view(const unsigned *p,
                                                      uint32_t i,
                                                      uint32_t stop) {
    uint64_t s = 0, seen = 0;
    do {
        int32_t idx = (int32_t)i;
        s += (uint64_t)p[idx] * (seen + 1u);
        seen += (uint64_t)(idx < 0 ? -idx : idx);
        ++i;
    } while (i != stop);
    return s * 1000003u + seen;
}

/* (1b) the 64-bit spelling of the same discontinuity. */
__attribute__((noinline)) static uint64_t signed_view64(const unsigned *p,
                                                        uint64_t i,
                                                        uint64_t stop) {
    uint64_t s = 0, seen = 0;
    do {
        int64_t idx = (int64_t)i;
        s += (uint64_t)p[idx] * (seen + 1u);
        seen += (uint64_t)(idx < 0 ? -idx : idx);
        ++i;
    } while (i != stop);
    return s * 1000003u + seen;
}

/* (2) unsigned counter, unsigned subscript, non-wrapping prefix. */
__attribute__((noinline)) static uint64_t plain_view(const unsigned *p,
                                                     uint32_t i,
                                                     uint32_t n) {
    uint64_t s = 0;
    for (uint32_t k = 0; k < n; ++k, ++i) s += (uint64_t)p[i] * (k + 1u);
    return s;
}

/* (3) signed counter incremented in unsigned arithmetic. */
__attribute__((noinline)) static uint64_t unsigned_step(int32_t i, int32_t n) {
    uint64_t s = 0;
    for (int32_t k = 0; k < n; ++k) {
        s += (uint64_t)a[(i & 7)] * (uint64_t)(k + 1);
        i = (int32_t)((uint32_t)i + 1u);
    }
    return s;
}

/* (4) affine offset `(iv + k0) + 0x80000000` with the same signed/unsigned
 * discontinuity. The induction variable MUST be the loop counter: an earlier
 * version indexed `p[i + 0x80000000]` with `i` a loop-invariant parameter, so it
 * read ONE element n times and exercised nothing about IVSR's affine path
 * however the header described it. With `i = 0x80000000` the subscript is
 * `(0x80000000 + k) + 0x80000000 == k` modulo 2^32, so the caller's n <= 6 keeps
 * every access inside the 8-element array while the compiler still sees the full
 * affine unsigned form. */
__attribute__((noinline)) static uint64_t affine_view(const unsigned *p,
                                                      uint32_t i,
                                                      uint32_t n) {
    uint64_t s = 0;
    for (uint32_t k = 0; k < n; ++k)
        s += (uint64_t)p[(i + k) + UINT32_C(0x80000000)] * (k + 1u);
    return s;
}

int main(void) {
    uint64_t t = 0;
    for (unsigned n = 1u; n <= 6u; ++n) {
        t = t * 31u + signed_view(a + 2, UINT32_MAX - 1u, n - 2u);
        t = t * 31u + signed_view64(a + 2, UINT64_MAX - 1u, (uint64_t)n - 2u);
        t = t * 31u + plain_view(a, 0u, n);
        t = t * 31u + unsigned_step(INT32_MAX - 3, (int32_t)n);
        t = t * 31u + affine_view(a, UINT32_C(0x80000000), n);
    }
    printf("ivsr_index_domains %llu\n", (unsigned long long)t);
    return 0;
}
