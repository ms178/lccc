/*
 * SIMD shape matrix — bit-exact differential corpus for the lccc SIMD stack.
 *
 * Provenance.  Every shape is distilled from a real workload: the in-tree
 * benchmark corpus (tests/bench/k_*.c, tests/benchmark/kernel_corpus/k*.c,
 * the C sources in tests/simd/) and the projects packaged in
 * ms178/archpkgbuilds
 * (zlib-ng, gzip, zstd, SQLite, expat, curl/OpenSSL, glibc, Linux jhash,
 * LightPixel, simdjson, Chromium/Skia, FFmpeg, libjpeg-turbo).  Three
 * complexity tiers are deliberate: trivially vectorizable maps, shapes that
 * defeat a naive recognize-and-pack pass, and shapes that stress the pass
 * interactions (nested reductions, AoS/SoA, masked tails, bypass CFG edges,
 * ISA gates, vzeroupper transitions, 512-bit EVEX folds).
 *
 * Contract.  Each shape has two independent implementations:
 *   - fast():  the shape under test, written so an auto-vectorizer, SLP
 *              packer or intrinsic lowering can transform it;
 *   - ref():   a de-optimized scalar oracle (volatile induction variable)
 *              that no compiler can vectorize or constant-fold away.
 * Both write the same number of uint32_t results into disjoint buffers.
 * main() folds each buffer with a strictly sequential FNV-1a chain and
 * prints both digests.  A candidate build is correct iff, for every shape,
 *   (a) fast == ref within the build (semantic correctness), and
 *   (b) both digests equal those of a -O0 reference build (optimization
 *       invariance).  The driver enforces both, so a compiler that breaks
 *       the shape and a compiler that breaks the oracle are both caught.
 *
 * Determinism.  Inputs come from an in-process xorshift32 with a fixed seed;
 * no I/O, no allocation, no clock dependence in `all`/`shape` modes.  Float
 * shapes use exact powers of two, so fused and unfused evaluation are
 * bit-identical and any mismatch is a codegen defect, never rounding.
 * Timing is confined to `bench` mode and written to stderr, never to the
 * compared stdout stream.
 *
 * Portability / UB.  C99 only (no _Static_assert / _Alignas — lccc's C11
 * coverage is not assumed).  Fixed-width types, unsigned wraparound only,
 * no strict-aliasing puns (float bits move through memcpy), no unaligned
 * scalar accesses, no signed overflow, no out-of-bounds indexing, no
 * allocation, no global mutable state shared between concurrent runs.
 * x86 intrinsic shapes are behind architecture + ISA predefines and have
 * scalar fallbacks that produce identical digests on AArch64/RISC-V/i686.
 *
 * Modes:  list | all (default) | shape <name> | bench [iters] | selftest
 */
#include <stdio.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#define N ((size_t)256)
#define BENCH_ITERS ((size_t)64)

typedef char assert_u32_is_4[(sizeof(uint32_t) == 4) ? 1 : -1];
typedef char assert_u64_is_8[(sizeof(uint64_t) == 8) ? 1 : -1];
typedef char assert_float_is_4[(sizeof(float) == 4) ? 1 : -1];
typedef char assert_ptr_is_32_or_64[(sizeof(void *) == 4 || sizeof(void *) == 8) ? 1 : -1];

#if (defined(__x86_64__) || defined(__i386__)) && defined(__AVX512F__) && defined(__AVX512BW__)
#define HAVE_AVX512BW 1
#include <immintrin.h>
#else
#define HAVE_AVX512BW 0
#endif

static uint32_t g_in[N];
static uint32_t g_out[N];
static uint32_t g_ref[N];
static float    g_fa[N];
static float    g_fb[N];

static uint32_t xs32(uint32_t *state)
{
    uint32_t x = *state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *state = x;
    return x;
}

static void gen_inputs(void)
{
    uint32_t state = 0x9e3779b9u;
    size_t i;
    for (i = 0; i < N; i++) {
        /* Small magnitudes keep every unsigned product below 2^32 and make
         * the float shapes exact powers of two. */
        g_in[i] = xs32(&state) & 0xffu;
        g_fa[i] = (float)(1u + (g_in[i] & 7u));
        g_fb[i] = (float)(1u + ((g_in[i] >> 3) & 3u)) * 0.5f;
    }
}

static uint64_t fold(const uint32_t *p, size_t n)
{
    uint64_t h = 1469598103934665603ULL;
    size_t i;
    for (i = 0; i < n; i++) {
        h ^= (uint64_t)p[i];
        h *= 1099511628211ULL;
    }
    return h;
}

static uint32_t f2u(float f)
{
    uint32_t u;
    memcpy(&u, &f, sizeof u);
    return u;
}

static uint32_t sat_u8(int32_t v)
{
    if (v < 0) return 0u;
    if (v > 255) return 255u;
    return (uint32_t)v;
}

/* ------------------------------------------------------------------ */
/* Tier 1 — trivially vectorizable maps and reductions                   */
/* ------------------------------------------------------------------ */

