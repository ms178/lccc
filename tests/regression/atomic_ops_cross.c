/* Atomic operations on AArch64 and RISC-V (check_switch_cross_execute.py).
 *
 * Every check compares one result with the value GCC 16.2 computes for the
 * same C on x86-64 (`-DGEN -latomic` prints the table below).  The program is
 * single-threaded, so each operation has its sequential meaning; what the
 * checks pin is the lowering: the width of every load-exclusive/AMO, the
 * extension of the returned old value (an `i8` -1 from `ldaxrb`/`lr.w` must
 * come back sign-extended, a `u32` 0xffffffff zero-extended once widened),
 * the expected value of a compare-exchange compared at the object's width and
 * written back on failure, and 128-bit operations that touch both halves
 * (the carry out of the low doubleword, a high half that differs alone).
 *
 * The pointer operand of every atomic is a register-homed value on both
 * targets: `loop_counter` keeps it in a register across a loop, and the
 * `pressure_*` functions keep a dozen values live across an atomic so a
 * register the lowering uses as scratch cannot hold one of them.
 *
 * RISC-V has no 128-bit atomic instruction; like GCC and Clang it calls
 * libatomic's `__atomic_*_16`, which this freestanding test supplies below
 * (sequentially: there is one thread).  main returns the 1-based index of the
 * first failing group of checks (`g__`), 255 for a group count mismatch.
 */
#ifdef GEN
#include <stdio.h>
#endif

#define NI __attribute__((noinline))

typedef signed char i8;
typedef unsigned char u8;
typedef short i16;
typedef unsigned short u16;
typedef int i32;
typedef unsigned u32;
typedef long long i64;
typedef unsigned long long u64;
typedef __int128 i128;
typedef unsigned __int128 u128;

/* Wrapping arithmetic for the driver: every sum/difference is formed in
 * u128 and converted back (modulo, as GCC, Clang and LCCC define it), so no
 * signed operation can overflow. */
#define W(e) ((u128)(e))

#define SC __ATOMIC_SEQ_CST
#define ACQ __ATOMIC_ACQUIRE
#define REL __ATOMIC_RELEASE
#define AR __ATOMIC_ACQ_REL
#define RLX __ATOMIC_RELAXED

/* ------------------------------------------------- per-type operations */
#define OPS(T)                                                                   \
    NI T ld_##T(T *p) { return __atomic_load_n(p, SC); }                        \
    NI T ld_acq_##T(T *p) { return __atomic_load_n(p, ACQ); }                   \
    NI T ld_rlx_##T(T *p) { return __atomic_load_n(p, RLX); }                   \
    NI void st_##T(T *p, T v) { __atomic_store_n(p, v, SC); }                   \
    NI void st_rel_##T(T *p, T v) { __atomic_store_n(p, v, REL); }              \
    NI void st_rlx_##T(T *p, T v) { __atomic_store_n(p, v, RLX); }              \
    NI T xchg_##T(T *p, T v) { return __atomic_exchange_n(p, v, AR); }          \
    NI T fadd_##T(T *p, T v) { return __atomic_fetch_add(p, v, SC); }           \
    NI T fsub_##T(T *p, T v) { return __atomic_fetch_sub(p, v, RLX); }          \
    NI T fand_##T(T *p, T v) { return __atomic_fetch_and(p, v, ACQ); }          \
    NI T for_##T(T *p, T v) { return __atomic_fetch_or(p, v, REL); }            \
    NI T fxor_##T(T *p, T v) { return __atomic_fetch_xor(p, v, SC); }           \
    NI T fnand_##T(T *p, T v) { return __atomic_fetch_nand(p, v, AR); }         \
    NI T addf_##T(T *p, T v) { return __atomic_add_fetch(p, v, SC); }           \
    NI T subf_##T(T *p, T v) { return __atomic_sub_fetch(p, v, ACQ); }          \
    NI T nandf_##T(T *p, T v) { return __atomic_nand_fetch(p, v, SC); }         \
    NI int cas_##T(T *p, T *e, T d)                                             \
    {                                                                            \
        return __atomic_compare_exchange_n(p, e, d, 0, SC, RLX);                \
    }                                                                            \
    NI int cas_rel_##T(T *p, T *e, T d)                                         \
    {                                                                            \
        return __atomic_compare_exchange_n(p, e, d, 0, REL, RLX);               \
    }                                                                            \
    /* A weak exchange may fail spuriously; retry while the expected value  */  \
    /* is still current, as a caller's loop does.                           */  \
    NI int casw_##T(T *p, T *e, T d)                                            \
    {                                                                            \
        T want = *e;                                                             \
        for (;;) {                                                               \
            if (__atomic_compare_exchange_n(p, e, d, 1, AR, ACQ))               \
                return 1;                                                        \
            if (*e != want)                                                      \
                return 0;                                                        \
        }                                                                        \
    }

