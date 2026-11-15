/* Red-team regression for vector_lane_patch: adversarial shapes.
 *
 * avx_union_lane_reload.c pins the common shapes.  This file pins the ones a
 * block-local store-forwarding rewrite most easily gets wrong:
 *
 *   loop_carried_read    a loop whose body writes and reloads one union that
 *                        is initialised outside the loop: the planner must
 *                        refuse the cross-block temporary and the loop must
 *                        still carry state through memory correctly (the
 *                        block-local incoming-read rule is pinned by the unit
 *                        test scalar_read_before_the_define_keeps_...);
 *   q_span / bytes       a 64-bit or byte read of lanes written earlier in the
 *                        block must observe exactly those lanes;
 *   nan_payloads         signalling and negative NaN payloads and negative
 *                        zero must survive the blend bit for bit;
 *   escape               a pointer to a lane handed to a call must refuse the
 *                        rewrite and see the callee's store;
 *   cross_block          a write in one block observed in a later block must
 *                        refuse the rewrite;
 *   redefine             a second whole definition must shadow the first
 *                        without corrupting a reload taken before it;
 *   observed_then_write  a scalar read between two writes to one lane must see
 *                        the first write.
 *
 * Every result is compared against a plain uint32 array reference, so a stale
 * lane, a dropped observed write or a payload canonicalised by the rewrite is a
 * failure.  Exit status is 0 only when every check holds.
 */
#include <stdio.h>
#include <string.h>

typedef float vf8 __attribute__((vector_size(32)));