/* BLAS saxpy / k20_axpy_f32.c */
static void s_axpy_f32(void)
{
    size_t i;
    for (i = 0; i < N; i++) g_out[i] = f2u(g_fa[i] * 4.0f + g_fb[i]);
}
static void r_axpy_f32(void)
{
    volatile size_t i;
    for (i = 0; i < N; i++) g_ref[i] = f2u(g_fa[i] * 4.0f + g_fb[i]);
}

/* k_map64_sub.c: 64-bit elementwise map, split into two exact halves */
static void s_map_u32(void)
{
    size_t i;
    for (i = 0; i < N; i++) g_out[i] = g_in[i] * 3u + 7u;
}
static void r_map_u32(void)
{
    volatile size_t i;
    for (i = 0; i < N; i++) g_ref[i] = g_in[i] * 3u + 7u;
}

/* k02_sum8.c: single-accumulator reduction */
static void s_sum_u32(void)
{
    uint32_t acc = 0u;
    size_t i;
    for (i = 0; i < N; i++) acc += g_in[i];
    g_out[0] = acc;
}
static void r_sum_u32(void)
{
    volatile uint32_t acc = 0u;
    volatile size_t i;
    for (i = 0; i < N; i++) acc = acc + g_in[i];
    g_ref[0] = acc;
}

/* k06_dot.c: dot product, exact float inputs */
static void s_dot_f32(void)
{
    float acc = 0.0f;
    size_t i;
    for (i = 0; i < N; i++) acc += g_fa[i] * g_fb[i];
    g_out[0] = f2u(acc);
}
static void r_dot_f32(void)
{
    volatile float acc = 0.0f;
    volatile size_t i;
    for (i = 0; i < N; i++) acc = acc + g_fa[i] * g_fb[i];
    g_ref[0] = f2u(acc);
}

/* k23_clamp_u8.c / LightPixel: unsigned saturating clamp */
static void s_clamp_u8(void)
{
    size_t i;
    for (i = 0; i < N; i++)
        g_out[i] = sat_u8((int32_t)(g_in[i] * 2u) - 64);
}
static void r_clamp_u8(void)
{
    volatile size_t i;
    for (i = 0; i < N; i++)
        g_ref[i] = sat_u8((int32_t)(g_in[i] * 2u) - 64);
}

/* k21_fir4.c: 4-tap FIR with a sliding window (guarded start) */
static void s_fir4(void)
{
    static const uint32_t taps[4] = { 1u, 2u, 1u, 1u };
    size_t i;
    for (i = 0; i < N; i++) {
        uint32_t acc = 0u;
        size_t k;
        for (k = 0; k < 4u; k++)
            acc += (i >= k) ? g_in[i - k] * taps[k] : 0u;
        g_out[i] = acc;
    }
}
static void r_fir4(void)
{
    static const uint32_t taps[4] = { 1u, 2u, 1u, 1u };
    volatile size_t i;
    for (i = 0; i < N; i++) {
        volatile uint32_t acc = 0u;
        volatile size_t k;
        for (k = 0; k < 4u; k++)
            acc = acc + ((i >= k) ? g_in[i - k] * taps[k] : 0u);
        g_ref[i] = acc;
    }
}

/* k19_minpos_f32.c: min-value + first index (two coupled reductions) */
static void s_minpos(void)
{
    uint32_t best = g_in[0], pos = 0u, i;
    for (i = 1u; i < (uint32_t)N; i++) {
        if (g_in[i] < best) { best = g_in[i]; pos = i; }
    }
    g_out[0] = best;
    g_out[1] = pos;
}
static void r_minpos(void)
{
    volatile uint32_t best = g_in[0], pos = 0u, i;
    for (i = 1u; i < (uint32_t)N; i++) {
        if (g_in[i] < best) { best = g_in[i]; pos = i; }
    }
    g_ref[0] = best;
    g_ref[1] = pos;
}

/* k15_bytemask.c / expat: multi-range byte classification */
static void s_bytemask(void)
{
    size_t i;
    for (i = 0; i < N; i++) {
        uint32_t c = g_in[i] & 0x7fu;
        uint32_t hit = (c >= 'A' && c <= 'Z') ? 1u : 0u;
        hit |= (c >= 'a' && c <= 'z') ? 2u : 0u;
        hit |= (c >= '0' && c <= '9') ? 4u : 0u;
        g_out[i] = hit;
    }
}
static void r_bytemask(void)
{
    volatile size_t i;
    for (i = 0; i < N; i++) {
        volatile uint32_t c = g_in[i] & 0x7fu;
        volatile uint32_t hit = (c >= 'A' && c <= 'Z') ? 1u : 0u;
        hit = hit | ((c >= 'a' && c <= 'z') ? 2u : 0u);
        hit = hit | ((c >= '0' && c <= '9') ? 4u : 0u);
        g_ref[i] = hit;
    }
}

/* ------------------------------------------------------------------ */
/* Tier 2 — shapes that defeat a naive recognize-and-pack pass          */
/* ------------------------------------------------------------------ */

/* zlib-ng adler32 / k_adler32.c + k_adler32_do8.c: two dependent accumulators */
static void s_adler32(void)
{
    uint32_t a = 1u, b = 0u, i;
    for (i = 0u; i < (uint32_t)N; i++) {
        a = (a + g_in[i]) % 65521u;
        b = (b + a) % 65521u;
    }
    g_out[0] = (b << 16) | a;
}
static void r_adler32(void)
{
    volatile uint32_t a = 1u, b = 0u, i;
    for (i = 0u; i < (uint32_t)N; i++) {
        a = (a + g_in[i]) % 65521u;
        b = (b + a) % 65521u;
    }
    g_ref[0] = (b << 16) | a;
}

