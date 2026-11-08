/*
 * simd_isa_matrix.c — cross-ISA, cross-shape bit-exact differential corpus.
 *
 * WHY THIS EXISTS
 * ---------------
 * simd_shape_matrix.c covers real-world *kernels* at 29 shapes.  This file
 * covers the ISA/shape *matrix*: three SIMD register widths (128/256/512 bit,
 * expressed as portable GCC/Clang vector extensions so aarch64 and riscv64
 * codegen is exercised too) times the profitable shape families, plus one
 * intrinsic-level probe per x86 ISA tier and per non-x86 vector ISA.
 *
 * DESIGN INVARIANTS (all deliberate, all testable)
 * ------------------------------------------------
 * 1. Every shape has a `fast()` implementation and a de-optimised `ref()`
 *    oracle.  `ref()` uses a volatile induction variable so the compiler
 *    cannot apply the same vectorisation we are trying to measure.
 * 2. All arithmetic is unsigned 32-bit with defined wraparound, or exact
 *    powers of two in float (fused == unfused bit-for-bit).  There is no
 *    implementation-defined behaviour to disagree about across compilers.
 * 3. Vector loads/stores go through memcpy, never a cast: no strict-aliasing
 *    pun, no alignment assumption, no UB on 32-bit or non-x86 targets.
 * 4. Intrinsic paths are guarded by the compiler's *own* predefined macros,
 *    with an always-correct scalar fallback.  `list` prints `available=0|1`
 *    per shape, so a run can never silently claim coverage it did not get.
 * 5. ISO C99 only: no POSIX calls, no _Static_assert, no VLA, no allocation,
 *    no clock dependence in `all`/`shape` modes.  `clock()` is used only by
 *    the optional `bench` mode.
 * 6. `all` reports EVERY shape even when one is wrong (one early return
 *    hides which other configurations share the defect), and returns nonzero
 *    on any mismatch.
 *
 * Modes: list | all (default) | shape <name> | selftest | bench [iters]
 *
 * Provenance: engineering/isa/intel-sdm-093-compiler-audit.md (Intel SDM
 * edition 325462-093), the 2026-09 Arm A-profile ISA XML from the
 * harness-cache-8 toolchain cache, GCC 16.2.0 and Clang 23.1.5 intrinsic
 * headers, and engineering/isa/sse-avx-public-api.json.
 */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#if (defined(__x86_64__) || defined(__i386__))
#define HAVE_X86 1
#include <immintrin.h>
#else
#define HAVE_X86 0
#endif
#if defined(__aarch64__) && defined(__ARM_NEON)
#define HAVE_NEON 1
#include <arm_neon.h>
#else
#define HAVE_NEON 0
#endif
#if defined(__ARM_FEATURE_SVE)
#define HAVE_SVE 1
#include <arm_sve.h>
#else
#define HAVE_SVE 0
#endif
#if defined(__riscv_vector)
#define HAVE_RVV 1
#include <riscv_vector.h>
#else
#define HAVE_RVV 0
#endif

#define N 64u
#define BENCH_REPS 7
#define FNV_OFF 1469598103934665603ULL
#define FNV_PRIME 1099511628211ULL

typedef unsigned int v4u __attribute__((vector_size(16)));
typedef unsigned int v8u __attribute__((vector_size(32)));
typedef unsigned int v16u __attribute__((vector_size(64)));
typedef float v4f __attribute__((vector_size(16)));
typedef float v8f __attribute__((vector_size(32)));
typedef float v16f __attribute__((vector_size(64)));

static uint32_t g_a[N], g_b[N], g_c[N], g_out[N], g_ref[N];
static uint16_t g_u16[N], g_v16[N];
static uint8_t g_u8[N * 2];
static int8_t g_i8[N * 2];
static uint64_t g_qa[8], g_qb[8];
static float g_fa[N], g_fb[N], g_fc[N];

static uint32_t f2u(float f)
{
    uint32_t u;
    memcpy(&u, &f, sizeof u);
    return u;
}

static uint64_t fold(const uint32_t *v, size_t n)
{
    uint64_t h = FNV_OFF;
    size_t i, j;
    for (i = 0; i < n; i++) {
        uint32_t x = v[i];
        for (j = 0; j < 4u; j++) {
            h ^= (uint64_t)((x >> (8u * j)) & 0xffu);
            h *= FNV_PRIME;
        }
    }
    return h;
}

static uint32_t xorshift(uint32_t *s)
{
    uint32_t x = *s;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *s = x;
    return x;
}

