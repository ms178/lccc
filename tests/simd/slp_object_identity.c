/*
 * BB-SLP object-identity regression corpus (2026-11-08).
 *
 * The PostVec unroller clones a loop body; every clone carries its OWN
 * `GlobalAddr` SSA value for the same file-scope array.  `global_addr_cse`
 * deliberately keeps those site-local indexed bases in distinct webs (each
 * one folds into an independent `sym(,%idx,scale)` RIP-relative memory
 * operand, saving a GPR), so duplicate roots are a PERMANENT property of
 * the IR, not a transient bug to be CSE'd away.
 *
 * `collect_seed_candidates` used to key store streams by that SSA value id.
 * The clones of `out[i]`, `out[i+1]`, ... therefore landed in different
 * streams and the packer produced ZERO seeds.  With object-identity keys the
 * clones form one consecutive run and the packer can fire.
 *
 * This corpus is both a positive optimization gate and a set of NEGATIVE
 * CONTROLS: every case where grouping would be unsound must stay scalar.
 * All kernels are checksummed against a volatile scalar oracle in the same
 * translation unit, so a miscompile is a hard failure rather than a timing
 * difference.  Build with -std=c99 -O2 -march=native; run with `all`.
 */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define N 257u /* deliberately NOT a multiple of any SIMD width */

static uint32_t g_in[N];
static uint32_t g_out[N];
static uint32_t g_other[N];
static uint8_t g_bytes_in[N];
static uint8_t g_bytes_out[N];
static uint16_t g_words_in[N];
static uint16_t g_words_out[N];
static float g_f32_in[N];
static float g_f32_out[N];

static uint32_t state = 0x9e3779b9u;
static uint32_t xs(void) {
    uint32_t x = state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    state = x;
    return x;
}
static void seed(uint32_t s) { state = s ? s : 1u; }

static uint32_t fnv(const void *p, size_t n) {
    const unsigned char *q = (const unsigned char *)p;
    uint32_t h = 0x811c9dc5u;
    for (size_t i = 0; i < n; ++i) { h ^= q[i]; h *= 0x01000193u; }
    return h;
}

/* 1. Positive: integer classifier.  Two unrolled stores to ONE global.  The
 *    original defect left this 100% scalar; the fix must emit a packed store. */
static void k_classifier_u32(void) {
    for (size_t i = 0; i < N; ++i) {
        uint32_t c = g_in[i] & 0x7fu;
        uint32_t hit = 0;
        if (c >= 'A' && c <= 'Z') hit |= 1u;
        if (c >= 'a' && c <= 'z') hit |= 2u;
        if (c >= '0' && c <= '9') hit |= 4u;
        g_out[i] = hit;
    }
}
static void r_classifier_u32(void) {
    for (size_t i = 0; i < N; ++i) {
        volatile uint32_t c = g_in[i] & 0x7fu;
        volatile uint32_t hit = 0;
        if (c >= 'A' && c <= 'Z') hit |= 1u;
        if (c >= 'a' && c <= 'z') hit |= 2u;
        if (c >= '0' && c <= '9') hit |= 4u;
        g_out[i] = hit;
    }
}

/* 2. Negative control: two DISTINCT globals.  Object identity must NOT merge
 *    them into one stream (distinct names are distinct objects, but their
 *    stores are interleaved in program order, so packing them into one vector
 *    store would need a cross-object shuffle that does not exist). */
static void k_two_globals(void) {
    for (size_t i = 0; i + 1 < N; i += 2) {
        g_out[i]     = g_in[i] * 3u + 7u;
        g_other[i]   = g_in[i] ^ 0xa5a5a5a5u;
        g_out[i + 1] = g_in[i + 1] * 3u + 7u;
        g_other[i + 1] = g_in[i + 1] ^ 0xa5a5a5a5u;
    }
}
static void r_two_globals(void) {
    for (size_t i = 0; i + 1 < N; i += 2) {
        volatile uint32_t a = g_in[i];
        volatile uint32_t b = g_in[i + 1];
        g_out[i]     = a * 3u + 7u;
        g_other[i]   = a ^ 0xa5a5a5a5u;
        g_out[i + 1] = b * 3u + 7u;
        g_other[i + 1] = b ^ 0xa5a5a5a5u;
    }
}

/* 3. Negative control: an intervening write to the SAME object between two
 *    lanes.  Rule (d) must decline: a vector store would clobber the
 *    intermediate value. */
static void k_intervening_write(void) {
    for (size_t i = 0; i + 8 < N; i += 8) {
        uint32_t v = g_in[i];
        g_out[i]     = v + 1u;
        g_out[i + 1] = v + 2u;
        g_out[i]     = v + 3u;       /* clobbers lane 0 */
        g_out[i + 2] = v + 4u;
        g_out[i + 3] = v + 5u;
    }
}
static void r_intervening_write(void) {
    for (size_t i = 0; i + 8 < N; i += 8) {
        volatile uint32_t v = g_in[i];
        g_out[i]     = v + 1u;
        g_out[i + 1] = v + 2u;
        g_out[i]     = v + 3u;
        g_out[i + 2] = v + 4u;
        g_out[i + 3] = v + 5u;
    }
}

/* 4. Positive, byte lanes: saturating clamp with two selects per lane. */
static void k_clamp_u8(void) {
    for (size_t i = 0; i < N; ++i) {
        int v = (int)g_bytes_in[i] - 64;
        g_bytes_out[i] = (uint8_t)(v < 0 ? 0 : (v > 255 ? 255 : v));
    }
}
static void r_clamp_u8(void) {
    for (size_t i = 0; i < N; ++i) {
        volatile int v = (int)g_bytes_in[i] - 64;
        g_bytes_out[i] = (uint8_t)(v < 0 ? 0 : (v > 255 ? 255 : v));
    }
}