/* zlib-ng / gzip crc32 braid (slicing-by-8 shape without a 1 KiB table) */
static void s_crc32_braid8(void)
{
    static const uint32_t poly[8] = {
        0x1u, 0x2u, 0x4u, 0x8u, 0x10u, 0x20u, 0x40u, 0x80u
    };
    uint32_t lane[8] = { 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u };
    size_t i;
    for (i = 0; i < N; i++) {
        size_t k = i & 7u;
        uint32_t v = lane[k] ^ g_in[i];
        lane[k] = (v << 1) ^ ((v >> 31) * poly[k]);
    }
    for (i = 0; i < 8u; i++) g_out[i] = lane[i];
}
static void r_crc32_braid8(void)
{
    static const uint32_t poly[8] = {
        0x1u, 0x2u, 0x4u, 0x8u, 0x10u, 0x20u, 0x40u, 0x80u
    };
    volatile uint32_t lane[8] = { 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u };
    volatile size_t i;
    for (i = 0; i < N; i++) {
        volatile size_t k = i & 7u;
        volatile uint32_t v = lane[k] ^ g_in[i];
        lane[k] = (v << 1) ^ ((v >> 31) * poly[k]);
    }
    for (i = 0; i < 8u; i++) g_ref[i] = lane[i];
}

/* expat / simdjson UTF-8 lead-byte classification with a branchy tail */
static void s_utf8_classify(void)
{
    size_t i;
    for (i = 0; i < N; i++) {
        uint32_t c = g_in[i];
        uint32_t need;
        if (c < 0x80u) need = 0u;
        else if (c < 0xc2u) need = 0xffu;      /* overlong / continuation */
        else if (c < 0xe0u) need = 1u;
        else if (c < 0xf0u) need = 2u;
        else if (c < 0xf5u) need = 3u;
        else need = 0xfeu;                    /* > U+10FFFF */
        g_out[i] = need;
    }
}
static void r_utf8_classify(void)
{
    volatile size_t i;
    for (i = 0; i < N; i++) {
        volatile uint32_t c = g_in[i];
        volatile uint32_t need;
        if (c < 0x80u) need = 0u;
        else if (c < 0xc2u) need = 0xffu;
        else if (c < 0xe0u) need = 1u;
        else if (c < 0xf0u) need = 2u;
        else if (c < 0xf5u) need = 3u;
        else need = 0xfeu;
        g_ref[i] = need;
    }
}

/* SQLite sqlite3GetVarint32 / k_varint.c: serial-dependent varint walk */
static void s_varint_scan(void)
{
    size_t i;
    uint32_t acc = 0u;
    for (i = 0; i < N; i++) {
        uint32_t v = g_in[i];
        uint32_t n = (v & 0x80u) ? 2u : 1u;
        acc = acc * 33u + ((v & 0x7fu) << (n * 7u));
    }
    g_out[0] = acc;
}
static void r_varint_scan(void)
{
    volatile size_t i;
    volatile uint32_t acc = 0u;
    for (i = 0; i < N; i++) {
        volatile uint32_t v = g_in[i];
        volatile uint32_t n = (v & 0x80u) ? 2u : 1u;
        acc = acc * 33u + ((v & 0x7fu) << (n * 7u));
    }
    g_ref[0] = acc;
}

/* glibc memchr / k_memchr.c: early-exit byte scan */
static void s_memchr_scan(void)
{
    size_t i;
    uint32_t found = 0xffffffffu;
    for (i = 0; i < N; i++) {
        if ((g_in[i] & 0x1fu) == 0x1fu) { found = (uint32_t)i; break; }
    }
    g_out[0] = found;
}
static void r_memchr_scan(void)
{
    volatile size_t i;
    volatile uint32_t found = 0xffffffffu;
    for (i = 0; i < N; i++) {
        if ((g_in[i] & 0x1fu) == 0x1fu) { found = (uint32_t)i; break; }
    }
    g_ref[0] = found;
}

/* glibc memmem / k_memmem_horspool.c: Boyer-Moore-Horspool over fixed data */
static void s_horspool(void)
{
    uint32_t shift[256];
    uint32_t hits = 0u, i;
    const uint32_t needle[4] = { 0x11u, 0x22u, 0x33u, 0x44u };
    for (i = 0; i < 256u; i++) shift[i] = 4u;
    for (i = 0; i < 3u; i++) shift[needle[i]] = 3u - (uint32_t)i;
    for (i = 0; i + 4u <= N; ) {
        uint32_t k;
        for (k = 0; k < 4u; k++) {
            if (g_in[i + k] != needle[k]) break;
        }
        if (k == 4u) { hits++; i += 1u; }
        else i += shift[g_in[i + 3u]];
    }
    g_out[0] = hits;
}
static void r_horspool(void)
{
    volatile uint32_t shift[256];
    volatile uint32_t hits = 0u, i;
    volatile uint32_t needle[4] = { 0x11u, 0x22u, 0x33u, 0x44u };
    for (i = 0; i < 256u; i++) shift[i] = 4u;
    for (i = 0; i < 3u; i++) shift[needle[i]] = 3u - i;
    for (i = 0; i + 4u <= N; ) {
        volatile uint32_t k;
        for (k = 0; k < 4u; k++) {
            if (g_in[i + k] != needle[k]) break;
        }
        if (k == 4u) { hits = hits + 1u; i = i + 1u; }
        else i = i + shift[g_in[i + 3u]];
    }
    g_ref[0] = hits;
}