#define SYNC_OPS(T)                                                              \
    NI T vcas_##T(T *p, T e, T d) { return __sync_val_compare_and_swap(p, e, d); } \
    NI int bcas_##T(T *p, T e, T d) { return __sync_bool_compare_and_swap(p, e, d); } \
    NI T tas_##T(T *p, T v) { return __sync_lock_test_and_set(p, v); }          \
    NI void rel_##T(T *p) { __sync_lock_release(p); }                           \
    NI T sadd_##T(T *p, T v) { return __sync_fetch_and_add(p, v); }             \
    NI T ssubf_##T(T *p, T v) { return __sync_sub_and_fetch(p, v); }

OPS(i8)
OPS(u8)
OPS(i16)
OPS(u16)
OPS(i32)
OPS(u32)
OPS(i64)
OPS(u64)
OPS(i128)
OPS(u128)
SYNC_OPS(i8)
SYNC_OPS(u8)
SYNC_OPS(i16)
SYNC_OPS(u16)
SYNC_OPS(i32)
SYNC_OPS(u32)
SYNC_OPS(i64)
SYNC_OPS(u64)

/* A 32-bit old value widened at the call site: the zero extension of the
 * `lr.w`/`ldaxr w` result is the only thing between 0xffffffff and -1. */
NI u64 vcas_u32_wide(u32 *p, u32 e, u32 d) { return __sync_val_compare_and_swap(p, e, d); }
NI i64 fadd_i32_wide(i32 *p, i32 v) { return __atomic_fetch_add(p, v, SC); }
NI u64 ld_u32_wide(u32 *p) { return __atomic_load_n(p, SC); }

/* --------------------------------------------- pointer and pressure tests */
NI u64 loop_counter(u64 *p, int n)
{
    for (int i = 0; i < n; i++)
        __atomic_fetch_add(p, (u64)i * 3 + 1, RLX);
    return __atomic_load_n(p, ACQ);
}

NI u32 loop_cas_max(u32 *p, const u32 *v, int n)
{
    for (int i = 0; i < n; i++) {
        u32 cur = __atomic_load_n(p, RLX);
        while (v[i] > cur && !__atomic_compare_exchange_n(p, &cur, v[i], 1, SC, RLX))
            ;
    }
    return __atomic_load_n(p, SC);
}

struct counters {
    u8 flag;
    i16 h;
    u32 w[4];
    i64 d[3];
};

NI i64 field_ops(struct counters *s, int i)
{
    u64 r = __atomic_fetch_add(&s->w[i & 3], 7u, SC);
    r += (u64)__atomic_fetch_sub(&s->d[i % 3], 11, AR);
    r += (u64)__atomic_exchange_n(&s->h, (i16)(i * 1000 - 3000), SC);
    r += (u64)__atomic_test_and_set(&s->flag, SC);
    return (i64)r;
}