static void gen_inputs(void)
{
    uint32_t seed = 0x9e3779b9u;
    size_t i;
    for (i = 0; i < N; i++) {
        g_a[i] = xorshift(&seed) & 0xffffu;
        g_b[i] = xorshift(&seed) & 0xffffu;
        g_c[i] = (xorshift(&seed) & 1u) ? 0xffffffffu : 0u;
        /* Exact powers of two: a*b and a*b+c are exact, so FMA and
         * unfused evaluation agree bit-for-bit under every -ffp-contract. */
        g_fa[i] = (float)(1u << (i % 6u));
        g_fb[i] = (float)(1u << (i % 4u)) * 0.5f;
        g_fc[i] = (float)(1u << (i % 5u));
    }
    for (i = 0; i < N; i++) {
        g_u16[i] = (uint16_t)(xorshift(&seed) & 0xffu);
        g_v16[i] = (uint16_t)(xorshift(&seed) & 0x7fu);
    }
    for (i = 0; i < N * 2u; i++) {
        g_u8[i] = (uint8_t)(xorshift(&seed) & 0xffu);
        g_i8[i] = (int8_t)(xorshift(&seed) & 0x7fu);
    }
    for (i = 0; i < 8u; i++) {
        g_qa[i] = (uint64_t)xorshift(&seed) | ((uint64_t)xorshift(&seed) << 32);
        g_qb[i] = (uint64_t)xorshift(&seed) | ((uint64_t)xorshift(&seed) << 32);
    }
}

/* ------------------------------------------------------------------ */
/* Portable vector-extension kernels: one macro, three register widths. */
/* ------------------------------------------------------------------ */

#define MAP_KERNEL(name, VT, LANES, EXPR)                                   \
static void name(uint32_t *out)                                             \
{                                                                           \
    size_t i;                                                               \
    for (i = 0; i + (LANES) <= N; i += (LANES)) {                          \
        VT a, b, c, r;                                                     \
        memcpy(&a, (const void *)&g_a[i], sizeof a);                       \
        memcpy(&b, (const void *)&g_b[i], sizeof b);                       \
        memcpy(&c, (const void *)&g_c[i], sizeof c);                       \
        r = (EXPR);                                                        \
        memcpy((void *)&out[i], &r, sizeof r);                             \
    }                                                                       \
}

#define REF_KERNEL(name, EXPR)                                              \
static void name(uint32_t *out)                                             \
{                                                                           \
    volatile size_t i;                                                     \
    for (i = 0; i < N; i++) out[i] = (uint32_t)(EXPR);                    \
}

REF_KERNEL(ref_add,   g_a[i] + g_b[i])
REF_KERNEL(ref_sub,   g_a[i] - g_b[i])
REF_KERNEL(ref_mul,   g_a[i] * g_b[i])
REF_KERNEL(ref_xor,   g_a[i] ^ g_b[i])
REF_KERNEL(ref_blend, ((g_a[i] ^ g_b[i]) & g_c[i]) ^ g_b[i])
REF_KERNEL(ref_shl3,  g_a[i] << 3)

MAP_KERNEL(map_add_w4,   v4u,  4u, a + b)
MAP_KERNEL(map_add_w8,   v8u,  8u, a + b)
MAP_KERNEL(map_add_w16,  v16u, 16u, a + b)
MAP_KERNEL(map_sub_w4,   v4u,  4u, a - b)
MAP_KERNEL(map_sub_w8,   v8u,  8u, a - b)
MAP_KERNEL(map_sub_w16,  v16u, 16u, a - b)
MAP_KERNEL(map_mul_w4,   v4u,  4u, a * b)
MAP_KERNEL(map_mul_w8,   v8u,  8u, a * b)
MAP_KERNEL(map_mul_w16,  v16u, 16u, a * b)
MAP_KERNEL(map_xor_w4,   v4u,  4u, a ^ b)
MAP_KERNEL(map_xor_w8,   v8u,  8u, a ^ b)
MAP_KERNEL(map_xor_w16,  v16u, 16u, a ^ b)
MAP_KERNEL(map_blend_w4, v4u,  4u, ((a ^ b) & c) ^ b)
MAP_KERNEL(map_blend_w8, v8u,  8u, ((a ^ b) & c) ^ b)
MAP_KERNEL(map_blend_w16,v16u, 16u, ((a ^ b) & c) ^ b)
MAP_KERNEL(map_shl3_w4,  v4u,  4u, a << 3)
MAP_KERNEL(map_shl3_w8,  v8u,  8u, a << 3)
MAP_KERNEL(map_shl3_w16, v16u, 16u, a << 3)

#define SUM_KERNEL(name, VT, LANES)                                         \
static void name(uint32_t *out)                                             \
{                                                                           \
    uint32_t tmp[LANES];                                                    \
    uint32_t acc = 0u;                                                     \
    size_t i, j;                                                           \
    for (i = 0; i + (LANES) <= N; i += (LANES)) {                         \
        VT a;                                                              \
        memcpy(&a, (const void *)&g_a[i], sizeof a);                       \
        memcpy(tmp, &a, sizeof a);                                         \
        for (j = 0; j < (LANES); j++) acc += tmp[j];                       \
    }                                                                       \
    memset(out, 0, N * sizeof out[0]);                                     \
    out[0] = acc;                                                          \
}

static void ref_sum(uint32_t *out)
{
    volatile size_t i;
    uint32_t acc = 0u;
    for (i = 0; i < N; i++) acc += g_a[i];
    memset(out, 0, N * sizeof out[0]);
    out[0] = acc;
}

SUM_KERNEL(reduce_sum_w4,  v4u,  4u)
SUM_KERNEL(reduce_sum_w8,  v8u,  8u)
SUM_KERNEL(reduce_sum_w16, v16u, 16u)