/* Linux include/linux/jhash.h / k_jhash.c: rotate-xor-add dependency chain */
static uint32_t rol32(uint32_t v, uint32_t n)
{
    return (v << n) | (v >> (32u - n));
}
static uint32_t jhash_one(uint32_t a, uint32_t b, uint32_t c, uint32_t seed)
{
    a += seed + 0x9e3779b9u;
    b += seed + 0x9e3779b9u;
    c += seed;
    a -= c; a ^= rol32(c, 4u);  c += b;
    b -= a; b ^= rol32(a, 6u);  a += c;
    c -= b; c ^= rol32(b, 8u);  b += a;
    a -= c; a ^= rol32(c, 16u); c += b;
    return c;
}
static void s_jhash(void)
{
    uint32_t i;
    for (i = 0u; i < (uint32_t)N; i++)
        g_out[i] = jhash_one(g_in[i], g_in[(i + 1u) % (uint32_t)N], i, 0xdeadbeefu);
}
static void r_jhash(void)
{
    volatile uint32_t i;
    for (i = 0u; i < (uint32_t)N; i++)
        g_ref[i] = jhash_one(g_in[i], g_in[(i + 1u) % (uint32_t)N], i, 0xdeadbeefu);
}

/* FNV-1a / k_hashmix.c: strictly serial multiply-xor chain */
static void s_fnv1a(void)
{
    uint32_t h = 2166136261u, i;
    for (i = 0u; i < (uint32_t)N; i++) h = (h ^ g_in[i]) * 16777619u;
    g_out[0] = h;
}
static void r_fnv1a(void)
{
    volatile uint32_t h = 2166136261u, i;
    for (i = 0u; i < (uint32_t)N; i++) h = (h ^ g_in[i]) * 16777619u;
    g_ref[0] = h;
}

/* curl/OpenSSL base64 / k_base64_decode.c: 4-char quanta -> 3 bytes */
static void s_base64_decode(void)
{
    static const char alphabet[] =
        "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    uint8_t table[256];
    uint32_t i, out = 0u;
    for (i = 0; i < 256u; i++) table[i] = 0xffu;
    for (i = 0; i < 64u; i++) table[(uint32_t)(unsigned char)alphabet[i]] = (uint8_t)i;
    for (i = 0; i + 4u <= (uint32_t)N; i += 4u) {
        uint32_t q = 0u, k;
        for (k = 0; k < 4u; k++) {
            uint32_t ch = (uint32_t)(unsigned char)alphabet[g_in[i + k] & 63u];
            q = (q << 6) | table[ch];
        }
        g_out[out++] = q & 0xffffffffu;
    }
    while (out < N) g_out[out++] = 0u;
}
static void r_base64_decode(void)
{
    static const char alphabet[] =
        "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    volatile uint8_t table[256];
    volatile uint32_t i, out = 0u;
    for (i = 0; i < 256u; i++) table[i] = 0xffu;
    for (i = 0; i < 64u; i++) table[(uint32_t)(unsigned char)alphabet[i]] = (uint8_t)i;
    for (i = 0; i + 4u <= (uint32_t)N; i += 4u) {
        volatile uint32_t q = 0u, k;
        for (k = 0; k < 4u; k++) {
            volatile uint32_t ch = (uint32_t)(unsigned char)alphabet[g_in[i + k] & 63u];
            q = (q << 6) | table[ch];
        }
        g_ref[out++] = q & 0xffffffffu;
    }
    while (out < N) g_ref[out++] = 0u;
}

/* gzip longest_match / k_matchlen.c: early-exit compare loop */
static void s_matchlen(void)
{
    uint32_t total = 0u, i;
    for (i = 0u; i + 8u <= (uint32_t)N; i += 8u) {
        uint32_t k = 0u;
        while (k < 8u && g_in[i + k] == g_in[(i + 8u + k) % (uint32_t)N]) k++;
        total += k;
    }
    g_out[0] = total;
}
static void r_matchlen(void)
{
    volatile uint32_t total = 0u, i;
    for (i = 0u; i + 8u <= (uint32_t)N; i += 8u) {
        volatile uint32_t k = 0u;
        while (k < 8u && g_in[i + k] == g_in[(i + 8u + k) % (uint32_t)N]) k = k + 1u;
        total = total + k;
    }
    g_ref[0] = total;
}

/* ------------------------------------------------------------------ */
/* Tier 3 — pass-interaction / godlike-complex shapes                   */
/* ------------------------------------------------------------------ */

