/* Self-checking regression for the generator-map vectorizer
 * (src/passes/vectorize_generator_map.rs).
 *
 * Each kernel is a pure induction-variable byte generator, dst[i] = (T)F(i).
 * The expected bytes come from a second loop whose index passes through an
 * empty asm barrier, so the reference is never vectorized and cannot share a
 * bug with the transform under test.
 *
 * Shapes covered: u32 and size_t inductions; trip counts that are, and are not,
 * multiples of 32; nonzero starts; trips shorter than one packed group (the
 * transform must decline); shifts, xor/or/and/sub/mul mixes; and signed
 * byte stores. Exit status 0 means every element matched. */
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

#define BARRIER(v) __asm__ volatile("" : "+r"(v))

static unsigned char a_hist[1u << 18];
static unsigned char a_ref[1u << 18];
static unsigned char b_gen[4096 + 7];
static unsigned char b_ref[4096 + 7];
static unsigned char c_gen[1000];
static unsigned char c_ref[1000];
static unsigned char d_gen[64];
static unsigned char d_ref[64];
static unsigned char e_gen[32];
static unsigned char e_ref[32];
static signed char f_gen[777];
static signed char f_ref[777];
static unsigned char g_gen[20];
static unsigned char g_ref[20];

/* A: histogram-init shape, u32 induction, full 2^18, multiplicative hash. */
static void gen_a(void) {
    for (unsigned i = 0; i < (1u << 18); ++i) {
        unsigned x = (i * 2654435761u) >> 24;
        a_hist[i] = (unsigned char)x;
    }
    for (unsigned i = 0; i < (1u << 18); ++i) {
        unsigned j = i;
        BARRIER(j);
        unsigned x = (j * 2654435761u) >> 24;
        a_ref[j] = (unsigned char)x;
    }
}

/* B: size_t induction, trip 4103 (not a multiple of 32), i*37+11 narrowed. */
static void gen_b(void) {
    for (size_t i = 0; i < sizeof b_gen; i++)
        b_gen[i] = (unsigned char)(i * 37 + 11);
    for (size_t i = 0; i < sizeof b_ref; i++) {
        size_t j = i;
        BARRIER(j);
        b_ref[j] = (unsigned char)(j * 37 + 11);
    }
}

/* C: nonzero start (C0 = 5), trip 995, mixed xor/shift/or/sub chain. */
static void gen_c(void) {
    for (unsigned i = 5; i < 1000; ++i)
        c_gen[i] = (unsigned char)((((i ^ 0x5au) * 3u) + (i >> 2) - 7u) | 1u);
    for (unsigned i = 5; i < 1000; ++i) {
        unsigned j = i;
        BARRIER(j);
        c_ref[j] = (unsigned char)((((j ^ 0x5au) * 3u) + (j >> 2) - 7u) | 1u);
    }
}

/* D: exactly two packed groups, shifts and masks (no tail). */
static void gen_d(void) {
    for (unsigned i = 0; i < 64; ++i)
        d_gen[i] = (unsigned char)(((i << 3) & 0xF0u) ^ i);
    for (unsigned i = 0; i < 64; ++i) {
        unsigned j = i;
        BARRIER(j);
        d_ref[j] = (unsigned char)(((j << 3) & 0xF0u) ^ j);
    }
}

/* E: exactly one packed group, start zero (no scalar tail at all). */
static void gen_e(void) {
    for (unsigned i = 0; i < 32; ++i)
        e_gen[i] = (unsigned char)(i * 7u + 3u);
    for (unsigned i = 0; i < 32; ++i) {
        unsigned j = i;
        BARRIER(j);
        e_ref[j] = (unsigned char)(j * 7u + 3u);
    }
}

/* F: signed byte store from a size_t induction (I8 store, U64 chain). */
static void gen_f(void) {
    for (size_t i = 0; i < sizeof f_gen; i++)
        f_gen[i] = (signed char)((long)i * -5 + 100);
    for (size_t i = 0; i < sizeof f_ref; i++) {
        size_t j = i;
        BARRIER(j);
        f_ref[j] = (signed char)((long)j * -5 + 100);
    }
}

/* G: trip shorter than one packed group, which the transform must decline. */
static void gen_g(void) {
    for (unsigned i = 0; i < 20; ++i)
        g_gen[i] = (unsigned char)(i * 11u);
    for (unsigned i = 0; i < 20; ++i) {
        unsigned j = i;
        BARRIER(j);
        g_ref[j] = (unsigned char)(j * 11u);
    }
}

static int check(const char *name, const void *got, const void *want, size_t n) {
    if (memcmp(got, want, n) == 0)
        return 0;
    size_t first = 0;
    const unsigned char *g = got, *w = want;
    while (first < n && g[first] == w[first])
        first++;
    printf("MISMATCH %s at %zu: got %u want %u\n", name, first, g[first], w[first]);
    return 1;
}

int main(void) {
    gen_a();
    gen_b();
    gen_c();
    gen_d();
    gen_e();
    gen_f();
    gen_g();
    int bad = 0;
    bad += check("a", a_hist, a_ref, sizeof a_hist);
    bad += check("b", b_gen, b_ref, sizeof b_gen);
    bad += check("c", c_gen + 5, c_ref + 5, sizeof c_gen - 5);
    bad += check("d", d_gen, d_ref, sizeof d_gen);
    bad += check("e", e_gen, e_ref, sizeof e_gen);
    bad += check("f", f_gen, f_ref, sizeof f_gen);
    bad += check("g", g_gen, g_ref, sizeof g_gen);
    if (bad) {
        printf("generator_map_vec: %d kernel(s) mismatched\n", bad);
        return 1;
    }
    printf("generator_map_vec: ok\n");
    return 0;
}