/* Indirect/gather-shaped loads: a legal but hard-to-vectorise shape. */
static void ref_gather(uint32_t *out)
{
    volatile size_t i;
    for (i = 0; i < N; i++) out[i] = g_a[g_b[i] % N];
}
#define GATHER_KERNEL(name, VT, LANES)                                      \
static void name(uint32_t *out)                                             \
{                                                                           \
    uint32_t tmp[LANES];                                                    \
    size_t i, j;                                                           \
    for (i = 0; i + (LANES) <= N; i += (LANES)) {                         \
        VT v;                                                              \
        for (j = 0; j < (LANES); j++) tmp[j] = g_a[g_b[i + j] % N];        \
        memcpy(&v, tmp, sizeof v);                                         \
        memcpy((void *)&out[i], &v, sizeof v);                             \
    }                                                                       \
}
GATHER_KERNEL(indirect_w4,  v4u,  4u)
GATHER_KERNEL(indirect_w8,  v8u,  8u)
GATHER_KERNEL(indirect_w16, v16u, 16u)

/* Prefix scan and ARX: deliberately serial / control-flow shapes. */
static void ref_scan(uint32_t *out)
{
    volatile size_t i;
    uint32_t acc = 0u;
    for (i = 0; i < N; i++) { acc += g_a[i]; out[i] = acc; }
}
static void scan_fast(uint32_t *out)
{
    size_t i;
    uint32_t acc = 0u;
    for (i = 0; i < N; i++) { acc += g_a[i]; out[i] = acc; }
}

static void ref_arx(uint32_t *out)
{
    volatile size_t i;
    uint32_t x = 0x243f6a88u;
    for (i = 0; i < N; i++) {
        uint32_t t = g_a[i];
        x ^= t; x = (x << 7) | (x >> 25); x += g_b[i];
        out[i] = x;
    }
}
static void arx_fast(uint32_t *out)
{
    uint32_t x = 0x243f6a88u;
    size_t i;
    for (i = 0; i < N; i++) {
        uint32_t t = g_a[i];
        x ^= t; x = (x << 7) | (x >> 25); x += g_b[i];
        out[i] = x;
    }
}

/* FMA orientation on exact powers of two, via portable vector types. */
static void ref_fma(uint32_t *out)
{
    volatile size_t i;
    for (i = 0; i < N; i++) {
        volatile float p = g_fa[i] * g_fb[i];
        volatile float s = p + g_fc[i];
        out[i] = f2u(s);
    }
}
#define FMA_KERNEL(name, VT, LANES)                                         \
static void name(uint32_t *out)                                             \
{                                                                           \
    size_t i, j;                                                           \
    float tmp[LANES];                                                      \
    for (i = 0; i + (LANES) <= N; i += (LANES)) {                         \
        VT a, b, c, r;                                                     \
        memcpy(&a, (const void *)&g_fa[i], sizeof a);                     \
        memcpy(&b, (const void *)&g_fb[i], sizeof b);                     \
        memcpy(&c, (const void *)&g_fc[i], sizeof c);                     \
        r = a * b + c;                                                     \
        memcpy(tmp, &r, sizeof r);                                         \
        for (j = 0; j < (LANES); j++) out[i + j] = f2u(tmp[j]);           \
    }                                                                       \
}
FMA_KERNEL(fma_orient_w4,  v4f,  4u)
FMA_KERNEL(fma_orient_w8,  v8f,  8u)
FMA_KERNEL(fma_orient_w16, v16f, 16u)

/* Portable scalar oracles for the intrinsic probes.  They must exist on
   every target, including the fallback SHAPE rows with available=0. */
static void ref_mask_add(uint32_t *out)
{
    volatile size_t i;
    for (i = 0; i < N; i++)
        out[i] = ((i % 16u) & 1u) ? g_a[i] + g_b[i] : g_a[i];
}
static void ref_mask_add_vl(uint32_t *out)
{
    volatile size_t i;
    for (i = 0; i < N; i++)
        out[i] = ((i % 4u) & 1u) ? g_a[i] + g_b[i] : g_a[i];
}
static void ref_mullo64(uint32_t *out)
{
    volatile size_t i;
    memset(out, 0, N * sizeof out[0]);
    for (i = 0; i < 8u; i++) {
        uint64_t p = g_qa[i] * g_qb[i];
        out[2u * i] = (uint32_t)p;
        out[2u * i + 1u] = (uint32_t)(p >> 32);
    }
}
static void ref_dpbusd(uint32_t *out)
{
    volatile size_t i;
    memset(out, 0, N * sizeof out[0]);
    for (i = 0; i < 16u; i++) {
        uint32_t acc = 0u;
        size_t j;
        for (j = 0; j < 4u; j++)
            acc += (uint32_t)g_u8[4u * i + j] * (uint32_t)(int32_t)g_i8[4u * i + j];
        out[i] = acc;
    }
}

/* ------------------------------------------------------------------ */
/* Intrinsic-level probes, one per ISA tier.  Each has a scalar oracle. */
/* ------------------------------------------------------------------ */