/* AoS complex multiply (aos_index_roots.c / FFmpeg, Skia) */
static void s_aos_cmul(void)
{
    size_t i;
    for (i = 0; i + 1u < N; i += 2u) {
        float ar = g_fa[i], ai = g_fa[i + 1u];
        float br = g_fb[i], bi = g_fb[i + 1u];
        g_out[i]      = f2u(ar * br - ai * bi);
        g_out[i + 1u] = f2u(ar * bi + ai * br);
    }
}
static void r_aos_cmul(void)
{
    volatile size_t i;
    for (i = 0; i + 1u < N; i += 2u) {
        volatile float ar = g_fa[i], ai = g_fa[i + 1u];
        volatile float br = g_fb[i], bi = g_fb[i + 1u];
        g_ref[i]      = f2u(ar * br - ai * bi);
        g_ref[i + 1u] = f2u(ar * bi + ai * br);
    }
}

/* 3-channel AoS cross-add (LightPixel / tests/simd/joint_pixel_shapes.c) */
static void s_aos_crossadd(void)
{
    size_t i;
    for (i = 0; i + 2u < N; i += 3u) {
        uint32_t a = g_in[i], b = g_in[i + 1u], c = g_in[i + 2u];
        g_out[i]      = a + b;
        g_out[i + 1u] = b + c;
        g_out[i + 2u] = c + a;
    }
    g_out[N - 1u] = g_in[N - 1u];
}
static void r_aos_crossadd(void)
{
    volatile size_t i;
    for (i = 0; i + 2u < N; i += 3u) {
        volatile uint32_t a = g_in[i], b = g_in[i + 1u], c = g_in[i + 2u];
        g_ref[i]      = a + b;
        g_ref[i + 1u] = b + c;
        g_ref[i + 2u] = c + a;
    }
    g_ref[N - 1u] = g_in[N - 1u];
}

/* SA-01: two independent dot-product accumulators (multi-reduction arm) */
static void s_dot2_multi_acc(void)
{
    float s1 = 0.0f, s2 = 0.0f;
    size_t i;
    for (i = 0; i < N; i++) {
        s1 += g_fa[i] * g_fb[i];
        s2 += g_fa[N - 1u - i] * g_fb[i];
    }
    g_out[0] = f2u(s1);
    g_out[1] = f2u(s2);
}
static void r_dot2_multi_acc(void)
{
    volatile float s1 = 0.0f, s2 = 0.0f;
    volatile size_t i;
    for (i = 0; i < N; i++) {
        s1 = s1 + g_fa[i] * g_fb[i];
        s2 = s2 + g_fa[N - 1u - i] * g_fb[i];
    }
    g_ref[0] = f2u(s1);
    g_ref[1] = f2u(s2);
}

/* SA-02: FMA orientation matrix — mul-on-lhs, mul-on-rhs, mirrored control.
 * Exact powers of two make fused and unfused evaluation bit-identical. */
static void s_fma_orientation(void)
{
    size_t i;
    for (i = 0; i < N; i += 4u) {
        float x = g_fa[i], k = g_fb[i], z = g_fa[i + 1u];
        g_out[i]      = f2u(x * k - z);        /* mul on lhs: needs vfmsub */
        g_out[i + 1u] = f2u(k * x - z);        /* mirrored multiplicand   */
        g_out[i + 2u] = f2u(z - x * k);        /* control orientation     */
        g_out[i + 3u] = f2u(z - k * x);        /* control, swapped        */
    }
}
static void r_fma_orientation(void)
{
    volatile size_t i;
    for (i = 0; i < N; i += 4u) {
        volatile float x = g_fa[i], k = g_fb[i], z = g_fa[i + 1u];
        g_ref[i]      = f2u(x * k - z);
        g_ref[i + 1u] = f2u(k * x - z);
        g_ref[i + 2u] = f2u(z - x * k);
        g_ref[i + 3u] = f2u(z - k * x);
    }
}

/* SA-25/26 + aos_pair_shapes: stride-4 gather / de-interleave */
static void s_gather_stride4(void)
{
    size_t i;
    for (i = 0; i < N / 4u; i++) {
        uint32_t a = g_in[4u * i], b = g_in[4u * i + 1u];
        uint32_t c = g_in[4u * i + 2u], d = g_in[4u * i + 3u];
        g_out[4u * i]      = a + c;
        g_out[4u * i + 1u] = b + d;
        g_out[4u * i + 2u] = a ^ d;
        g_out[4u * i + 3u] = b ^ c;
    }
}
static void r_gather_stride4(void)
{
    volatile size_t i;
    for (i = 0; i < N / 4u; i++) {
        volatile uint32_t a = g_in[4u * i], b = g_in[4u * i + 1u];
        volatile uint32_t c = g_in[4u * i + 2u], d = g_in[4u * i + 3u];
        g_ref[4u * i]      = a + c;
        g_ref[4u * i + 1u] = b + d;
        g_ref[4u * i + 2u] = a ^ d;
        g_ref[4u * i + 3u] = b ^ c;
    }
}