/* 5. Positive, word lanes. */
static void k_word_map(void) {
    for (size_t i = 0; i < N; ++i)
        g_words_out[i] = (uint16_t)((g_words_in[i] * 5u + 11u) & 0xffffu);
}
static void r_word_map(void) {
    for (size_t i = 0; i < N; ++i) {
        volatile uint32_t v = g_words_in[i];
        g_words_out[i] = (uint16_t)((v * 5u + 11u) & 0xffffu);
    }
}

/* 6. Positive, FP lanes: exact lane-parallel arithmetic, no reassociation. */
static void k_f32_map(void) {
    for (size_t i = 0; i < N; ++i)
        g_f32_out[i] = g_f32_in[i] * 1.5f + 2.25f;
}
static void r_f32_map(void) {
    for (size_t i = 0; i < N; ++i) {
        volatile float v = g_f32_in[i];
        g_f32_out[i] = v * 1.5f + 2.25f;
    }
}

/* 7. Edge cases: exact SIMD-width trips and a zero-length trip.  The kernel
 *    takes `lo`/`hi` so one compiled body covers both boundaries. */
static void k_bounded(uint32_t lo, uint32_t hi) {
    for (uint32_t i = lo; i < hi; ++i)
        g_out[i] = (g_in[i] ^ 0x5a5a5a5au) + 1u;
}
static void r_bounded(uint32_t lo, uint32_t hi) {
    for (uint32_t i = lo; i < hi; ++i) {
        volatile uint32_t v = g_in[i];
        g_out[i] = (v ^ 0x5a5a5a5au) + 1u;
    }
}

struct kernel {
    const char *name;
    void (*run)(void);
    void (*ref)(void);
    const void *out;
    size_t out_bytes;
};

static void prepare(void) {
    seed(0x1234abcdu);
    for (size_t i = 0; i < N; ++i) {
        g_in[i] = xs();
        g_bytes_in[i] = (uint8_t)xs();
        g_words_in[i] = (uint16_t)xs();
        g_f32_in[i] = (float)(int32_t)(xs() & 0xffffu) / 4096.0f;
        g_out[i] = 0xdeadbeefu;
        g_other[i] = 0xdeadbeefu;
        g_bytes_out[i] = 0x5au;
        g_words_out[i] = 0x5a5au;
        g_f32_out[i] = -1.0f;
    }
}

static const struct kernel kernels[] = {
    {"classifier_u32", k_classifier_u32, r_classifier_u32, g_out, sizeof g_out},
    {"two_globals", k_two_globals, r_two_globals, g_out, sizeof g_out},
    {"intervening_write", k_intervening_write, r_intervening_write, g_out, sizeof g_out},
    {"clamp_u8", k_clamp_u8, r_clamp_u8, g_bytes_out, sizeof g_bytes_out},
    {"word_map", k_word_map, r_word_map, g_words_out, sizeof g_words_out},
    {"f32_map", k_f32_map, r_f32_map, g_f32_out, sizeof g_f32_out},
};

int main(int argc, char **argv) {
    const char *which = argc > 1 ? argv[1] : "all";
    int fails = 0;
    for (size_t k = 0; k < sizeof kernels / sizeof kernels[0]; ++k) {
        if (strcmp(which, "all") != 0 && strcmp(which, kernels[k].name) != 0) continue;
        prepare();
        kernels[k].run();
        uint32_t got = fnv(kernels[k].out, kernels[k].out_bytes);
        /* Two-globals also writes g_other; include it in the digest. */
        if (strcmp(kernels[k].name, "two_globals") == 0)
            got ^= fnv(g_other, sizeof g_other);
        uint32_t got_zero = got;
        prepare();
        kernels[k].ref();
        uint32_t want = fnv(kernels[k].out, kernels[k].out_bytes);
        if (strcmp(kernels[k].name, "two_globals") == 0)
            want ^= fnv(g_other, sizeof g_other);
        int ok = got == want;
        printf("%-22s %s got=%08x want=%08x\n", kernels[k].name,
               ok ? "ok" : "FAIL", got_zero, want);
        if (!ok) ++fails;
    }
    /* Bounded-window edge cases: 0, 1, 3, 7, 8, 15, 16, 17, 32, 33 elements. */
    static const uint32_t bounds[][2] = {
        {0, 0}, {0, 1}, {0, 3}, {0, 7}, {0, 8}, {0, 15},
        {0, 16}, {0, 17}, {0, 32}, {0, 33}, {7, 7}, {7, 40}, {200, 257},
    };
    for (size_t b = 0; b < sizeof bounds / sizeof bounds[0]; ++b) {
        if (strcmp(which, "all") != 0 && strcmp(which, "bounded") != 0) break;
        uint32_t lo = bounds[b][0], hi = bounds[b][1];
        prepare(); k_bounded(lo, hi);
        uint32_t got = fnv(g_out, sizeof g_out);
        prepare(); r_bounded(lo, hi);
        uint32_t want = fnv(g_out, sizeof g_out);
        int ok = got == want;
        printf("bounded_%u_%u          %s got=%08x want=%08x\n", lo, hi,
               ok ? "ok" : "FAIL", got, want);
        if (!ok) ++fails;
    }
    return fails ? 1 : 0;
}