#define PRESSURE(T)                                                              \
    NI i64 pressure_##T(T *p, u64 a, u64 b, u64 c, u64 d, u64 e, u64 f)         \
    {                                                                            \
        u64 t0 = a * 3, t1 = b * 5, t2 = c * 7, t3 = d * 11, t4 = e * 13;       \
        u64 t5 = f * 17, t6 = a ^ b, t7 = c ^ d, t8 = e ^ f, t9 = a + f;        \
        u64 t10 = b - e, t11 = c * d;                                           \
        T old = __atomic_fetch_add(p, (T)t0, SC);                               \
        T prev = __atomic_exchange_n(p, (T)(W(old) + 1), AR);                   \
        T exp = (T)(prev ^ 1);                                                  \
        int ok = __atomic_compare_exchange_n(p, &exp, (T)t1, 0, SC, SC);        \
        return (i64)((u64)old + (u64)prev + (u64)exp + (u64)ok + t0 + 2 * t1 +  \
                     3 * t2 + 4 * t3 + 5 * t4 + 6 * t5 + 7 * t6 + 8 * t7 +      \
                     9 * t8 + 10 * t9 + 11 * t10 + 12 * t11);                   \
    }
PRESSURE(i8)
PRESSURE(u16)
PRESSURE(i32)
PRESSURE(u64)

/* Floating-point objects through the generic builtins: the access width is
 * the object's (a 4-byte `float` used to be exchanged as 8 bytes on AArch64
 * and RISC-V, clobbering its neighbour, and as `xchgq %eax` on x86-64). */
struct fpair {
    float a, b;
    double c;
};
NI float xchg_f32(float *p, float v)
{
    float o;
    __atomic_exchange(p, &v, &o, SC);
    return o;
}
NI float load_f32(float *p)
{
    float o;
    __atomic_load(p, &o, ACQ);
    return o;
}
NI void store_f32(float *p, float v) { __atomic_store(p, &v, REL); }
NI double xchg_f64(double *p, double v)
{
    double o;
    __atomic_exchange(p, &v, &o, AR);
    return o;
}
NI int cas_f32(float *p, float *e, float d) { return __atomic_compare_exchange(p, e, &d, 0, SC, ACQ); }
static u32 f32_bits(float f)
{
    union {
        float f;
        u32 u;
    } x = { f };
    return x.u;
}
static u64 f64_bits(double d)
{
    union {
        double d;
        u64 u;
    } x = { d };
    return x.u;
}

/* `_Atomic` objects: compound assignment is a CAS loop or a fetch-op. */
_Atomic int ai = 5;
_Atomic double ad = 1.25;
_Atomic u8 ab = 250;
NI int atomic_compound(int k)
{
    ai += k;
    ai *= 3;
    ab += (u8)k;
    ad += 0.5 * k;
    return ai + ab + (int)(ad * 4);
}

#ifdef __riscv
/* libatomic's sized 16-byte entry points (one thread: plain accesses). */
u128 __atomic_load_16(const volatile void *p, int m)
{
    (void)m;
    return *(const volatile u128 *)p;
}
void __atomic_store_16(volatile void *p, u128 v, int m)
{
    (void)m;
    *(volatile u128 *)p = v;
}
u128 __atomic_exchange_16(volatile void *p, u128 v, int m)
{
    (void)m;
    u128 o = *(volatile u128 *)p;
    *(volatile u128 *)p = v;
    return o;
}
_Bool __atomic_compare_exchange_16(volatile void *p, void *e, u128 d, int s, int f)
{
    (void)s;
    (void)f;
    u128 o = *(volatile u128 *)p;
    if (o == *(u128 *)e) {
        *(volatile u128 *)p = d;
        return 1;
    }
    *(u128 *)e = o;
    return 0;
}
#define FETCH16(name, expr)                                                      \
    u128 __atomic_fetch_##name##_16(volatile void *p, u128 v, int m)            \
    {                                                                            \
        (void)m;                                                                 \
        u128 o = *(volatile u128 *)p;                                            \
        *(volatile u128 *)p = (expr);                                            \
        return o;                                                                \
    }                                                                            \
    u128 __atomic_##name##_fetch_16(volatile void *p, u128 v, int m)            \
    {                                                                            \
        (void)m;                                                                 \
        u128 o = *(volatile u128 *)p;                                            \
        u128 n = (expr);                                                         \
        *(volatile u128 *)p = n;                                                 \
        return n;                                                                \
    }