/* SA-24: internal branch + masked tail (predication / masked-store class) */
static void s_masked_tail(void)
{
    size_t i;
    for (i = 0; i < N; i++) {
        uint32_t v = g_in[i];
        if ((v & 1u) != 0u) v = v * 3u + 1u;
        else v = v >> 1;
        g_out[i] = (i < N - 5u) ? v : 0u;      /* masked tail */
    }
}
static void r_masked_tail(void)
{
    volatile size_t i;
    for (i = 0; i < N; i++) {
        volatile uint32_t v = g_in[i];
        if ((v & 1u) != 0u) v = v * 3u + 1u;
        else v = v >> 1;
        g_ref[i] = (i < N - 5u) ? v : 0u;
    }
}

/* SA-21: inclusive-exit IV (<=), the recognizer's rejected comparison */
static void s_inclusive_exit(void)
{
    uint32_t acc = 0u;
    uint32_t i;
    for (i = 0u; i <= (uint32_t)N - 1u; i++) acc += g_in[i] ^ i;
    g_out[0] = acc;
}
static void r_inclusive_exit(void)
{
    volatile uint32_t acc = 0u;
    volatile uint32_t i;
    for (i = 0u; i <= (uint32_t)N - 1u; i++) acc = acc + (g_in[i] ^ i);
    g_ref[0] = acc;
}

/* SA-06/07: ChaCha20-style ARX double round with a preheader-dominated
 * bypass edge, plus a result word leaving through the return terminator. */
static void s_chacha_rounds(void)
{
    uint32_t x[16];
    uint32_t r, acc = 0u;
    size_t i;
    for (i = 0; i < 16u; i++) x[i] = g_in[i] | 0x01010101u;
    for (r = 0u; r < 4u; r++) {
        size_t k;
        for (k = 0; k < 4u; k++) {
            uint32_t a = x[k], b = x[4u + k], c = x[8u + k], d = x[12u + k];
            a += b; d ^= a; d = (d << 16) | (d >> 16);
            c += d; b ^= c; b = (b << 12) | (b >> 20);
            a += b; d ^= a; d = (d << 8)  | (d >> 24);
            c += d; b ^= c; b = (b << 7)  | (b >> 25);
            x[k] = a; x[4u + k] = b; x[8u + k] = c; x[12u + k] = d;
        }
    }
    for (i = 0; i < 16u; i++) acc = acc * 31u + x[i];
    g_out[0] = acc;
    g_out[1] = x[9];                           /* terminator use of a word */
}
static void r_chacha_rounds(void)
{
    volatile uint32_t x[16];
    volatile uint32_t r, acc = 0u;
    volatile size_t i;
    for (i = 0; i < 16u; i++) x[i] = g_in[i] | 0x01010101u;
    for (r = 0u; r < 4u; r++) {
        volatile size_t k;
        for (k = 0; k < 4u; k++) {
            volatile uint32_t a = x[k], b = x[4u + k], c = x[8u + k], d = x[12u + k];
            a += b; d ^= a; d = (d << 16) | (d >> 16);
            c += d; b ^= c; b = (b << 12) | (b >> 20);
            a += b; d ^= a; d = (d << 8)  | (d >> 24);
            c += d; b ^= c; b = (b << 7)  | (b >> 25);
            x[k] = a; x[4u + k] = b; x[8u + k] = c; x[12u + k] = d;
        }
    }
    for (i = 0; i < 16u; i++) acc = acc * 31u + x[i];
    g_ref[0] = acc;
    g_ref[1] = x[9];
}

/* SA-33: EVEX -> VEX -> call transition (vzeroupper arming class).  The
 * asm-shape oracle lives in check_audit_2026-11-08.py; this kernel pins the
 * data-flow so the transition cannot be optimized away. */
static void s_vzeroupper_transition(void)
{
    uint32_t acc = 0u;
    size_t i;
    for (i = 0; i < N; i++) acc += g_in[i] * 3u;
#if HAVE_AVX512BW
    {
        __m512i v = _mm512_loadu_si512(g_in);
        __m512i w = _mm512_set1_epi32(3);
        __m512i p = _mm512_mullo_epi32(v, w);
        uint32_t lanes[16];
        _mm512_storeu_si512(lanes, p);
        for (i = 0; i < 16u; i++) acc += lanes[i];
    }
#else
    for (i = 0; i < 16u; i++) acc += g_in[i] * 3u;
#endif
    g_out[0] = acc;
}
static void r_vzeroupper_transition(void)
{
    volatile uint32_t acc = 0u;
    volatile size_t i;
    for (i = 0; i < N; i++) acc = acc + g_in[i] * 3u;
    for (i = 0; i < 16u; i++) acc = acc + g_in[i] * 3u;
    g_ref[0] = acc;
}

/* SA-05: 512-bit maddubs memory-fold operand roles, scalar-equivalent
 * digest on every architecture (the ISA-specific repro is in the pack). */
static void s_reduce_i16_sat(void)
{
    uint32_t acc = 0u;
    size_t i;
    for (i = 0; i < N; i += 2u) {
        int32_t p = (int32_t)g_in[i] * 2 + (int32_t)g_in[i + 1u] * 2;
        acc += (uint32_t)sat_u8(p > 32767 ? 32767 : p);
    }
    g_out[0] = acc;
}
static void r_reduce_i16_sat(void)
{
    volatile uint32_t acc = 0u;
    volatile size_t i;
    for (i = 0; i < N; i += 2u) {
        volatile int32_t p = (int32_t)g_in[i] * 2 + (int32_t)g_in[i + 1u] * 2;
        acc = acc + (uint32_t)sat_u8(p > 32767 ? 32767 : p);
    }
    g_ref[0] = acc;
}

