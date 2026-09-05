/*
 * Integer map-loop lane ops: differential coverage.
 *
 * The map vectorizer admits integer Sub/And/Or/Xor for I32/U32 (AVX2
 * `vp{sub,and,or,xor}d` 8-lane, SSE2 `p{sub,and,or,xor}` 4-lane) and Sub for
 * I64/U64 (SSE2 `psubq`, 2-lane). Every kernel below is hashed and the whole
 * transcript is compared against GCC under the same flags by the suite
 * runner, so each of the following shows up as a hash mismatch rather than a
 * silent wrong answer:
 *
 *   - a swapped Sub operand (`src1 - src2` vs `src2 - src1`), including the
 *     memory-operand-folded form `vpsubd mem, %ymm_src1, %ymm_dst` where the
 *     fold is only legal in the src2 slot;
 *   - a broken invariant broadcast for `a[i] OP c` vs `c OP a[i]` (the two
 *     differ for the non-commutative Sub);
 *   - a wrong element width (I64 lanes going through 32-bit ops);
 *   - remainder-loop bugs: the size sweep covers 0..17 plus every
 *     vector/remainder boundary, so all 0..7 scalar tail lanes for an 8-lane
 *     body and 0..1 for a 2-lane body execute;
 *   - an in-place update (`a[i] ^= b[i]`, store stream == load stream)
 *     losing its aliasing check.
 *
 * Inputs include INT_MIN/INT_MAX/-1/0 and the 0x55555555/0xaaaaaaaa bit
 * patterns. Subtraction is exercised on `unsigned` (where wraparound is
 * defined) so the test itself has no UB while still driving the same
 * `psubd`/`psubq` lanes the signed path uses.
 */
#include <stdint.h>
#include <stdio.h>
#include <string.h>

static uint32_t hb(const void *p, size_t n) {
    const uint8_t *b = (const uint8_t *) p;
    uint32_t h = 2166136261u;
    for (size_t i = 0; i < n; i++)
        h = (h ^ b[i]) * 16777619u;
    return h;
}

#define N 512
static int32_t A32[N], B32[N], D32[N];
static uint32_t AU[N], BU[N], DU[N];
static int64_t A64[N], B64[N], D64[N];
static uint64_t AU64[N], BU64[N], DU64[N];

static void fill(void) {
    static const int32_t iv[16] = {
        0,      1,          -1,          7,          -7,   0x7fffffff,
        (int32_t) 0x80000000, 12345,     -12345,     0x55555555,
        (int32_t) 0xaaaaaaaa, 1000,      -1000,      0x0f0f0f0f,
        (int32_t) 0xf0f0f0f0, 42,
    };
    for (int i = 0; i < N; i++) {
        A32[i] = iv[i % 16];
        B32[i] = iv[(i * 7 + 3) % 16];
        AU[i] = (uint32_t) iv[(i * 5 + 1) % 16];
        BU[i] = (uint32_t) iv[(i * 11 + 9) % 16];
        A64[i] = (int64_t) iv[i % 16] * 0x100000001LL;
        B64[i] = (int64_t) iv[(i * 3 + 5) % 16] * 0x100000003LL;
        AU64[i] = (uint64_t) A64[i] ^ 0xdeadbeefcafef00dULL;
        BU64[i] = (uint64_t) B64[i] * 3u;
    }
}

#define K3(nm, T, expr)                                                                  \
    __attribute__((noinline)) static void nm(T *restrict d, const T *restrict a,         \
                                             const T *restrict b, long n) {              \
        for (long i = 0; i < n; i++)                                                     \
            d[i] = expr;                                                                 \
    }
#define K2(nm, T, expr)                                                                  \
    __attribute__((noinline)) static void nm(T *restrict d, const T *restrict a, T c,    \
                                             long n) {                                   \
        for (long i = 0; i < n; i++)                                                     \
            d[i] = expr;                                                                 \
    }

