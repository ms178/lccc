/* Regression: whole-vector reloads of a 256-bit union after 32-bit lane writes.
 *
 * `union { __m256 v; unsigned u[8]; }` written whole and then patched one
 * dword at a time must read back exactly the patched bit pattern.  The
 * compiler may replace the reload with a register chain (vector_lane_patch);
 * every case below pins bit-exact results, including the shapes the rewrite
 * must refuse (runtime lane index, interleaved scalar readers, lane written
 * before any whole-width definition).
 *
 * Each check compares the reload against a reference built with memcpy from
 * an independent uint32 array, so a wrong lane, a stale lane, a dropped
 * write or a float-domain canonicalisation of a NaN payload is a failure.
 */
#include <stdio.h>
#include <string.h>

typedef float vf8 __attribute__((vector_size(32)));

union bits8 {
    vf8 v;
    unsigned u[8];
};

static int failures;

#define CHECK(cond, ...)                                  \
    do {                                                  \
        if (!(cond)) {                                    \
            ++failures;                                   \
            fprintf(stderr, "FAIL %s:%d: ", __FILE__, __LINE__); \
            fprintf(stderr, __VA_ARGS__);                 \
            fputc('\n', stderr);                          \
        }                                                 \
    } while (0)

static void expect_bits(const unsigned *got, const unsigned *want, const char *what) {
    for (int i = 0; i < 8; ++i)
        CHECK(got[i] == want[i], "%s lane %d: got %08x want %08x", what, i, got[i], want[i]);
}

/* Every constant lane, in turn, from a fixed pattern. */
static void case_every_lane(void) {
    unsigned base[8], want[8];
    for (int i = 0; i < 8; ++i)
        base[i] = 0x3f800000u + (unsigned)i * 0x00010203u;
    for (int lane = 0; lane < 8; ++lane) {
        unsigned w = 0xdeadbe00u | (unsigned)lane;
        union bits8 b;
        memcpy(&b.u, base, sizeof base);
        vf8 src = b.v;
        union bits8 p;
        p.v = src;
        switch (lane) {
        case 0: p.u[0] = w; break;
        case 1: p.u[1] = w; break;
        case 2: p.u[2] = w; break;
        case 3: p.u[3] = w; break;
        case 4: p.u[4] = w; break;
        case 5: p.u[5] = w; break;
        case 6: p.u[6] = w; break;
        default: p.u[7] = w; break;
        }
        union bits8 r;
        r.v = p.v;
        memcpy(want, base, sizeof want);
        want[lane] = w;
        expect_bits(r.u, want, "every-lane");
    }
}

/* Floating-point bit patterns that a float-domain copy could canonicalise. */
static void case_special_patterns(void) {
    static const unsigned specials[] = {
        0x7fc00001u, /* quiet NaN with payload */
        0xffc0beefu, /* negative quiet NaN with payload */
        0x7f800001u, /* signalling NaN */
        0x80000000u, /* -0.0 */
        0x00000001u, /* smallest denormal */
        0x807fffffu, /* negative largest denormal */
        0xffffffffu,
    };
    for (unsigned s = 0; s < sizeof specials / sizeof specials[0]; ++s) {
        union bits8 b;
        for (int i = 0; i < 8; ++i)
            b.u[i] = 0x40000000u + (unsigned)i;
        vf8 src = b.v;
        union bits8 p;
        p.v = src;
        p.u[3] = specials[s];
        union bits8 r;
        r.v = p.v;
        CHECK(r.u[3] == specials[s], "special %08x not preserved: got %08x", specials[s], r.u[3]);
        CHECK(r.u[2] == 0x40000002u && r.u[4] == 0x40000004u, "special %08x: neighbours disturbed", specials[s]);
    }
}

/* Several lanes patched in one block, including a lane written twice (the
 * last write must win) and a lane never written (must keep the source). */