/* PF-15 / k_strcmp_signed.c: signed-char comparison carrier */
static void s_strcmp_signed(void)
{
    uint32_t acc = 0u, i;
    for (i = 0u; i < (uint32_t)N; i++) {
        int32_t d = (int32_t)(int8_t)(uint8_t)g_in[i] -
                    (int32_t)(int8_t)(uint8_t)g_in[N - 1u - i];
        acc += (uint32_t)(d & 0x7fffffff);
    }
    g_out[0] = acc;
}
static void r_strcmp_signed(void)
{
    volatile uint32_t acc = 0u, i;
    for (i = 0u; i < (uint32_t)N; i++) {
        volatile int32_t d = (int32_t)(int8_t)(uint8_t)g_in[i] -
                             (int32_t)(int8_t)(uint8_t)g_in[N - 1u - i];
        acc = acc + (uint32_t)(d & 0x7fffffff);
    }
    g_ref[0] = acc;
}

/* ------------------------------------------------------------------ */

typedef void (*shape_fn)(void);
typedef struct {
    const char *name;
    shape_fn    fast;
    shape_fn    ref;
    size_t      n;
    const char *origin;
} shape_t;

static const shape_t SHAPES[] = {
    { "axpy_f32",          s_axpy_f32,          r_axpy_f32,          N,  "BLAS saxpy / kernel_corpus/k20_axpy_f32.c" },
    { "map_u32",           s_map_u32,           r_map_u32,           N,  "k_map64_sub.c elementwise map" },
    { "sum_u32",           s_sum_u32,           r_sum_u32,           1,  "kernel_corpus/k02_sum8.c" },
    { "dot_f32",           s_dot_f32,           r_dot_f32,           1,  "kernel_corpus/k06_dot.c" },
    { "clamp_u8",          s_clamp_u8,          r_clamp_u8,          N,  "k23_clamp_u8.c / LightPixel" },
    { "fir4",              s_fir4,              r_fir4,              N,  "kernel_corpus/k21_fir4.c" },
    { "minpos",            s_minpos,            r_minpos,            2,  "kernel_corpus/k19_minpos_f32.c" },
    { "bytemask",          s_bytemask,          r_bytemask,          N,  "kernel_corpus/k15_bytemask.c / expat" },
    { "adler32",           s_adler32,           r_adler32,           1,  "zlib-ng k_adler32.c / k_adler32_do8.c" },
    { "crc32_braid8",      s_crc32_braid8,      r_crc32_braid8,      8,  "zlib-ng/gzip k_crc32_slice8.c" },
    { "utf8_classify",     s_utf8_classify,     r_utf8_classify,     N,  "expat/simdjson k_utf8_validate.c" },
    { "varint_scan",       s_varint_scan,       r_varint_scan,       1,  "SQLite k_sqlite_varint_scan.c / k_varint.c" },
    { "memchr_scan",       s_memchr_scan,       r_memchr_scan,       1,  "glibc k_memchr.c" },
    { "horspool",          s_horspool,          r_horspool,          1,  "glibc memmem k_memmem_horspool.c" },
    { "jhash",             s_jhash,             r_jhash,             N,  "Linux jhash / k_jhash.c" },
    { "fnv1a",             s_fnv1a,             r_fnv1a,             1,  "k_hashmix.c" },
    { "base64_decode",     s_base64_decode,     r_base64_decode,     N,  "curl/OpenSSL k_base64_decode.c" },
    { "matchlen",          s_matchlen,          r_matchlen,          1,  "gzip/zstd k_matchlen.c" },
    { "aos_cmul",          s_aos_cmul,          r_aos_cmul,          N,  "aos_index_roots.c / FFmpeg, Skia" },
    { "aos_crossadd",      s_aos_crossadd,      r_aos_crossadd,      N,  "LightPixel / joint_pixel_shapes.c" },
    { "dot2_multi_acc",    s_dot2_multi_acc,    r_dot2_multi_acc,    2,  "SA-01 multi-reduction arm" },
    { "fma_orientation",   s_fma_orientation,   r_fma_orientation,   N,  "SA-02 SLP packed-FMA orientation" },
    { "gather_stride4",    s_gather_stride4,    r_gather_stride4,    N,  "SA-25/26 / aos_pair_shapes.py" },
    { "masked_tail",       s_masked_tail,       r_masked_tail,       N,  "SA-24 predication / masked stores" },
    { "inclusive_exit",    s_inclusive_exit,    r_inclusive_exit,    1,  "SA-21 inclusive-exit IV" },
    { "chacha_rounds",     s_chacha_rounds,     r_chacha_rounds,     2,  "SA-06/07 vec_arx / arx_vectorize" },
    { "vzeroupper_trans",  s_vzeroupper_transition, r_vzeroupper_transition, 1, "SA-33 dirty-upper arming" },
    { "reduce_i16_sat",    s_reduce_i16_sat,    r_reduce_i16_sat,    1,  "SA-05 maddubs roles / word lanes" },
    { "strcmp_signed",     s_strcmp_signed,     r_strcmp_signed,     1,  "PF-15 k_strcmp_signed.c" },
};

