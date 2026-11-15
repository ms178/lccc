/* Regression: a lane-patch broadcast must not disturb a vector result that is
 * still pending in the scratch register.
 *
 * `union { __m256 v; unsigned u[8]; }` written whole, then patched in one or
 * more 32-bit lanes, then reloaded whole is rewritten into a register chain
 * (`vector_lane_patch`): one VecBroadcastI32x8 per patched lane feeding a
 * BlendPs256.  The x86 emitter used to stage each broadcast through %ymm0,
 * which holds the pending base value, so it had to commit that value to its
 * stack home and reload it: a 32-byte store and reload on every patch, on the
 * loop-carried dependency chain.  The staging now avoids the pending bank.
 *
 * The shapes below stress exactly the interleavings that change touches:
 *   - two patches in one block, the second broadcast while the first base is
 *     still pending;
 *   - a whole-vector operation consuming the pending value between patches;
 *   - a loop whose patched vector is clobbered by a callee every iteration, so
 *     the value must really round-trip through memory and be correct;
 *   - NaN payloads, signed zeros and denormals in patched lanes.
 * Every result is compared bit-for-bit against an independent scalar model.
 */
#include <stdio.h>
#include <string.h>

typedef float vf8 __attribute__((vector_size(32)));

union bits8 {
    vf8 v;
    unsigned u[8];
};

static int failures;

#define CHECK(cond, ...)                                              \
    do {                                                              \
        if (!(cond)) {                                                \
            ++failures;                                               \
            fprintf(stderr, "FAIL %s:%d: ", __FILE__, __LINE__);      \
            fprintf(stderr, __VA_ARGS__);                             \
            fputc('\n', stderr);                                      \
        }                                                             \
    } while (0)

static void expect_bits(const unsigned *got, const unsigned *want, const char *what) {
    for (int i = 0; i < 8; ++i)
        CHECK(got[i] == want[i], "%s lane %d: got %08x want %08x", what, i, got[i], want[i]);
}

/* Two patches in one block: the second broadcast runs while the first patched
 * value is the pending base. */
static void case_two_patches_chain(void) {
    for (int a = 0; a < 8; ++a) {
        for (int b = 0; b < 8; ++b) {
            unsigned base[8], want[8];
            for (int i = 0; i < 8; ++i)
                base[i] = 0x3f800000u + (unsigned)i * 0x00010001u;
            union bits8 src;
            memcpy(src.u, base, sizeof base);
            vf8 in = src.v;
            union bits8 p;
            p.v = in;
            p.u[a] = 0xa0000000u | (unsigned)a;
            p.u[b] = 0xb0000000u | (unsigned)b; /* b may equal a: last write wins */
            union bits8 r;
            r.v = p.v;
            memcpy(want, base, sizeof want);
            want[a] = 0xa0000000u | (unsigned)a;
            want[b] = 0xb0000000u | (unsigned)b;
            expect_bits(r.u, want, "two-patch-chain");
        }
    }
}

/* A whole-vector multiply consumes the pending patched value before the reload
 * is used; the result must match the scalar model lane by lane. */
static void case_patch_then_vector_op(void) {
    unsigned base[8];
    for (int i = 0; i < 8; ++i)
        base[i] = 0x40000000u + (unsigned)i;
    union bits8 src;
    memcpy(src.u, base, sizeof base);
    vf8 in = src.v;
    union bits8 p;
    p.v = in;
    p.u[5] = 0x41000000u; /* 8.0f */
    vf8 two = {2, 2, 2, 2, 2, 2, 2, 2};
    vf8 doubled = p.v * two;
    union bits8 r;
    r.v = doubled;
    unsigned want[8];
    for (int i = 0; i < 8; ++i) {
        float f;
        unsigned w = (i == 5) ? 0x41000000u : base[i];
        memcpy(&f, &w, 4);
        f = f * 2.0f;
        memcpy(&want[i], &f, 4);
    }
    expect_bits(r.u, want, "patch-then-vector-op");
}

/* Clobbers the caller-saved vector state: the loop value can only survive the
 * call through its memory home. */
__attribute__((noinline)) static void clobber(vf8 *scratch) {
    vf8 k = {3, 3, 3, 3, 3, 3, 3, 3};
    for (int i = 0; i < 8; ++i)
        scratch[i] = scratch[i] * k + k;
}

/* The benchmark's loop shape, checked against a plain-array model over many
 * iterations: one lane patched per iteration from a running state. */
static unsigned long long case_loop(void) {
    vf8 input[8];
    vf8 scratch[8];
    for (int i = 0; i < 8; ++i) {
        unsigned tmp[8];
        for (int j = 0; j < 8; ++j)
            tmp[j] = 0x3f000000u;
        memcpy(&input[i], tmp, sizeof tmp);
        memcpy(&scratch[i], tmp, sizeof tmp);
    }
    unsigned long long sum = 0, want = 0;
    unsigned state = 1;
    unsigned mstate = 1;
    /* Plain-array model of lane 0: the value the vector loop must carry. */
    unsigned model0 = 0x3f000000u;
    for (int n = 0; n < 4000; ++n) {
        state = state * 1664525u + 1013904223u;
        union bits8 bits;
        bits.v = input[0];
        bits.u[0] = 0x3e000000u | (state & 0x007fffffu);
        input[0] = bits.v;
        clobber(scratch);
        union bits8 out;
        out.v = input[0];
        sum += out.u[0];

        mstate = mstate * 1664525u + 1013904223u;
        model0 = 0x3e000000u | (mstate & 0x007fffffu);
        want += model0;
    }
    CHECK(sum == want, "loop checksum %llu want %llu", sum, want);
    return sum;
}

/* Special patterns in a patched lane while a neighbour is pending: bit-exact. */
static void case_special_patterns_pending(void) {
    static const unsigned specials[] = {
        0x7fc00001u, 0xffc0beefu, 0x7f800001u, 0x80000000u,
        0x00000001u, 0x807fffffu, 0xffffffffu, 0x00000000u,
    };
    for (unsigned s = 0; s < sizeof specials / sizeof specials[0]; ++s) {
        unsigned base[8];
        for (int i = 0; i < 8; ++i)
            base[i] = 0x40400000u + (unsigned)i;
        union bits8 src;
        memcpy(src.u, base, sizeof base);
        vf8 in = src.v;
        union bits8 p;
        p.v = in;
        p.u[2] = specials[s];
        p.u[6] = specials[(s + 1) % 8];
        union bits8 r;
        r.v = p.v;
        unsigned want[8];
        memcpy(want, base, sizeof want);
        want[2] = specials[s];
        want[6] = specials[(s + 1) % 8];
        expect_bits(r.u, want, "special-pending");
    }
}

int main(void) {
    case_two_patches_chain();
    case_patch_then_vector_op();
    case_special_patterns_pending();
    unsigned long long loop = case_loop();
    printf("avx_lane_broadcast_pending loop=%llu failures=%d\n", loop, failures);
    return failures == 0 ? 0 : 1;
}