union b8 {
    vf8 v;
    unsigned u[8];
    unsigned char c[32];
    unsigned long long q[4];
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

static vf8 vec_from(const unsigned *w) {
    vf8 t;
    memcpy(&t, w, sizeof t);
    return t;
}

/* The union is initialised before the loop, so its accesses span two blocks and
 * the rewrite must refuse it.  The loop body reads lane 0 before writing it, so
 * the carried state is exercised through memory on every iteration. */
static unsigned long long case_loop_carried_read(void) {
    unsigned ref[8], src[8], rv[8];
    for (int i = 0; i < 8; ++i)
        ref[i] = 0x3f800000u + (unsigned)i;
    union b8 acc;
    acc.v = vec_from(ref);
    memcpy(rv, ref, sizeof rv);
    unsigned long long sum = 0, refsum = 0;
    for (unsigned n = 0; n < 200; ++n) {
        /* Straight-line body: no loop between the read and the reload. */
        unsigned prev = acc.u[0];
        unsigned h = n * 2654435761u;
        src[0] = 0x40000000u ^ (h + 0u);
        src[1] = 0x40000000u ^ (h + 1u);
        src[2] = 0x40000000u ^ (h + 2u);
        src[3] = 0x40000000u ^ (h + 3u);
        src[4] = 0x40000000u ^ (h + 4u);
        src[5] = 0x40000000u ^ (h + 5u);
        src[6] = 0x40000000u ^ (h + 6u);
        src[7] = 0x40000000u ^ (h + 7u);
        acc.v = vec_from(src);
        acc.u[0] = prev * 3u + n;
        union b8 out;
        out.v = acc.v;
        sum += out.u[0] ^ out.u[7];

        unsigned rprev = rv[0];
        memcpy(rv, src, sizeof rv);
        rv[0] = rprev * 3u + n;
        refsum += rv[0] ^ rv[7];
    }
    CHECK(sum == refsum, "loop_carried_read checksum %llu want %llu", sum, refsum);
    return sum;
}

/* 64-bit reads of lane pairs after lane writes: q[0] is lanes 0-1, q[1] lanes 2-3. */
static void case_q_span(void) {
    unsigned w[8];
    for (int i = 0; i < 8; ++i)
        w[i] = 0x3f000000u + (unsigned)i;
    union b8 p;
    p.v = vec_from(w);
    p.u[1] = 0x11111111u;
    p.u[2] = 0x22222222u;
    union b8 r;
    r.v = p.v;
    unsigned long long a = p.q[0];
    unsigned long long b = p.q[1];
    unsigned long long wa = (unsigned long long)w[0] | ((unsigned long long)0x11111111u << 32);
    unsigned long long wb = (unsigned long long)0x22222222u | ((unsigned long long)w[3] << 32);
    CHECK(a == wa, "q_span lanes 0-1: got %016llx want %016llx", a, wa);
    CHECK(b == wb, "q_span lanes 2-3: got %016llx want %016llx", b, wb);
    CHECK(r.u[1] == 0x11111111u && r.u[2] == 0x22222222u, "q_span reload lanes 1-2");
}

/* Byte read inside a lane written earlier in the block (lane 3, byte 1). */
static void case_bytes(void) {
    unsigned w[8];
    for (int i = 0; i < 8; ++i)
        w[i] = 0x50000000u + (unsigned)i;
    union b8 p;
    p.v = vec_from(w);
    p.u[3] = 0x04030201u;
    unsigned char c13 = p.c[13];
    unsigned char c12 = p.c[12];
    union b8 r;
    r.v = p.v;
    CHECK(c13 == 0x02 && c12 == 0x01, "bytes: c12=%02x c13=%02x", c12, c13);
    CHECK(r.u[3] == 0x04030201u, "bytes reload lane 3 %08x", r.u[3]);
}

/* Signalling NaN, negative quiet NaN with payload, negative zero, denormal. */
static void case_nan_payloads(void) {
    unsigned w[8] = {0, 0, 0, 0, 0, 0, 0, 0};
    unsigned pat[4] = {0x7fa00001u, 0xffc0beefu, 0x80000000u, 0x00000001u};
    union b8 p;
    p.v = vec_from(w);
    p.u[0] = pat[0];
    p.u[2] = pat[1];
    p.u[4] = pat[2];
    p.u[6] = pat[3];
    union b8 r;
    r.v = p.v;
    CHECK(r.u[0] == pat[0] && r.u[2] == pat[1] && r.u[4] == pat[2] && r.u[6] == pat[3],
          "nan payloads: %08x %08x %08x %08x", r.u[0], r.u[2], r.u[4], r.u[6]);
    CHECK(r.u[1] == 0 && r.u[3] == 0 && r.u[5] == 0 && r.u[7] == 0, "nan payloads: untouched lanes");
}

static __attribute__((noinline)) void poke(unsigned *p, unsigned v) { *p = v; }

/* The address of a lane escapes into a call that writes it. */
static void case_escape(void) {
    unsigned w[8];
    for (int i = 0; i < 8; ++i)
        w[i] = 0x60000000u + (unsigned)i;
    union b8 p;
    p.v = vec_from(w);
    p.u[2] = 0x1u;
    poke(&p.u[2], 0xdeadu);
    union b8 r;
    r.v = p.v;
    CHECK(r.u[2] == 0xdeadu, "escape lane 2: got %08x", r.u[2]);
    for (int i = 0; i < 8; ++i)
        if (i != 2)
            CHECK(r.u[i] == w[i], "escape untouched lane %d", i);
}

/* Lane 5 is written in one block; the reload sits in a block reached on both
 * paths.  The write on the taken path must be visible. */
static void case_cross_block(unsigned mode) {
    unsigned w[8];
    for (int i = 0; i < 8; ++i)
        w[i] = 0x70000000u + (unsigned)i;
    union b8 p;
    p.v = vec_from(w);
    p.u[5] = 0x55555555u;
    if (mode & 1u)
        p.u[6] = 0x66666666u;
    union b8 r;
    r.v = p.v;
    CHECK(r.u[5] == 0x55555555u, "cross_block mode %u lane 5", mode);
    CHECK(r.u[6] == ((mode & 1u) ? 0x66666666u : w[6]), "cross_block mode %u lane 6", mode);
}

/* A second whole definition shadows the first; the reload taken before it must
 * still see the first chain. */
static void case_redefine(void) {
    unsigned wa[8], wb[8];
    for (int i = 0; i < 8; ++i) {
        wa[i] = 0x80000000u + (unsigned)i;
        wb[i] = 0x90000000u + (unsigned)i;
    }
    union b8 p;
    p.v = vec_from(wa);
    p.u[0] = 0xaaaa0000u;
    union b8 r1;
    r1.v = p.v;
    p.v = vec_from(wb);
    p.u[5] = 0xbbbb0000u;
    union b8 r2;
    r2.v = p.v;
    CHECK(r1.u[0] == 0xaaaa0000u && r1.u[5] == wa[5], "redefine first reload");
    CHECK(r2.u[0] == wb[0] && r2.u[5] == 0xbbbb0000u, "redefine second reload");
}

/* A scalar read observes the first write to lane 4; the second write must not
 * be mistaken for the first, and the first must not be dropped. */
static void case_observed_then_write(void) {
    unsigned w[8];
    for (int i = 0; i < 8; ++i)
        w[i] = 0x12340000u + (unsigned)i;
    union b8 p;
    p.v = vec_from(w);
    p.u[4] = 0xaaaaaaaau;
    unsigned seen = p.u[4];
    p.u[4] = 0xbbbbbbbbu;
    union b8 r;
    r.v = p.v;
    CHECK(seen == 0xaaaaaaaau, "observed_then_write read %08x", seen);
    CHECK(r.u[4] == 0xbbbbbbbbu, "observed_then_write reload %08x", r.u[4]);
}

/* No definition in the loop body: the lane write and the reload share a block
 * but the define is outside it, so the rewrite must refuse and the loop must
 * still accumulate through memory. */
static unsigned long long case_loop_without_define(void) {
    unsigned w[8];
    for (int i = 0; i < 8; ++i)
        w[i] = 0x11000000u + (unsigned)i;
    union b8 p;
    p.v = vec_from(w);
    unsigned long long sum = 0;
    unsigned ref = w[1];
    for (unsigned n = 0; n < 64; ++n) {
        unsigned old = p.u[1];
        p.u[1] = old + n;
        union b8 r;
        r.v = p.v;
        sum += r.u[1];
        ref = ref + n;
    }
    CHECK(sum != 0 && p.u[1] == ref, "loop_without_define final %08x want %08x", p.u[1], ref);
    return sum;
}

int main(void) {
    unsigned long long loop = case_loop_carried_read();
    case_q_span();
    case_bytes();
    case_nan_payloads();
    case_escape();
    for (unsigned mode = 0; mode < 2; ++mode)
        case_cross_block(mode);
    case_redefine();
    case_observed_then_write();
    unsigned long long noloop = case_loop_without_define();
    printf("avx_union_lane_redteam loop=%llu noloop=%llu failures=%d\n", loop, noloop, failures);
    return failures == 0 ? 0 : 1;
}