/* PMADDWD: 8 signed i16 -> 4 i32 dotted pairs. */
static uint32_t madd16_ref(size_t k)
{
    size_t i = 2u * k;
    int32_t x = (int32_t)(int16_t)g_u16[i] * (int32_t)(int16_t)g_v16[i]
              + (int32_t)(int16_t)g_u16[i + 1u] * (int32_t)(int16_t)g_v16[i + 1u];
    return (uint32_t)x;
}
static void ref_madd16(uint32_t *out)
{
    volatile size_t i;
    memset(out, 0, N * sizeof out[0]);
    for (i = 0; i < N / 2u; i++) out[i] = madd16_ref(i);
}
__attribute__((unused))
static void madd16_scalar(uint32_t *out)
{
    size_t i;
    memset(out, 0, N * sizeof out[0]);
    for (i = 0; i < N / 2u; i++) out[i] = madd16_ref(i);
}
#if HAVE_X86 && defined(__SSE2__)
static void madd16_sse2(uint32_t *out)
{
    size_t i;
    memset(out, 0, N * sizeof out[0]);
    for (i = 0; i + 8u <= N; i += 8u) {
        __m128i a = _mm_loadu_si128((const __m128i *)(const void *)&g_u16[i]);
        __m128i b = _mm_loadu_si128((const __m128i *)(const void *)&g_v16[i]);
        __m128i r = _mm_madd_epi16(a, b);
        _mm_storeu_si128((__m128i *)(void *)&out[i / 2u], r);
    }
}
#endif

/* PMADDUBSW: 16 u8 x 16 i8 -> 8 saturating i16. */
static uint32_t maddubs_ref(size_t k)
{
    size_t i = 2u * k;
    int32_t p = (int32_t)g_u8[i] * (int32_t)g_i8[i]
              + (int32_t)g_u8[i + 1u] * (int32_t)g_i8[i + 1u];
    if (p > 32767) p = 32767;
    if (p < -32768) p = -32768;
    return (uint32_t)(uint16_t)(int16_t)p;
}
static void ref_maddubs(uint32_t *out)
{
    volatile size_t i;
    memset(out, 0, N * sizeof out[0]);
    for (i = 0; i < N; i++) out[i] = maddubs_ref(i);
}
__attribute__((unused))
static void maddubs_scalar(uint32_t *out)
{
    size_t i;
    memset(out, 0, N * sizeof out[0]);
    for (i = 0; i < N; i++) out[i] = maddubs_ref(i);
}
#if HAVE_X86 && defined(__SSSE3__)
static void maddubs_ssse3(uint32_t *out)
{
    size_t i;
    int16_t tmp[8];
    memset(out, 0, N * sizeof out[0]);
    for (i = 0; i + 16u <= N * 2u; i += 16u) {
        __m128i a = _mm_loadu_si128((const __m128i *)(const void *)&g_u8[i]);
        __m128i b = _mm_loadu_si128((const __m128i *)(const void *)&g_i8[i]);
        __m128i r = _mm_maddubs_epi16(a, b);
        _mm_storeu_si128((__m128i *)(void *)tmp, r);
        {
            size_t j;
            for (j = 0; j < 8u; j++)
                out[i / 2u + j] = (uint32_t)(uint16_t)tmp[j];
        }
    }
}
#endif