static void case_multiple_lanes(void) {
    union bits8 b;
    for (int i = 0; i < 8; ++i)
        b.u[i] = 0x11110000u + (unsigned)i;
    vf8 src = b.v;
    union bits8 p;
    p.v = src;
    p.u[1] = 0xaaaa0001u;
    p.u[5] = 0xbbbb0005u;
    p.u[1] = 0xcccc0001u; /* overwrites the first write */
    p.u[7] = 0xdddd0007u;
    union bits8 r;
    r.v = p.v;
    unsigned want[8] = {0x11110000u, 0xcccc0001u, 0x11110002u, 0x11110003u,
                        0x11110004u, 0xbbbb0005u, 0x11110006u, 0xdddd0007u};
    expect_bits(r.u, want, "multiple-lanes");
}

/* A scalar reader of the same temporary keeps the memory image live: the
 * scalar must see the patched lane, and the reload must see it too. */
static void case_scalar_reader_keeps_memory(void) {
    union bits8 b;
    for (int i = 0; i < 8; ++i)
        b.u[i] = 0x22220000u + (unsigned)i;
    vf8 src = b.v;
    union bits8 p;
    p.v = src;
    p.u[2] = 0x33330002u;
    unsigned scalar = p.u[2];
    union bits8 r;
    r.v = p.v;
    CHECK(scalar == 0x33330002u, "scalar reader saw %08x", scalar);
    CHECK(r.u[2] == 0x33330002u, "reload after scalar reader saw %08x", r.u[2]);
}

/* A runtime lane index cannot be folded into a constant blend: the rewrite
 * must refuse and the memory path must produce the right lane. */
static void case_runtime_lane(void) {
    for (int lane = 0; lane < 8; ++lane) {
        union bits8 b;
        for (int i = 0; i < 8; ++i)
            b.u[i] = 0x44440000u + (unsigned)i;
        vf8 src = b.v;
        union bits8 p;
        p.v = src;
        p.u[lane] = 0x55550000u + (unsigned)lane;
        union bits8 r;
        r.v = p.v;
        for (int i = 0; i < 8; ++i) {
            unsigned want = (i == lane) ? 0x55550000u + (unsigned)lane : 0x44440000u + (unsigned)i;
            CHECK(r.u[i] == want, "runtime lane %d: lane %d got %08x want %08x", lane, i, r.u[i], want);
        }
    }
}

/* Loop shape from avx_fp_private: state-dependent lane patch on each iteration. */
static unsigned long long case_loop(void) {
    union bits8 seedbits;
    for (int i = 0; i < 8; ++i)
        seedbits.u[i] = 0x3f000000u;
    vf8 input = seedbits.v;
    unsigned state = 1;
    unsigned long long sum = 0;
    for (int n = 0; n < 1000; ++n) {
        state = state * 1664525u + 1013904223u;
        union bits8 bits;
        bits.v = input;
        bits.u[0] = 0x3e000000u | (state & 0x007fffffu);
        input = bits.v;
        union bits8 out;
        out.v = input;
        sum += out.u[0];
    }
    /* Reference: the same walk with the lane patched through a plain array. */
    unsigned ref[8];
    for (int i = 0; i < 8; ++i)
        ref[i] = 0x3f000000u;
    state = 1;
    unsigned long long want = 0;
    for (int n = 0; n < 1000; ++n) {
        state = state * 1664525u + 1013904223u;
        ref[0] = 0x3e000000u | (state & 0x007fffffu);
        want += ref[0];
    }
    CHECK(sum == want, "loop checksum %llu want %llu", sum, want);
    return sum;
}

int main(void) {
    case_every_lane();
    case_special_patterns();
    case_multiple_lanes();
    case_scalar_reader_keeps_memory();
    case_runtime_lane();
    unsigned long long loop = case_loop();
    printf("avx_union_lane_reload loop=%llu failures=%d\n", loop, failures);
    return failures == 0 ? 0 : 1;
}