#define SHAPE_COUNT (sizeof(SHAPES) / sizeof(SHAPES[0]))

static const shape_t *find_shape(const char *name)
{
    size_t i;
    for (i = 0; i < SHAPE_COUNT; i++) {
        if (strcmp(SHAPES[i].name, name) == 0) return &SHAPES[i];
    }
    return NULL;
}

static int run_shape(const shape_t *s, uint64_t *fast_digest, uint64_t *ref_digest)
{
    memset(g_out, 0, sizeof g_out);
    memset(g_ref, 0, sizeof g_ref);
    s->fast();
    s->ref();
    *fast_digest = fold(g_out, s->n);
    *ref_digest  = fold(g_ref, s->n);
    return *fast_digest == *ref_digest;
}

/* ISO C clock() only.  POSIX clock_gettime() would add a feature-macro
 * contract that lccc's bundled headers do not guarantee on every supported
 * target; clock() is C89, needs no feature macros, and measuring a batch of
 * iterations (not one) makes its coarse tick irrelevant. */
static double cpu_seconds(void)
{
    return (double)clock() / (double)CLOCKS_PER_SEC;
}

#define BENCH_REPS ((size_t)7)

static int bench(size_t iters)
{
    size_t i, j, rep;
    fprintf(stderr, "# shape median_ns_per_iter iters=%lu reps=%lu\n",
            (unsigned long)iters, (unsigned long)BENCH_REPS);
    for (i = 0; i < SHAPE_COUNT; i++) {
        double samples[BENCH_REPS];
        double median;
        for (rep = 0; rep < BENCH_REPS; rep++) {
            double t0 = cpu_seconds();
            for (j = 0; j < iters; j++) {
                SHAPES[i].fast();
                /* g_out is observable file-scope state, so the batch cannot
                 * be elided; the guard keeps the read explicit. */
                if (g_out[0] == 0xdeadbeefu) return 3;
            }
            samples[rep] = (cpu_seconds() - t0) * 1e9 / (double)iters;
        }
        /* Insertion sort over seven samples: no comparator, no libc call. */
        for (rep = 1; rep < BENCH_REPS; rep++) {
            double key = samples[rep];
            size_t k = rep;
            while (k > 0u && samples[k - 1u] > key) {
                samples[k] = samples[k - 1u];
                k--;
            }
            samples[k] = key;
        }
        median = samples[BENCH_REPS / 2u];
        fprintf(stderr, "%-20s %.1f\n", SHAPES[i].name, median);
    }
    return 0;
}

static int selftest(void)
{
    /* Independent constant for fold(): catches a broken oracle chain. */
    static const uint32_t probe[4] = { 1u, 2u, 3u, 4u };
    uint64_t expect = 1469598103934665603ULL;
    size_t i;
    int bad = 0;
    for (i = 0; i < 4u; i++) {
        expect ^= (uint64_t)probe[i];
        expect *= 1099511628211ULL;
    }
    if (fold(probe, 4u) != expect) {
        fprintf(stderr, "selftest: fold() chain is broken\n");
        return 1;
    }
    gen_inputs();
    for (i = 0; i < SHAPE_COUNT; i++) {
        uint64_t f, r;
        if (!run_shape(&SHAPES[i], &f, &r)) {
            fprintf(stderr, "selftest: %s fast=%016llx ref=%016llx\n",
                    SHAPES[i].name, (unsigned long long)f, (unsigned long long)r);
            bad = 1;
        }
    }
    printf("selftest %s shapes=%lu\n", bad ? "FAIL" : "PASS",
           (unsigned long)SHAPE_COUNT);
    return bad;
}

int main(int argc, char **argv)
{
    size_t i;
    const char *mode = (argc > 1) ? argv[1] : "all";

    if (strcmp(mode, "list") == 0) {
        for (i = 0; i < SHAPE_COUNT; i++)
            printf("%s\t%s\n", SHAPES[i].name, SHAPES[i].origin);
        return 0;
    }
    if (strcmp(mode, "bench") == 0) {
        size_t iters = BENCH_ITERS;
        if (argc > 2) {
            long v = strtol(argv[2], NULL, 10);
            if (v > 0 && v <= 100000L) iters = (size_t)v;
        }
        gen_inputs();
        return bench(iters);
    }
    if (strcmp(mode, "selftest") == 0) return selftest();

    if (strcmp(mode, "shape") == 0) {
        const shape_t *s;
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
    gen_inputs();
    /* Report EVERY shape even when one is wrong: a single early return hides
       which other flag sets or kernels share the defect.  The nonzero exit
       still tells the driver a mismatch happened, but the receipt now lists
       all 29 rows so the failure is diagnosable in one run. */
    {
        int bad = 0;
        for (i = 0; i < SHAPE_COUNT; i++) {
            uint64_t f, r;
            int ok = run_shape(&SHAPES[i], &f, &r);
            printf("%-20s %016llx %016llx %s\n", SHAPES[i].name,
                   (unsigned long long)f, (unsigned long long)r,
                   ok ? "ok" : "MISMATCH");
            bad |= !ok;
        }
        return bad;
    }
}