FETCH16(add, o + v)
FETCH16(sub, o - v)
FETCH16(and, o & v)
FETCH16(or, o | v)
FETCH16(xor, o ^ v)
FETCH16(nand, ~(o & v))
#endif

/* ------------------------------------------------------------- checks */
/* Every result is folded into an FNV-1a hash; GROUP() closes a group of
 * checks and compares its hash with the table (GCC 16.2, x86-64, `-DGEN
 * -latomic` prints it).  `-DGEN -DTRACE` prints each value instead, to find
 * the failing check of a group. */
static unsigned long long h__ = 1469598103934665603ull;
static int g__;
#define CHECK(e) (h__ = (h__ ^ (unsigned long long)(long long)(e)) * 1099511628211ull, \
                  TRACE_VALUE(e))
#if defined(GEN) && defined(TRACE)
#define TRACE_VALUE(e) printf("%d: %lld\n", g__, (long long)(e))
#else
#define TRACE_VALUE(e) ((void)0)
#endif
#ifdef GEN
#define GROUP()                                                                  \
    do {                                                                         \
        printf("    0x%016llxull,\n", h__);                                     \
        g__++;                                                                   \
    } while (0)
#else
static const unsigned long long expected[] = {
    0xe7b9335f2960294dull,
    0x2d44f5feaea0024bull,
    0x6e877fec06764525ull,
    0x16005e93727edd13ull,
    0xa16d4f8e7fb0719dull,
    0xbc688672534aab1bull,
    0x04bd1a0ea5c474ecull,
    0xe6da9199f93ade97ull,
    0xf970ae87b34f2455ull,
    0xb78311b3d38e9b53ull,
    0x36a543751ef1bb11ull,
    0xc9f05240eb5b7217ull,
    0xa94e5e1fcab87a0aull,
    0x43baf8de8898e147ull,
    0x04a3f46ac44098baull,
    0x6eaf672aa25b9af7ull,
    0x57acd0ac669c77abull,
    0x5341cde86b7b58cbull,
    0x098009b8a10a3f9dull,
    0x53aa29f5aa56f92aull,
    0xe0002bae5ebc5568ull,
    0x45fde5f81b73de4cull,
    0x9fa6237b34e78796ull,
    0xa36b8f465468e6e5ull,
    0xbc25d5f808d3da1dull,
    0xb70ae64f6d364e4bull,
    0x14176c98880b5dd9ull,
    0xee8e6c7569534e0bull,
    0x0ca3ea20585aed03ull,
    0x46d10da40ae3dc7bull,
    0x822730e59856fc90ull,
    0xa90256fd93e483f8ull,
    0xb82bfdf4e28cc9b9ull,
    0x680345f2a1cd720bull,
    0xdee223b0ec064e5eull,
    0x22e1a1d51302a228ull,
    0x9d493dae24cb8491ull,
    0x72c23be666a2cf8full,
};
#define GROUP()                                                                  \
    do {                                                                         \
        if (g__ >= (int)(sizeof expected / sizeof expected[0]) || h__ != expected[g__]) \
            return g__ + 1;                                                      \
        g__++;                                                                   \
    } while (0)
#endif
#define CHECK128(e)                                  \
    do {                                             \
        u128 v128__ = (u128)(e);                     \
        CHECK((u64)v128__);                          \
        CHECK((u64)(v128__ >> 64));                  \
    } while (0)

