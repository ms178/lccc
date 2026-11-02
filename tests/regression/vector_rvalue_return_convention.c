/*
 * VECRET-1: a vector returned BY VALUE across a real call boundary.
 *
 * Retired from artifacts/open_bugs/ on 2026-10-22. The reported symptom was
 * that `passret` -- a 16-byte vector returned from a function that COMPUTES it
 * rather than passing a parameter through -- read the result alloca's POINTER
 * as the value (amd64: lanes {ptr_lo, ptr_hi, 0, 0}, i.e. `si={ptr_lo,
 * ptr_hi}`), while the identity return stayed correct.
 *
 * That is a call-site/callee convention mismatch, not a value bug: one side
 * believed the result arrives in %xmm0 and the other believed %rax holds the
 * address of a hidden buffer. It does not reproduce on this tree, nor on any
 * build from the report date onward (verified: -O0/-O1/-O2/-O3, SSE4.2/AVX/
 * AVX2/AVX-512, PIE/no-PIE/static, -g, CCC_VERIFY_IR=1, and against GCC 16.2
 * in both cross-link directions). The artifact was also never compiled by CI:
 * nothing references artifacts/open_bugs/, so the shape had no coverage at
 * all. This file is that coverage.
 *
 * What it pins, all through `noinline` boundaries so the psABI is really
 * exercised at -O2 (the reported level):
 *
 *   - the identity return and the computed return side by side, which is the
 *     distinction the report turns on;
 *   - every 16-byte lane type, plus the packed <= 8-byte shapes and the one-
 *     lane FP `vector_size` shapes that the SysV classifier routes to MEMORY
 *     (hidden pointer) -- the family where "the pointer, not the pointee"
 *     actually lives;
 *   - runtime, constant and two-operand shuffle masks, arithmetic rvalues,
 *     conditional returns, composite literals and a memory round trip;
 *   - consumers beyond lane printing: direct argument passing, array and
 *     global stores, a struct-wrapped result and a call through a function
 *     pointer.
 *
 * Every value is checked twice: against an independent scalar oracle computed
 * here (so the test fails on its own, without an external compiler) and
 * against the corpus runner's GCC comparison of the printed lanes.
 *
 * exit 0 = pass (the runner's convention for tests/regression/*.c).
 */
#include <stdio.h>
#include <stdint.h>
#include <string.h>

typedef int8_t  v16i8 __attribute__((vector_size(16)));
typedef int16_t v8i16 __attribute__((vector_size(16)));
typedef int32_t v4i32 __attribute__((vector_size(16)));
typedef int64_t v2i64 __attribute__((vector_size(16)));
typedef float   v4f32 __attribute__((vector_size(16)));
typedef double  v2f64 __attribute__((vector_size(16)));
typedef int32_t v2i32 __attribute__((vector_size(8)));
typedef int32_t v1si  __attribute__((vector_size(4)));
typedef float   v1sf  __attribute__((vector_size(4)));
typedef double  v1df  __attribute__((vector_size(8)));

typedef struct { v4i32 v; } wrapped;

static int failures;

/* Independent scalar oracle for GCC's shuffle convention:
 * result[i] = concat(a, b)[mask[i] mod 2N], N = lanes(a) -- the mask indexes
 * the FULL concatenation, so a mask of 5 on a 4-lane vector selects b[1]
 * (measured against GCC 16.2.0). The two-argument form passes a twice.
 * Computed here with plain integer arrays, so it shares no code with the
 * lowering under test. */
static void oracle_shuffle(const int32_t *a, const int32_t *b, const int32_t *mask,
                           int32_t *out, int lanes) {
    unsigned span = 2u * (unsigned)lanes;
    for (int i = 0; i < lanes; i++) {
        unsigned idx = (unsigned)mask[i] % span;
        out[i] = idx < (unsigned)lanes ? a[idx] : b[idx - (unsigned)lanes];
    }
}

#define CHECK(what, got, want)                                                       \
    do {                                                                             \
        long long g_ = (long long)(got), w_ = (long long)(want);                      \
        if (g_ != w_) {                                                              \
            printf("FAIL %s: got %lld want %lld\n", (what), g_, w_);                  \
            failures++;                                                              \
        }                                                                            \
    } while (0)