#if HAVE_X86 && defined(__SSE4_1__)
static void mullo_sse41(uint32_t *out)
{
    size_t i;
    for (i = 0; i + 4u <= N; i += 4u) {
        __m128i a = _mm_loadu_si128((const __m128i *)(const void *)&g_a[i]);
        __m128i b = _mm_loadu_si128((const __m128i *)(const void *)&g_b[i]);
        _mm_storeu_si128((__m128i *)(void *)&out[i], _mm_mullo_epi32(a, b));
    }
}
#endif
#if HAVE_X86 && defined(__AVX2__)
static void madd_avx2(uint32_t *out)
{
    size_t i;
    memset(out, 0, N * sizeof out[0]);
    for (i = 0; i + 16u <= N; i += 16u) {
        __m256i a = _mm256_loadu_si256((const __m256i *)(const void *)&g_u16[i]);
        __m256i b = _mm256_loadu_si256((const __m256i *)(const void *)&g_v16[i]);
        __m256i r = _mm256_madd_epi16(a, b);
        _mm256_storeu_si256((__m256i *)(void *)&out[i / 2u], r);
    }
}
#endif
#if HAVE_X86 && defined(__FMA__)
static void fmadd_fma(uint32_t *out)
{
    size_t i;
    for (i = 0; i + 8u <= N; i += 8u) {
        __m256 a = _mm256_loadu_ps(&g_fa[i]);
        __m256 b = _mm256_loadu_ps(&g_fb[i]);
        __m256 c = _mm256_loadu_ps(&g_fc[i]);
        __m256 r = _mm256_fmadd_ps(a, b, c);
        float tmp[8];
        size_t j;
        _mm256_storeu_ps(tmp, r);
        for (j = 0; j < 8u; j++) out[i + j] = f2u(tmp[j]);
    }
}
#endif
#if HAVE_X86 && defined(__AVX512F__)
static void mask_add_avx512f(uint32_t *out)
{
    size_t i;
    for (i = 0; i + 16u <= N; i += 16u) {
        __m512i a = _mm512_loadu_si512((const void *)&g_a[i]);
        __m512i b = _mm512_loadu_si512((const void *)&g_b[i]);
        __mmask16 k = (__mmask16)0xaaaa;
        _mm512_storeu_si512((void *)&out[i],
                            _mm512_mask_add_epi32(a, k, a, b));
    }
}
#endif
#if HAVE_X86 && defined(__AVX512BW__)
static void maddubs_avx512bw(uint32_t *out)
{
    size_t i;
    int16_t tmp[32];
    memset(out, 0, N * sizeof out[0]);
    for (i = 0; i + 64u <= N * 2u; i += 64u) {
        __m512i a = _mm512_loadu_si512((const void *)&g_u8[i]);
        __m512i b = _mm512_loadu_si512((const void *)&g_i8[i]);
        __m512i r = _mm512_maddubs_epi16(a, b);
        size_t j;
        _mm512_storeu_si512((void *)tmp, r);
        for (j = 0; j < 32u; j++)
            out[i / 2u + j] = (uint32_t)(uint16_t)tmp[j];
    }
}
#endif
#if HAVE_X86 && defined(__AVX512DQ__)
static void mullo64_avx512dq(uint32_t *out)
{
    __m512i a = _mm512_loadu_si512((const void *)g_qa);
    __m512i b = _mm512_loadu_si512((const void *)g_qb);
    uint64_t tmp[8];
    size_t j;
    _mm512_storeu_si512((void *)tmp, _mm512_mullo_epi64(a, b));
    memset(out, 0, N * sizeof out[0]);
    for (j = 0; j < 8u; j++) {
        out[2u * j] = (uint32_t)tmp[j];
        out[2u * j + 1u] = (uint32_t)(tmp[j] >> 32);
    }
}
#endif
#if HAVE_X86 && defined(__AVX512VL__)
static void mask_add_avx512vl(uint32_t *out)
{
    size_t i;
    for (i = 0; i + 4u <= N; i += 4u) {
        __m128i a = _mm_loadu_si128((const __m128i *)(const void *)&g_a[i]);
        __m128i b = _mm_loadu_si128((const __m128i *)(const void *)&g_b[i]);
        __mmask8 k = (__mmask8)0xa;
        _mm_storeu_si128((__m128i *)(void *)&out[i],
                         _mm_mask_add_epi32(a, k, a, b));
    }
}
#endif
#if HAVE_X86 && defined(__AVX512VNNI__)
static void dpbusd_vnni(uint32_t *out)
{
    __m512i a = _mm512_loadu_si512((const void *)g_u8);
    __m512i b = _mm512_loadu_si512((const void *)g_i8);
    __m512i acc = _mm512_setzero_si512();
    uint32_t tmp[16];
    size_t j;
    _mm512_storeu_si512((void *)tmp, _mm512_dpbusd_epi32(acc, a, b));
    memset(out, 0, N * sizeof out[0]);
    for (j = 0; j < 16u; j++) out[j] = tmp[j];
}
#endif
#if HAVE_NEON
static void neon_add(uint32_t *out)
{
    size_t i;
    for (i = 0; i + 4u <= N; i += 4u) {
        uint32x4_t a = vld1q_u32(&g_a[i]);
        uint32x4_t b = vld1q_u32(&g_b[i]);
        vst1q_u32(&out[i], vaddq_u32(a, b));
    }
}
#endif
#if HAVE_SVE
static void sve_add(uint32_t *out)
{
    size_t i = 0u;
    while (i < N) {
        svbool_t pg = svwhilelt_b32((uint64_t)i, (uint64_t)N);
        svuint32_t a = svld1(pg, &g_a[i]);
        svuint32_t b = svld1(pg, &g_b[i]);
        svst1(pg, &out[i], svadd_u32_x(pg, a, b));
        i += svcntw();
    }
}
#endif
#if HAVE_RVV
static void rvv_add(uint32_t *out)
{
    /* Scalable vectors are length-AGNOSTIC: one vsetvl covers only VLEN/32
       lanes.  A single strip would leave out[vl..N) undefined.  Loop with a
       tail-predicated vsetvl, exactly the SVE `svwhilelt` idiom above. */
    size_t i = 0u;
    while (i < N) {
        size_t vl = __riscv_vsetvl_e32m1(N - i);
        vuint32m1_t a = __riscv_vle32_v_u32m1(&g_a[i], vl);
        vuint32m1_t b = __riscv_vle32_v_u32m1(&g_b[i], vl);
        __riscv_vse32_v_u32m1(&out[i],
                              __riscv_vadd_vv_u32m1(a, b, vl), vl);
        i += vl;
    }
}
#endif

/* ------------------------------------------------------------------ */

typedef void (*kernel_t)(uint32_t *out);
typedef struct {
    const char *name;
    const char *isa;
    const char *shape;
    const char *width;
    kernel_t fast;
    kernel_t ref;
    int available;
    const char *origin;
} isa_shape_t;

