/* Regression: a 256-bit datum store must not strand a value that has further
 * readers.
 *
 * A whole-vector store of a register-resident value (`VecStoreF32x8`) can
 * consume that value's deferred home store: the store already moved the
 * value's bits out of the register. Once consumed, the value's stack home is
 * no longer written by that deferral, so every LATER reader of the value must
 * still see the correct bits. These shapes exercise that contract directly:
 *
 *   - a patched value stored to memory, then clobbered by a callee, then read
 *     again lane by lane and as a whole vector;
 *   - the same value stored to two different destinations with a clobber in
 *     between;
 *   - a value consumed by a whole-vector operation after its datum store;
 *   - NaN payloads and signed zeros in the lanes that are read back.
 *
 * Every observed lane is compared bit-for-bit against an independent scalar
 * model. The program prints a summary line and exits with the failure count,
 * so both stdout and the exit status are compared against GCC.
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

/* Arrays that the stores target. Global so their addresses escape. */
static vf8 sink_a[4];
static vf8 sink_b[4];
/* Clobber output: must never alias the stores under test. */
static vf8 clobber_pad[4];
static unsigned clobber_seed;

/* Writes every vector register the caller might be holding a value in. */
__attribute__((noinline)) static void clobber_vectors(void) {
    union bits8 junk;
    for (int i = 0; i < 8; ++i)
        junk.u[i] = 0xdeadbeefu ^ (unsigned)(i * 0x9e3779b9u) ^ clobber_seed;
    for (int r = 0; r < 4; ++r) {
        vf8 t = junk.v * (float)(r + 3);
        clobber_pad[r] = t + junk.v;
        clobber_pad[(r + 1) & 3] += t - junk.v;
    }
    clobber_seed ^= clobber_pad[0][0] > 0.0f ? 1u : 0u;
}

/* Patch lane `lane` of a loaded base, store the patched value to `dst`, clobber
 * every vector register, then read the same value back lane by lane. */
static void case_store_clobber_reread(int lane, unsigned patch) {
    unsigned base[8], want[8];
    for (int i = 0; i < 8; ++i)
        base[i] = 0x3f800000u + (unsigned)i * 0x00010001u;
    union bits8 src;
    memcpy(src.u, base, sizeof base);
    vf8 in = src.v;

    union bits8 p;
    p.v = in;
    p.u[lane] = patch;

    /* Datum store: may consume the value's deferred home store. */
    sink_a[0] = p.v;
    clobber_vectors();

    memcpy(want, base, sizeof want);
    want[lane] = patch;

    /* Later readers of the SAME value: the lane reads and a whole-vector copy
     * must observe the patched bits, not a stale home. */
    union bits8 back;
    back.v = p.v;
    expect_bits(back.u, want, "store-clobber-reread (whole)");
    for (int i = 0; i < 8; ++i)
        CHECK(p.u[i] == want[i], "store-clobber-reread (lane) %d: got %08x want %08x",
              i, p.u[i], want[i]);
}

/* The same value is stored to two destinations with a clobber in between. */
static void case_two_destinations(unsigned patch) {
    unsigned base[8], want[8];
    for (int i = 0; i < 8; ++i)
        base[i] = 0x40000000u + (unsigned)i * 0x00000003u;
    union bits8 src;
    memcpy(src.u, base, sizeof base);
    vf8 in = src.v;

    union bits8 p;
    p.v = in;
    p.u[5] = patch;

    sink_a[1] = p.v;
    clobber_vectors();
    sink_b[1] = p.v;
    clobber_vectors();

    memcpy(want, base, sizeof want);
    want[5] = patch;
    union bits8 a, b;
    a.v = sink_a[1];
    b.v = sink_b[1];
    expect_bits(a.u, want, "two-destinations first");
    expect_bits(b.u, want, "two-destinations second");
}

/* The stored value is then consumed by a whole-vector operation. Bit-exact
 * addition of the value to a constant must match the scalar model. */
static void case_store_then_vector_op(unsigned patch) {
    unsigned base[8], want[8];
    for (int i = 0; i < 8; ++i)
        base[i] = 0x3f800000u + (unsigned)i;
    union bits8 src;
    memcpy(src.u, base, sizeof base);

    union bits8 p;
    p.v = src.v;
    p.u[2] = patch;

    sink_b[2] = p.v;
    clobber_vectors();

    union bits8 r;
    r.v = p.v * 1.0f;
    memcpy(want, base, sizeof want);
    want[2] = patch;
    expect_bits(r.u, want, "store-then-vector-op");
}

/* NaN payloads and a signed zero in lanes that are read back after the
 * clobber. */
static void case_special_values(void) {
    const unsigned specials[4] = { 0x7fc0beefu, 0xffa12345u, 0x80000000u, 0x00000001u };
    for (int k = 0; k < 4; ++k) {
        unsigned base[8], want[8];
        for (int i = 0; i < 8; ++i)
            base[i] = 0x3f800000u;
        union bits8 src;
        memcpy(src.u, base, sizeof base);
        union bits8 p;
        p.v = src.v;
        p.u[k * 2] = specials[k];
        sink_a[3] = p.v;
        clobber_vectors();
        memcpy(want, base, sizeof want);
        want[k * 2] = specials[k];
        union bits8 back;
        back.v = sink_a[3];
        expect_bits(back.u, want, "special-values");
    }
}

int main(void) {
    for (int lane = 0; lane < 8; ++lane)
        for (unsigned k = 0; k < 4; ++k)
            case_store_clobber_reread(lane, 0xa0000000u | (k << 8) | (unsigned)lane);
    for (unsigned k = 0; k < 8; ++k)
        case_two_destinations(0xb0000000u | k);
    for (unsigned k = 0; k < 8; ++k)
        case_store_then_vector_op(0x40400000u + k);
    case_special_values();

    printf("avx_pending_store_multiuse: %s (%d failure%s)\n",
           failures ? "FAIL" : "PASS", failures, failures == 1 ? "" : "s");
    return failures ? 1 : 0;
}