/* ── the reported pair: identity return vs computed return ─────────────── */
static __attribute__((noinline)) v4i32 idret(v4i32 a) { return a; }
static __attribute__((noinline)) v4i32 passret(v4i32 a, v4i32 m) {
    return __builtin_shuffle(a, m); /* runtime mask: the reported shape */
}

/* ── computed rvalues, one per 16-byte lane type ───────────────────────── */
static __attribute__((noinline)) v4i32 add_4i32(v4i32 a, v4i32 b) { return a + b; }
static __attribute__((noinline)) v2i64 add_2i64(v2i64 a, v2i64 b) { return a + b; }
static __attribute__((noinline)) v8i16 add_8i16(v8i16 a, v8i16 b) { return a + b; }
static __attribute__((noinline)) v16i8 add_16i8(v16i8 a, v16i8 b) { return a + b; }
static __attribute__((noinline)) v4f32 mul_4f32(v4f32 a, v4f32 b) { return a * b; }
static __attribute__((noinline)) v2f64 add_2f64(v2f64 a, v2f64 b) { return a + b; }

/* Constant and two-operand masks. */
static __attribute__((noinline)) v4i32 rev_4i32(v4i32 a) {
    v4i32 k = {3, 2, 1, 0};
    return __builtin_shuffle(a, k);
}
static __attribute__((noinline)) v4i32 pick_4i32(v4i32 a, v4i32 b) {
    v4i32 k = {0, 5, 2, 7}; /* 0,2 from a; 1,3 from b */
    return __builtin_shuffle(a, b, k);
}

/* Remaining rvalue shapes. */
static __attribute__((noinline)) v4i32 cond_4i32(v4i32 a, v4i32 b, int c) {
    v4i32 k = {1, 1, 1, 1};
    return c ? __builtin_shuffle(a, k) : __builtin_shuffle(b, k);
}
static __attribute__((noinline)) v4i32 lit_4i32(void) { return (v4i32){9, 8, 7, 6}; }
static __attribute__((noinline)) v4i32 memrt_4i32(v4i32 a, v4i32 m) {
    v4i32 t;
    memcpy(&t, &a, sizeof t);
    t = __builtin_shuffle(t, m);
    v4i32 u;
    memcpy(&u, &t, sizeof u);
    return u;
}
static __attribute__((noinline)) wrapped wrap_4i32(v4i32 a) {
    wrapped w;
    w.v = __builtin_shuffle(a, a);
    return w;
}

/* ── packed <= 8-byte and one-lane FP (MEMORY / hidden-pointer) shapes ──── */
static __attribute__((noinline)) v2i32 add_2i32(v2i32 a, v2i32 b) { return a + b; }
static __attribute__((noinline)) v1si add_1si(v1si a) { return a * a; }
static __attribute__((noinline)) v1sf mul_1sf(v1sf a) { return a * 2.0f; }
static __attribute__((noinline)) v1df add_1df(v1df a) { return a + 1.0; }

/* ── consumers other than lane printing ────────────────────────────────── */
static __attribute__((noinline)) int sum_4i32(v4i32 a) {
    return a[0] + a[1] + a[2] + a[3];
}
static __attribute__((noinline)) void sink_4i32(v4i32 a, int tag) {
    int32_t lanes[4];
    memcpy(lanes, &a, sizeof lanes);
    printf("sink%d %d %d %d %d\n", tag, lanes[0], lanes[1], lanes[2], lanes[3]);
}

static v4i32 g_vec;
static v4i32 g_array[2];