#define SHAPE(n, isa_, shape_, width_, fast_, ref_, avail_, origin_) \
    { n, isa_, shape_, width_, fast_, ref_, avail_, origin_ }

static const isa_shape_t SHAPES[] = {
    SHAPE("map_add_w4",    "any/simd",  "map",     "128", map_add_w4,    ref_add, 1, "GCC/Clang vector extensions"),
    SHAPE("map_add_w8",    "any/simd",  "map",     "256", map_add_w8,    ref_add, 1, "GCC/Clang vector extensions"),
    SHAPE("map_add_w16",   "any/simd",  "map",     "512", map_add_w16,   ref_add, 1, "GCC/Clang vector extensions"),
    SHAPE("map_sub_w4",    "any/simd",  "map",     "128", map_sub_w4,    ref_sub, 1, "GCC/Clang vector extensions"),
    SHAPE("map_sub_w8",    "any/simd",  "map",     "256", map_sub_w8,    ref_sub, 1, "GCC/Clang vector extensions"),
    SHAPE("map_sub_w16",   "any/simd",  "map",     "512", map_sub_w16,   ref_sub, 1, "GCC/Clang vector extensions"),
    SHAPE("map_mul_w4",    "any/simd",  "map",     "128", map_mul_w4,    ref_mul, 1, "GCC/Clang vector extensions"),
    SHAPE("map_mul_w8",    "any/simd",  "map",     "256", map_mul_w8,    ref_mul, 1, "GCC/Clang vector extensions"),
    SHAPE("map_mul_w16",   "any/simd",  "map",     "512", map_mul_w16,   ref_mul, 1, "GCC/Clang vector extensions"),
    SHAPE("map_xor_w4",    "any/simd",  "map",     "128", map_xor_w4,    ref_xor, 1, "GCC/Clang vector extensions"),
    SHAPE("map_xor_w8",    "any/simd",  "map",     "256", map_xor_w8,    ref_xor, 1, "GCC/Clang vector extensions"),
    SHAPE("map_xor_w16",   "any/simd",  "map",     "512", map_xor_w16,   ref_xor, 1, "GCC/Clang vector extensions"),
    SHAPE("map_blend_w4",  "any/simd",  "blend",   "128", map_blend_w4,  ref_blend, 1, "masked select"),
    SHAPE("map_blend_w8",  "any/simd",  "blend",   "256", map_blend_w8,  ref_blend, 1, "masked select"),
    SHAPE("map_blend_w16", "any/simd",  "blend",   "512", map_blend_w16, ref_blend, 1, "masked select"),
    SHAPE("map_shl3_w4",   "any/simd",  "shift",   "128", map_shl3_w4,   ref_shl3, 1, "uniform shift"),
    SHAPE("map_shl3_w8",   "any/simd",  "shift",   "256", map_shl3_w8,   ref_shl3, 1, "uniform shift"),
    SHAPE("map_shl3_w16",  "any/simd",  "shift",   "512", map_shl3_w16,  ref_shl3, 1, "uniform shift"),
    SHAPE("reduce_sum_w4", "any/simd",  "reduce",  "128", reduce_sum_w4,  ref_sum, 1, "horizontal reduction"),
    SHAPE("reduce_sum_w8", "any/simd",  "reduce",  "256", reduce_sum_w8,  ref_sum, 1, "horizontal reduction"),
    SHAPE("reduce_sum_w16","any/simd",  "reduce",  "512", reduce_sum_w16, ref_sum, 1, "horizontal reduction"),
    SHAPE("indirect_w4",   "any/simd",  "gather",  "128", indirect_w4,   ref_gather, 1, "AoS / indexed load"),
    SHAPE("indirect_w8",   "any/simd",  "gather",  "256", indirect_w8,   ref_gather, 1, "AoS / indexed load"),
    SHAPE("indirect_w16",  "any/simd",  "gather",  "512", indirect_w16,  ref_gather, 1, "AoS / indexed load"),
    SHAPE("fma_orient_w4", "any/simd",  "fma",     "128", fma_orient_w4,  ref_fma, 1, "SA-02 orientation"),
    SHAPE("fma_orient_w8", "any/simd",  "fma",     "256", fma_orient_w8,  ref_fma, 1, "SA-02 orientation"),
    SHAPE("fma_orient_w16","any/simd",  "fma",     "512", fma_orient_w16, ref_fma, 1, "SA-02 orientation"),
    SHAPE("prefix_scan",   "scalar",    "scan",    "1",   scan_fast,      ref_scan, 1, "Linux jhash / serial dependency"),
    SHAPE("arx_chain",     "scalar",    "arx",     "1",   arx_fast,       ref_arx, 1, "ChaCha20 QR / SA-06/07"),
#if HAVE_X86 && defined(__SSE2__)
    SHAPE("madd16_sse2", "sse2", "dot16", "128", madd16_sse2, ref_madd16, 1, "Intel SDM 093: PMADDWD"),
#else
    SHAPE("madd16_sse2", "sse2", "dot16", "128", madd16_scalar, ref_madd16, 0, "Intel SDM 093: PMADDWD"),
#endif
#if HAVE_X86 && defined(__SSSE3__)
    SHAPE("maddubs_ssse3", "ssse3", "dot8sat", "128", maddubs_ssse3, ref_maddubs, 1, "Intel SDM 093: PMADDUBSW"),
#else
    SHAPE("maddubs_ssse3", "ssse3", "dot8sat", "128", maddubs_scalar, ref_maddubs, 0, "Intel SDM 093: PMADDUBSW"),
#endif
#if HAVE_X86 && defined(__SSE4_1__)
    SHAPE("mullo_sse41", "sse4.1", "map", "128", mullo_sse41, ref_mul, 1, "Intel SDM 093: PMULLD"),
#else
    SHAPE("mullo_sse41", "sse4.1", "map", "128", map_mul_w4, ref_mul, 0, "Intel SDM 093: PMULLD"),
#endif
#if HAVE_X86 && defined(__AVX2__)
    SHAPE("madd_avx2", "avx2", "dot16", "256", madd_avx2, ref_madd16, 1, "Intel SDM 093: VPMADDWD"),
#else
    SHAPE("madd_avx2", "avx2", "dot16", "256", madd16_scalar, ref_madd16, 0, "Intel SDM 093: VPMADDWD"),
#endif
#if HAVE_X86 && defined(__FMA__)
    SHAPE("fmadd_fma", "fma3", "fma", "256", fmadd_fma, ref_fma, 1, "Intel SDM 093: VFMADD231PS"),
#else
    SHAPE("fmadd_fma", "fma3", "fma", "256", fma_orient_w8, ref_fma, 0, "Intel SDM 093: VFMADD231PS"),
#endif
#if HAVE_X86 && defined(__AVX512F__)
    SHAPE("mask_add_avx512f", "avx512f", "masked", "512", mask_add_avx512f, ref_mask_add, 1, "Intel SDM 093: EVEX mask"),
#else
    SHAPE("mask_add_avx512f", "avx512f", "masked", "512", ref_mask_add, ref_mask_add, 0, "Intel SDM 093: EVEX mask"),
#endif
#if HAVE_X86 && defined(__AVX512BW__)
    SHAPE("maddubs_avx512bw", "avx512bw", "dot8sat", "512", maddubs_avx512bw, ref_maddubs, 1, "GCC 16.2 avx512bwintrin.h"),
#else
    SHAPE("maddubs_avx512bw", "avx512bw", "dot8sat", "512", maddubs_scalar, ref_maddubs, 0, "GCC 16.2 avx512bwintrin.h"),
#endif
#if HAVE_X86 && defined(__AVX512DQ__)
    SHAPE("mullo64_avx512dq", "avx512dq", "map64", "512", mullo64_avx512dq, ref_mullo64, 1, "GCC 16.2 avx512dqintrin.h"),
#else
    SHAPE("mullo64_avx512dq", "avx512dq", "map64", "512", ref_mullo64, ref_mullo64, 0, "GCC 16.2 avx512dqintrin.h"),
#endif
#if HAVE_X86 && defined(__AVX512VL__)
    SHAPE("mask_add_avx512vl", "avx512vl", "masked", "128", mask_add_avx512vl, ref_mask_add_vl, 1, "GCC 16.2 avx512vlintrin.h"),
#else
    SHAPE("mask_add_avx512vl", "avx512vl", "masked", "128", ref_mask_add_vl, ref_mask_add_vl, 0, "GCC 16.2 avx512vlintrin.h"),
#endif
#if HAVE_X86 && defined(__AVX512VNNI__)
    SHAPE("dpbusd_vnni", "avx512vnni", "dot8", "512", dpbusd_vnni, ref_dpbusd, 1, "Intel SDM 093: VPDPBUSD"),
#else
    SHAPE("dpbusd_vnni", "avx512vnni", "dot8", "512", ref_dpbusd, ref_dpbusd, 0, "Intel SDM 093: VPDPBUSD"),
#endif
#if HAVE_NEON
    SHAPE("neon_add", "neon", "map", "128", neon_add, ref_add, 1, "Arm A64 ISA 2026-09: ADD V.4S"),
#else
    SHAPE("neon_add", "neon", "map", "128", map_add_w4, ref_add, 0, "Arm A64 ISA 2026-09: ADD V.4S"),
#endif
#if HAVE_SVE
    SHAPE("sve_add", "sve", "map", "scalable", sve_add, ref_add, 1, "Arm A64 ISA 2026-09: ADD Z.S"),
#else
    SHAPE("sve_add", "sve", "map", "scalable", map_add_w4, ref_add, 0, "Arm A64 ISA 2026-09: ADD Z.S"),
#endif
#if HAVE_RVV
    SHAPE("rvv_add", "rvv", "map", "scalable", rvv_add, ref_add, 1, "RISC-V V 1.0: vadd.vv"),
#else
    SHAPE("rvv_add", "rvv", "map", "scalable", map_add_w4, ref_add, 0, "RISC-V V 1.0: vadd.vv"),
#endif
};