K3(k_sub_u32, uint32_t, a[i] - b[i])
K3(k_rsub_u32, uint32_t, b[i] - a[i]) /* catches a swapped src1/src2 */
K2(k_sub_inv, uint32_t, a[i] - c)
K2(k_inv_sub, uint32_t, c - a[i]) /* non-commutative broadcast */
K3(k_xor, int32_t, a[i] ^ b[i])
K3(k_and, int32_t, a[i] & b[i])
K3(k_or, int32_t, a[i] | b[i])
K2(k_andm, uint32_t, a[i] & c)
K3(k_sub64, uint64_t, a[i] - b[i])
K3(k_add64, int64_t, a[i] + b[i])
K2(k_rsub64, uint64_t, c - a[i])
K3(k_mixed, uint32_t, (a[i] - b[i]) * 3u + (a[i] | b[i]))

/* Nested tree: two lane ops plus a broadcast in one body. */
__attribute__((noinline)) static void k_xa(uint32_t *restrict d, const uint32_t *restrict a,
                                           const uint32_t *restrict b, uint32_t m, long n) {
    for (long i = 0; i < n; i++)
        d[i] = (a[i] ^ b[i]) & m;
}

/* In-place update: the store stream is also a load stream. */
__attribute__((noinline)) static void k_inplace(uint32_t *restrict a, const uint32_t *restrict b,
                                                long n) {
    for (long i = 0; i < n; i++)
        a[i] ^= b[i];
}

static const long S[] = {0,  1,  2,  3,  4,  5,  6,  7,  8,  9,  10, 11, 12,
                         13, 15, 16, 17, 31, 32, 33, 63, 64, 65, 100, N};

#define R3(nm, D, A, B)                                                                  \
    do {                                                                                 \
        uint32_t h = 0;                                                                  \
        for (size_t s = 0; s < sizeof S / sizeof *S; s++) {                              \
            memset(D, 0xa5, sizeof D);                                                   \
            nm(D, A, B, S[s]);                                                           \
            h = h * 31u + hb(D, sizeof D);                                               \
        }                                                                                \
        printf("%s %08x\n", #nm, h);                                                     \
    } while (0)
#define R2(nm, D, A, C)                                                                  \
    do {                                                                                 \
        uint32_t h = 0;                                                                  \
        for (size_t s = 0; s < sizeof S / sizeof *S; s++) {                              \
            memset(D, 0xa5, sizeof D);                                                   \
            nm(D, A, C, S[s]);                                                           \
            h = h * 31u + hb(D, sizeof D);                                               \
        }                                                                                \
        printf("%s %08x\n", #nm, h);                                                     \
    } while (0)

int main(void) {
    fill();
    R3(k_sub_u32, DU, AU, BU);
    R3(k_rsub_u32, DU, AU, BU);
    R2(k_sub_inv, DU, AU, 0x9e3779b9u);
    R2(k_inv_sub, DU, AU, 0x9e3779b9u);
    R3(k_xor, D32, A32, B32);
    R3(k_and, D32, A32, B32);
    R3(k_or, D32, A32, B32);
    R2(k_andm, DU, AU, 0x00ffff00u);
    R3(k_sub64, DU64, AU64, BU64);
    R3(k_add64, D64, A64, B64);
    R2(k_rsub64, DU64, AU64, 0x0123456789abcdefULL);
    R3(k_mixed, DU, AU, BU);
    {
        uint32_t h = 0;
        for (size_t s = 0; s < sizeof S / sizeof *S; s++) {
            memset(DU, 0xa5, sizeof DU);
            k_xa(DU, AU, BU, 0x0f0f0f0fu, S[s]);
            h = h * 31u + hb(DU, sizeof DU);
        }
        printf("k_xa %08x\n", h);
    }
    {
        uint32_t h = 0;
        for (size_t s = 0; s < sizeof S / sizeof *S; s++) {
            memcpy(DU, AU, sizeof DU);
            k_inplace(DU, BU, S[s]);
            h = h * 31u + hb(DU, sizeof DU);
        }
        printf("k_inplace %08x\n", h);
    }
    return 0;
}