static const u64 seeds[] = {
    0, 1, 0x7f, 0x80, 0xff, 0x7fff, 0x8000, 0xffff, 0x7fffffffu, 0x80000000u,
    0xffffffffu, 0x123456789abcdef0ull, 0x8000000000000000ull, 0xffffffffffffffffull,
};
#define NSEEDS (int)(sizeof seeds / sizeof seeds[0])

#define RUN(T, CK)                                                               \
    for (int i = 0; i < NSEEDS; i++) {                                           \
        T x = (T)seeds[i], y = (T)seeds[(i + 3) % NSEEDS], z;                     \
        T m = x;                                                                 \
        CK(ld_##T(&m));                                                          \
        CK(ld_acq_##T(&m));                                                      \
        CK(ld_rlx_##T(&m));                                                      \
        st_##T(&m, y);                                                           \
        CK(m);                                                                   \
        st_rel_##T(&m, x);                                                       \
        CK(m);                                                                   \
        st_rlx_##T(&m, (T)(W(x) + W(y)));                                              \
        CK(m);                                                                   \
        m = x;                                                                   \
        CK(xchg_##T(&m, y));                                                     \
        CK(m);                                                                   \
        CK(fadd_##T(&m, x));                                                     \
        CK(m);                                                                   \
        CK(fsub_##T(&m, (T)(W(y) + 1)));                                            \
        CK(m);                                                                   \
        CK(fand_##T(&m, (T)(x | 0x55)));                                         \
        CK(m);                                                                   \
        CK(for_##T(&m, (T)(y & 0x3c3c)));                                        \
        CK(m);                                                                   \
        CK(fxor_##T(&m, x));                                                     \
        CK(m);                                                                   \
        CK(fnand_##T(&m, y));                                                    \
        CK(m);                                                                   \
        CK(addf_##T(&m, (T)~x));                                                 \
        CK(subf_##T(&m, y));                                                     \
        CK(nandf_##T(&m, (T)(x ^ y)));                                           \
        /* compare-exchange: success, then failure with the expected value  */  \
        /* written back, at the full width (seeds differing in high bits).  */  \
        m = x;                                                                   \
        z = x;                                                                   \
        CHECK(cas_##T(&m, &z, y));                                               \
        CK(m);                                                                   \
        CK(z);                                                                   \
        z = (T)(x ^ (T)1);                                                       \
        CHECK(cas_##T(&m, &z, x));                                               \
        CK(m);                                                                   \
        CK(z);                                                                   \
        z = y;                                                                   \
        CHECK(cas_rel_##T(&m, &z, (T)(W(x) - 1)));                                  \
        CK(m);                                                                   \
        z = x;                                                                   \
        CHECK(casw_##T(&m, &z, y));                                              \
        CK(m);                                                                   \
        CK(z);                                                                   \
        z = (T)(W(x) - 1);                                                          \
        CHECK(casw_##T(&m, &z, (T)7));                                           \
        CK(m);                                                                   \
    }

#define RUN_SYNC(T)                                                              \
    for (int i = 0; i < NSEEDS; i++) {                                           \
        T x = (T)seeds[i], y = (T)seeds[(i + 5) % NSEEDS];                        \
        T m = x;                                                                 \
        CHECK(vcas_##T(&m, x, y));                                               \
        CHECK(m);                                                                \
        CHECK(vcas_##T(&m, x, (T)3));                                            \
        CHECK(m);                                                                \
        CHECK(bcas_##T(&m, y, x));                                               \
        CHECK(bcas_##T(&m, y, x));                                               \
        CHECK(m);                                                                \
        CHECK(tas_##T(&m, (T)1));                                                \
        CHECK(m);                                                                \
        rel_##T(&m);                                                             \
        CHECK(m);                                                                \
        m = y;                                                                   \
        CHECK(sadd_##T(&m, x));                                                  \
        CHECK(ssubf_##T(&m, (T)(y >> 1)));                                       \
    }

static const i128 wide[] = {
    0,
    1,
    (i128)0xffffffffffffffffull,
    ((i128)1 << 64),
    ((i128)0x0123456789abcdefll << 64) | (i128)0xfedcba9876543210ull,
    -1,
    (i128)((u128)1 << 127),
    (i128)(((u128)-2 << 64) | 0x8000000000000000ull),
};
#define NWIDE (int)(sizeof wide / sizeof wide[0])

int main(void)
{
    RUN(i8, CHECK)
    GROUP();
    RUN(u8, CHECK)
    GROUP();
    RUN(i16, CHECK)
    GROUP();
    RUN(u16, CHECK)
    GROUP();
    RUN(i32, CHECK)
    GROUP();
    RUN(u32, CHECK)
    GROUP();
    RUN(i64, CHECK)
    GROUP();
    RUN(u64, CHECK)
    GROUP();
    RUN_SYNC(i8)
    GROUP();
    RUN_SYNC(u8)
    GROUP();
    RUN_SYNC(i16)
    GROUP();
    RUN_SYNC(u16)
    GROUP();
    RUN_SYNC(i32)
    GROUP();
    RUN_SYNC(u32)
    GROUP();
    RUN_SYNC(i64)
    GROUP();
    RUN_SYNC(u64)
    GROUP();
    for (int i = 0; i < NSEEDS; i++) {
        u32 w = (u32)seeds[i];
        CHECK(vcas_u32_wide(&w, (u32)seeds[i], 0xffffffffu));
        CHECK(vcas_u32_wide(&w, 0xffffffffu, (u32)seeds[(i + 1) % NSEEDS]));
        CHECK(ld_u32_wide(&w));
        i32 s = (i32)seeds[i];
        CHECK(fadd_i32_wide(&s, -1));
        CHECK(s);
    }
    GROUP();
    /* 128-bit: pairs whose sums carry out of the low doubleword. */
    for (int j = 0; j < NWIDE; j++) {
        static const u64 lo[] = { 0, 1, 0x7f, 0x80, 0xffffffffffffffffull };
        for (int k = 0; k < 2; k++) {
            i128 x = wide[j], y = wide[(j + 1 + k) % NWIDE];
            {
                i128 m = x, z;
                CHECK128(ld_i128(&m));
                CHECK128(ld_acq_i128(&m));
                CHECK128(ld_rlx_i128(&m));
                st_i128(&m, y);
                CHECK128(m);
                st_rel_i128(&m, x);
                CHECK128(m);
                st_rlx_i128(&m, (i128)(W(x) + W(y)));
                CHECK128(m);
                m = x;
                CHECK128(xchg_i128(&m, y));
                CHECK128(fadd_i128(&m, x));
                CHECK128(m);
                CHECK128(fsub_i128(&m, (i128)(W(y) + 1)));
                CHECK128(fand_i128(&m, x | 0x55));
                CHECK128(for_i128(&m, y & 0x3c3c));
                CHECK128(fxor_i128(&m, x));
                CHECK128(fnand_i128(&m, y));
                CHECK128(addf_i128(&m, ~x));
                CHECK128(subf_i128(&m, y));
                CHECK128(nandf_i128(&m, x ^ y));
                CHECK128(m);
                m = x;
                z = x;
                CHECK(cas_i128(&m, &z, y));
                CHECK128(m);
                /* Same low half, different high half: must fail. */
                z = x ^ ((i128)1 << 64);
                CHECK(cas_i128(&m, &z, x));
                CHECK128(m);
                CHECK128(z);
                z = y;
                CHECK(casw_i128(&m, &z, (i128)(W(x) - 1)));
                CHECK128(m);
                z = m;
                CHECK(cas_rel_i128(&m, &z, (i128)lo[j % 5]));
                CHECK128(m);
            }
            {
                u128 m = (u128)x, z = (u128)y;
                CHECK128(fadd_u128(&m, (u128)lo[k + 3]));
                CHECK128(m);
                CHECK128(fsub_u128(&m, (u128)lo[k + 1]));
                CHECK128(m);
                CHECK(cas_u128(&m, &z, (u128)x));
                CHECK128(z);
                CHECK128(ld_u128(&m));
            }
            GROUP();
        }
    }
    {
        u64 c = 5;
        CHECK(loop_counter(&c, 0));
        CHECK(loop_counter(&c, 1));
        CHECK(loop_counter(&c, 17));
        static const u32 vals[] = { 3, 9, 1, 0xfffffff0u, 4, 0x80000000u, 0xffffffffu, 2 };
        u32 mx = 0;
        CHECK(loop_cas_max(&mx, vals, 3));
        CHECK(loop_cas_max(&mx, vals, 8));
    }
    GROUP();
    {
        struct counters s = { 0, -7, { 1, 2, 0xfffffffeu, 4 }, { -5, 0, 0x7fffffffffffffffll } };
        for (int i = 0; i < 9; i++) {
            CHECK(field_ops(&s, i));
            CHECK(s.flag);
            CHECK(s.h);
            s.flag = 0;
        }
        for (int i = 0; i < 4; i++)
            CHECK(s.w[i]);
        for (int i = 0; i < 3; i++)
            CHECK(s.d[i]);
    }
    GROUP();
    for (int i = 0; i < NSEEDS; i++) {
        i64 a = (i64)seeds[i], b = (i64)seeds[(i + 1) % NSEEDS];
        i8 c8 = (i8)a;
        u16 c16 = (u16)b;
        i32 c32 = (i32)(a ^ b);
        u64 c64 = (u64)a - (u64)b;
        CHECK(pressure_i8(&c8, (u64)a, (u64)b, (u64)i, (u64)(a >> 3), (u64)(b >> 5), -(u64)a));
        CHECK(c8);
        CHECK(pressure_u16(&c16, (u64)b, (u64)a, -(u64)i, (u64)(a >> 7), (u64)b, 3));
        CHECK(c16);
        CHECK(pressure_i32(&c32, (u64)a, (u64)a, (u64)b, (u64)b, (u64)i, (u64)i));
        CHECK(c32);
        CHECK(pressure_u64(&c64, -(u64)a, (u64)b, (u64)a, -(u64)b, 1, 2));
        CHECK(c64);
    }
    GROUP();
    {
        static const float fv[] = { 1.5f, -0.0f, 3.25e10f, -7.0f };
        struct fpair q = { 0.5f, -2.75f, 9.5 };
        for (int i = 0; i < 4; i++) {
            CHECK(f32_bits(xchg_f32(&q.a, fv[i])));
            CHECK(f32_bits(q.a));
            CHECK(f32_bits(q.b));
            CHECK(f32_bits(load_f32(&q.b)));
            store_f32(&q.b, fv[(i + 1) & 3]);
            CHECK(f32_bits(q.a));
            CHECK(f32_bits(q.b));
            CHECK(f64_bits(q.c));
            CHECK(f64_bits(xchg_f64(&q.c, (double)fv[i] * 2.0)));
            float e = q.a;
            CHECK(cas_f32(&q.a, &e, fv[(i + 2) & 3]));
            CHECK(f32_bits(q.a));
            e = 1234.5f;
            CHECK(cas_f32(&q.a, &e, 0.0f));
            CHECK(f32_bits(e));
            CHECK(f32_bits(q.b));
        }
    }
    GROUP();
    for (int k = -3; k <= 3; k++)
        CHECK(atomic_compound(k));
    GROUP();
#ifdef GEN
    return 0;
#else
    return g__ == (int)(sizeof expected / sizeof expected[0]) ? 0 : 255;
#endif
}