#define SHAPE_COUNT (sizeof(SHAPES) / sizeof(SHAPES[0]))

static const isa_shape_t *find_shape(const char *name)
{
    size_t i;
    for (i = 0; i < SHAPE_COUNT; i++)
        if (strcmp(SHAPES[i].name, name) == 0) return &SHAPES[i];
    return NULL;
}

static int run_shape(const isa_shape_t *s, uint64_t *f, uint64_t *r)
{
    memset(g_out, 0, sizeof g_out);
    memset(g_ref, 0, sizeof g_ref);
    s->fast(g_out);
    s->ref(g_ref);
    *f = fold(g_out, N);
    *r = fold(g_ref, N);
    return *f == *r;
}

static int selftest(void)
{
    static const uint32_t probe[4] = { 1u, 2u, 3u, 4u };
    uint64_t expect = FNV_OFF;
    size_t i, j;
    int bad = 0;
    for (i = 0; i < 4u; i++) {
        for (j = 0; j < 4u; j++) {
            expect ^= (uint64_t)((probe[i] >> (8u * j)) & 0xffu);
            expect *= FNV_PRIME;
        }
    }
    if (fold(probe, 4u) != expect) {
        fprintf(stderr, "selftest: fold chain broken\n");
        return 1;
    }
    gen_inputs();
    {
        size_t available = 0u;
        for (i = 0; i < SHAPE_COUNT; i++) {
            uint64_t f, r;
            available += (size_t)SHAPES[i].available;
            if (!run_shape(&SHAPES[i], &f, &r)) {
                fprintf(stderr, "selftest: %s fast=%016llx ref=%016llx\n",
                        SHAPES[i].name, (unsigned long long)f,
                        (unsigned long long)r);
                bad = 1;
            }
        }
        printf("selftest %s shapes=%lu available=%lu\n",
               bad ? "FAIL" : "PASS", (unsigned long)SHAPE_COUNT,
               (unsigned long)available);
    }
    return bad;
}

