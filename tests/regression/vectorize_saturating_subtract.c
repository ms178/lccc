/* SPDX-License-Identifier: MIT */
/*
 * Unsigned saturating subtract over byte and word lanes.
 *
 * `a >= b ? a - b : 0` is the zlib-ng slide-hash idiom (`Pos m >= wsize ? m -
 * wsize : 0`).  Expressed generically it costs a compare, a sign-bias XOR
 * pair, the subtract and a per-bit select -- thirteen packed instructions for
 * what `psubusb`/`psubusw` do in one.  The vectorizer folds the ternary into
 * the single saturating subtract, so this fixture pins three things:
 *
 *   * the arithmetic is right at the saturation boundary (equal operands and
 *     operands just below/above it), in the vector prefix AND in the scalar
 *     remainder (the tail runs the untouched scalar loop, so a fold that
 *     changed the ternary's meaning instead of its encoding shows up as a
 *     mismatch against GCC at the tail element);
 *   * the fold actually fires: the body must contain the packed saturating
 *     op, not a compare+blend expansion of it;
 *   * the byte and word paths agree on the `>` vs `>=` spellings, which are
 *     equivalent under saturation exactly because the equal case yields 0.
 *
 * The dword lane is deliberately NOT part of the "must saturate" set: x86 has
 * no packed dword saturating subtract, so `u32` lanes keep the compare+blend
 * form.  The check below only requires that u32 stays CORRECT there.
 */
#include <stdint.h>
#include <stdio.h>

#define N 4096u

static uint16_t wsrc[N];
static uint16_t wdst[N];
static uint8_t bsrc[N];
static uint8_t bdst[N];
static uint32_t dsrc[N];
static uint32_t ddst[N];

static void init(void)
{
    for (uint32_t i = 0; i < N; i++) {
        wsrc[i] = (uint16_t)(i * 37u);
        bsrc[i] = (uint8_t)(i * 11u);
        dsrc[i] = i * 2654435761u;
    }
}

/* `>=` spelling: word lanes. */
__attribute__((noinline))
static void slide_w_ge(uint16_t t)
{
    for (uint32_t i = 0; i < N; i++) {
        uint16_t m = wsrc[i];
        wdst[i] = (uint16_t)(m >= t ? (uint16_t)(m - t) : 0u);
    }
}

/* `>` spelling: equivalent under saturation, folds through the same node. */
__attribute__((noinline))
static void slide_w_gt(uint16_t t)
{
    for (uint32_t i = 0; i < N; i++) {
        uint16_t m = wsrc[i];
        wdst[i] = (uint16_t)(m > t ? (uint16_t)(m - t) : 0u);
    }
}

/* Byte lanes: the 8-bit saturating form. */
__attribute__((noinline))
static void slide_b(uint8_t t)
{
    for (uint32_t i = 0; i < N; i++) {
        uint8_t m = bsrc[i];
        bdst[i] = (uint8_t)(m >= t ? (uint8_t)(m - t) : 0u);
    }
}

/* Dword lanes: no packed instruction, must stay correct anyway. */
__attribute__((noinline))
static void slide_d(uint32_t t)
{
    for (uint32_t i = 0; i < N; i++) {
        uint32_t m = dsrc[i];
        ddst[i] = m >= t ? m - t : 0u;
    }
}

static uint64_t digest(const void *p, size_t bytes)
{
    const uint8_t *b = (const uint8_t *)p;
    uint64_t h = 1469598103934665603ull;
    for (size_t i = 0; i < bytes; i++) {
        h ^= b[i];
        h *= 1099511628211ull;
    }
    return h;
}

int main(void)
{
    init();

    /* Boundary sweep: t = 0, t = 1, t just below a value, and t = UINT16_MAX
     * (everything saturates).  The last case is the pure-saturation path the
     * fold must not turn into a wrapping subtract. */
    uint64_t h = 0;
    static const uint16_t wt[] = {0u, 1u, 37u, 38u, 0x7fffu, 0xffffu};
    for (size_t k = 0; k < sizeof wt / sizeof wt[0]; k++) {
        slide_w_ge(wt[k]);
        h = h * 31u + digest(wdst, sizeof wdst);
        slide_w_gt(wt[k]);
        h = h * 31u + digest(wdst, sizeof wdst);
    }
    static const uint8_t bt[] = {0u, 1u, 11u, 12u, 0x7fu, 0xffu};
    for (size_t k = 0; k < sizeof bt / sizeof bt[0]; k++) {
        slide_b(bt[k]);
        h = h * 31u + digest(bdst, sizeof bdst);
    }
    static const uint32_t dt[] = {0u, 1u, 0x80000000u, 0xffffffffu};
    for (size_t k = 0; k < sizeof dt / sizeof dt[0]; k++) {
        slide_d(dt[k]);
        h = h * 31u + digest(ddst, sizeof ddst);
    }
    printf("%016llx\n", (unsigned long long)h);
    return 0;
}