int main(void) {
    const v4i32 a = {1, 2, 3, 4};
    const v4i32 b = {10, 20, 30, 40};
    const int32_t a_l[4] = {1, 2, 3, 4};
    const int32_t b_l[4] = {10, 20, 30, 40};

    /* runtime mask, lane-dependent (3,2,1,0) permuted so a transpose-like
     * mask would also show up */
    const int32_t mask_l[4] = {3, 0, 2, 1};
    const v4i32 m = {3, 0, 2, 1};
    int32_t want[4];

    /* 1. the reported pair ------------------------------------------------ */
    v4i32 r_id = idret(a);
    for (int i = 0; i < 4; i++) CHECK("idret lane", r_id[i], a_l[i]);
    printf("idret   %d %d %d %d\n", r_id[0], r_id[1], r_id[2], r_id[3]);

    v4i32 r_pass = passret(a, m);
    oracle_shuffle(a_l, a_l, mask_l, want, 4);
    for (int i = 0; i < 4; i++) CHECK("passret lane", r_pass[i], want[i]);
    printf("passret %d %d %d %d\n", r_pass[0], r_pass[1], r_pass[2], r_pass[3]);

    /* A returned vector is never an address: the reported corruption put a
     * stack pointer in lanes 0..1. Checked directly so the failure message
     * names the defect instead of just a lane mismatch. */
    CHECK("passret is not a stack address (lo)", r_pass[0] != (int32_t)(uintptr_t)&r_pass, 1);
    CHECK("passret is not a stack address (hi)", r_pass[1] != (int32_t)((uintptr_t)&r_pass >> 32), 1);

    /* 2. arithmetic rvalues, every 16-byte lane type ---------------------- */
    v4i32 r_add = add_4i32(a, b);
    for (int i = 0; i < 4; i++) CHECK("add_4i32", r_add[i], a_l[i] + b_l[i]);
    printf("add_4i32 %d %d %d %d\n", r_add[0], r_add[1], r_add[2], r_add[3]);

    {
        v2i64 x = {1, 2}, y = {10, 20};
        v2i64 r = add_2i64(x, y);
        CHECK("add_2i64[0]", r[0], 11);
        CHECK("add_2i64[1]", r[1], 22);
        printf("add_2i64 %lld %lld\n", (long long)r[0], (long long)r[1]);
    }
    {
        v8i16 x = {1, 2, 3, 4, 5, 6, 7, 8}, y = {10, 20, 30, 40, 50, 60, 70, 80};
        v8i16 r = add_8i16(x, y);
        for (int i = 0; i < 8; i++) CHECK("add_8i16", r[i], (i + 1) + 10 * (i + 1));
        printf("add_8i16 %d %d %d %d %d %d %d %d\n", r[0], r[1], r[2], r[3], r[4], r[5], r[6], r[7]);
    }
    {
        v16i8 x, y;
        for (int i = 0; i < 16; i++) { x[i] = (int8_t)(i + 1); y[i] = (int8_t)(i * 2); }
        v16i8 r = add_16i8(x, y);
        for (int i = 0; i < 16; i++) CHECK("add_16i8", r[i], (i + 1) + i * 2);
        printf("add_16i8 %d %d %d %d %d %d %d %d %d %d %d %d %d %d %d %d\n",
               r[0], r[1], r[2], r[3], r[4], r[5], r[6], r[7],
               r[8], r[9], r[10], r[11], r[12], r[13], r[14], r[15]);
    }
    {
        v4f32 x = {1.0f, 2.0f, 3.0f, 4.0f}, y = {2.0f, 3.0f, 4.0f, 5.0f};
        v4f32 r = mul_4f32(x, y);
        /* integer images compare exactly, and print without FP formatting */
        uint32_t got[4];
        memcpy(got, &r, sizeof got);
        for (int i = 0; i < 4; i++) {
            float want_f = (float)(i + 1) * (float)(i + 2);
            uint32_t want_bits;
            memcpy(&want_bits, &want_f, sizeof want_bits);
            CHECK("mul_4f32 lane bits", got[i], want_bits);
        }
        printf("mul_4f32 %u %u %u %u\n", got[0], got[1], got[2], got[3]);
    }
    {
        v2f64 x = {1.5, 2.5}, y = {3.25, 4.25};
        v2f64 r = add_2f64(x, y);
        uint64_t got[2];
        memcpy(got, &r, sizeof got);
        double w0 = 4.75, w1 = 6.75;
        uint64_t wb[2];
        memcpy(wb, &w0, 8);
        memcpy(wb + 1, &w1, 8);
        CHECK("add_2f64[0] bits", got[0], wb[0]);
        CHECK("add_2f64[1] bits", got[1], wb[1]);
        printf("add_2f64 %llu %llu\n", (unsigned long long)got[0], (unsigned long long)got[1]);
    }

    /* 3. constant and two-operand masks ----------------------------------- */
    v4i32 r_rev = rev_4i32(a);
    for (int i = 0; i < 4; i++) CHECK("rev_4i32", r_rev[i], a_l[3 - i]);
    printf("rev_4i32 %d %d %d %d\n", r_rev[0], r_rev[1], r_rev[2], r_rev[3]);

    v4i32 r_pick = pick_4i32(a, b);
    {
        const int32_t k_l[4] = {0, 5, 2, 7};
        oracle_shuffle(a_l, b_l, k_l, want, 4);
        for (int i = 0; i < 4; i++) CHECK("pick_4i32", r_pick[i], want[i]);
    }
    printf("pick_4i32 %d %d %d %d\n", r_pick[0], r_pick[1], r_pick[2], r_pick[3]);

    /* 4. remaining rvalue shapes ------------------------------------------ */
    v4i32 r_cond = cond_4i32(a, b, 1);
    for (int i = 0; i < 4; i++) CHECK("cond_4i32", r_cond[i], a_l[1]);
    printf("cond_4i32 %d %d %d %d\n", r_cond[0], r_cond[1], r_cond[2], r_cond[3]);

    v4i32 r_lit = lit_4i32();
    for (int i = 0; i < 4; i++) CHECK("lit_4i32", r_lit[i], 9 - i);
    printf("lit_4i32 %d %d %d %d\n", r_lit[0], r_lit[1], r_lit[2], r_lit[3]);

    v4i32 r_mem = memrt_4i32(a, m);
    oracle_shuffle(a_l, a_l, mask_l, want, 4);
    for (int i = 0; i < 4; i++) CHECK("memrt_4i32", r_mem[i], want[i]);
    printf("memrt_4i32 %d %d %d %d\n", r_mem[0], r_mem[1], r_mem[2], r_mem[3]);

    wrapped w = wrap_4i32(a);
    /* `__builtin_shuffle(a, a)` uses the second `a` as the MASK, so the lanes
     * are permuted by a's own values, not an identity. */
    oracle_shuffle(a_l, a_l, a_l, want, 4);
    for (int i = 0; i < 4; i++) CHECK("wrap_4i32", w.v[i], want[i]);
    printf("wrap_4i32 %d %d %d %d\n", w.v[0], w.v[1], w.v[2], w.v[3]);

    /* 5. packed <= 8-byte and one-lane FP (MEMORY / hidden pointer) ------- */
    {
        v2i32 x = {10, 20}, y = {1, 2};
        v2i32 r = add_2i32(x, y);
        CHECK("add_2i32[0]", r[0], 11);
        CHECK("add_2i32[1]", r[1], 22);
        printf("add_2i32 %d %d\n", r[0], r[1]);
    }
    {
        v1si x = {7};
        v1si r = add_1si(x);
        CHECK("add_1si", r[0], 49);
        printf("add_1si %d\n", r[0]);
    }
    {
        v1sf x = {2.5f};
        v1sf r = mul_1sf(x);
        CHECK("mul_1sf", r[0], 5.0f);
        printf("mul_1sf %.3f\n", (double)r[0]);
    }
    {
        v1df x = {3.25};
        v1df r = add_1df(x);
        CHECK("add_1df", r[0], 4.25);
        printf("add_1df %.3f\n", r[0]);
    }

    /* 6. consumers other than lane printing ------------------------------- */
    CHECK("direct argument", sum_4i32(idret(a)), 10);
    sink_4i32(idret(b), 7);

    g_array[1] = passret(a, m);
    oracle_shuffle(a_l, a_l, mask_l, want, 4);
    for (int i = 0; i < 4; i++) CHECK("array store", g_array[1][i], want[i]);
    printf("array   %d %d %d %d\n", g_array[1][0], g_array[1][1], g_array[1][2], g_array[1][3]);

    g_vec = add_4i32(a, b);
    for (int i = 0; i < 4; i++) CHECK("global store", g_vec[i], a_l[i] + b_l[i]);
    printf("global  %d %d %d %d\n", g_vec[0], g_vec[1], g_vec[2], g_vec[3]);

    {
        /* an indirect call reaches the same convention through a pointer */
        v4i32 (*fp)(v4i32, v4i32) = passret;
        v4i32 r = fp(a, m);
        oracle_shuffle(a_l, a_l, mask_l, want, 4);
        for (int i = 0; i < 4; i++) CHECK("indirect call", r[i], want[i]);
        printf("indirect %d %d %d %d\n", r[0], r[1], r[2], r[3]);
    }

    if (failures) {
        printf("FAILURES: %d\n", failures);
        return 1;
    }
    printf("OK vector_rvalue_return_convention\n");
    return 0;
}