static int run_all(void)
{
    size_t i;
    int bad = 0;
    gen_inputs();
    for (i = 0; i < SHAPE_COUNT; i++) {
        uint64_t f, r;
        int ok = run_shape(&SHAPES[i], &f, &r);
        printf("%-22s %016llx %016llx %s\n", SHAPES[i].name,
               (unsigned long long)f, (unsigned long long)r,
               ok ? "ok" : "MISMATCH");
        bad |= !ok;
    }
    return bad;
}

static double cpu_seconds(void)
{
    return (double)clock() / (double)CLOCKS_PER_SEC;
}

static int bench(size_t iters)
{
    size_t rep, i;
    double samples[BENCH_REPS];
    double median;
    gen_inputs();
    for (rep = 0; rep < BENCH_REPS; rep++) {
        double t0 = cpu_seconds();
        for (i = 0; i < iters * SHAPE_COUNT; i++) {
            size_t k = i % SHAPE_COUNT;
            g_out[0] = 0u;
            SHAPES[k].fast(g_out);
        }
        samples[rep] = (cpu_seconds() - t0) /
                       (double)(iters * SHAPE_COUNT);
    }
    {
        size_t a, b;
        for (a = 0; a < BENCH_REPS; a++)
            for (b = a + 1u; b < BENCH_REPS; b++)
                if (samples[b] < samples[a]) {
                    double t = samples[a]; samples[a] = samples[b];
                    samples[b] = t;
                }
    }
    median = samples[BENCH_REPS / 2u];
    fprintf(stderr, "# median seconds per shape (7 batches x %lu iterations)\n",
            (unsigned long)iters);
    fprintf(stderr, "all_shapes %.9f\n", median);
    return 0;
}

int main(int argc, char **argv)
{
    const char *mode = (argc > 1) ? argv[1] : "all";
    if (strcmp(mode, "list") == 0) {
        size_t i, avail = 0u;
        for (i = 0; i < SHAPE_COUNT; i++) {
            printf("%s\t%s\t%s\t%s\t%d\t%s\n", SHAPES[i].name,
                   SHAPES[i].isa, SHAPES[i].shape, SHAPES[i].width,
                   SHAPES[i].available, SHAPES[i].origin);
            avail += (size_t)SHAPES[i].available;
        }
        fprintf(stderr, "# shapes=%lu available=%lu\n",
                (unsigned long)SHAPE_COUNT, (unsigned long)avail);
        return 0;
    }
    if (strcmp(mode, "bench") == 0) {
        size_t iters = 1024u;
        if (argc > 2) {
            long v = strtol(argv[2], NULL, 10);
            if (v > 0 && v <= 100000L) iters = (size_t)v;
        }
        return bench(iters);
    }
    if (strcmp(mode, "selftest") == 0) return selftest();
    if (strcmp(mode, "shape") == 0) {
        const isa_shape_t *s;
        uint64_t f, r;
        if (argc < 3) return 2;
        s = find_shape(argv[2]);
        if (!s) return 2;
        gen_inputs();
        run_shape(s, &f, &r);
        printf("%s %016llx %016llx\n", s->name,
               (unsigned long long)f, (unsigned long long)r);
        return f == r ? 0 : 1;
    }
    if (strcmp(mode, "all") != 0) return 2;
    return run_all();
}
