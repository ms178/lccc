/* x86-64 bitwise atomic RMW (and/or/xor/nand) operand sourcing.
 *
 * The CMPXCHG loop owns %rax (old), %rcx (address) and %rdx (candidate) and
 * reads its operand in place.  Every source the emitter can pick is driven
 * here, each checked against a plain-C reference:
 *   - an imm32 (including narrow constants in [2^31, 2^32) re-spelled as
 *     their negative imm32 twin, and the 64-bit sign-extension boundary);
 *   - a register home: the operand in %rsi (second argument), in %rdi
 *     (first argument, pointer second), and in a callee-saved home;
 *   - the stack-staged fallback: 64-bit constants beyond imm32, and an
 *     operand spilled under register pressure, with %rsp-relative slots read
 *     after the push/pop pair (frame-offset bookkeeping);
 *   - registers the loop must NOT clobber: %rdi/%rsi/%r8/%r9 arguments read
 *     after the atomic, and values live across it in a loop;
 *   - register-homed atomic POINTERS (x86-64 narrow atomics keep their
 *     address in a register): a counter pointer live across a loop, struct
 *     field addresses (GEP-derived), every atomic kind addressing through
 *     the same home. */
#include <stdio.h>

typedef signed char i8;
typedef unsigned char u8;
typedef short i16;
typedef unsigned short u16;
typedef int i32;
typedef unsigned u32;
typedef long long i64;
typedef unsigned long long u64;

#define NI __attribute__((noinline))

/* ---- register operand, all ops x all widths -------------------------- */
#define RMW_REG(T, NAME, BUILTIN) \
    NI static T NAME(T *p, T v) { return BUILTIN(p, v, __ATOMIC_SEQ_CST); }
#define WIDTH(T, S)                                   \
    RMW_REG(T, and_##S, __atomic_fetch_and)           \
    RMW_REG(T, or_##S, __atomic_fetch_or)             \
    RMW_REG(T, xor_##S, __atomic_fetch_xor)           \
    RMW_REG(T, nand_##S, __atomic_fetch_nand)
WIDTH(i8, i8)
WIDTH(u8, u8)
WIDTH(i16, i16)
WIDTH(u16, u16)
WIDTH(i32, i32)
WIDTH(u32, u32)
WIDTH(i64, i64)
WIDTH(u64, u64)

/* ---- operand in %rdi: pointer is the SECOND argument ----------------- */
NI static u64 or_swapped(u64 v, u64 *p) { return __atomic_fetch_or(p, v, __ATOMIC_SEQ_CST); }
NI static u32 nand_swapped(u32 v, u32 *p) { return __atomic_fetch_nand(p, v, __ATOMIC_RELAXED); }

/* ---- immediates -------------------------------------------------------- */
NI static u32 and_u32_hi(u32 *p) { return __atomic_fetch_and(p, 0xF0F0F0F0u, __ATOMIC_SEQ_CST); }
NI static u32 or_u32_top(u32 *p) { return __atomic_fetch_or(p, 0x80000000u, __ATOMIC_SEQ_CST); }
NI static u16 xor_u16_hi(u16 *p) { return __atomic_fetch_xor(p, (u16)0xFF00, __ATOMIC_SEQ_CST); }
NI static u8 nand_u8_hi(u8 *p) { return __atomic_fetch_nand(p, (u8)0xA5, __ATOMIC_SEQ_CST); }
NI static i64 and_i64_max32(i64 *p) { return __atomic_fetch_and(p, 0x7FFFFFFFLL, __ATOMIC_SEQ_CST); }
NI static i64 or_i64_min32(i64 *p) { return __atomic_fetch_or(p, -0x80000000LL, __ATOMIC_SEQ_CST); }
NI static u64 xor_u64_ones(u64 *p) { return __atomic_fetch_xor(p, ~0ULL, __ATOMIC_SEQ_CST); }
/* beyond imm32: stack-staged */
NI static u64 and_u64_wide(u64 *p) { return __atomic_fetch_and(p, 0xFFFFFFFF00000000ULL, __ATOMIC_SEQ_CST); }
NI static u64 or_u64_2p31(u64 *p) { return __atomic_fetch_or(p, 0x80000000ULL, __ATOMIC_SEQ_CST); }
NI static u64 nand_u64_wide(u64 *p) { return __atomic_fetch_nand(p, 0x123456789ULL, __ATOMIC_SEQ_CST); }

/* ---- arguments read after the atomic must survive it ------------------ */
NI static u64 keep_args(u64 *p, u64 v, u64 a, u64 b, u64 c, u64 d) {
    u64 old = __atomic_fetch_xor(p, v, __ATOMIC_SEQ_CST);
    /* p (%rdi), v (%rsi), c (%r8), d (%r9) are all read afterwards */
    return old + (u64)p * 0 + v * 3 + a * 5 + b * 7 + c * 11 + d * 13 + *p;
}

/* ---- register pressure: operands spill, slots read after push/pop ----- */
NI static u64 pressure(u64 *p, const u64 *src, int n) {
    u64 acc = 0;
    for (int i = 0; i < n; ++i) {
        u64 a = src[i] * 3, b = src[i] ^ 0x55, c = src[i] + 7, d = src[i] << 2;
        u64 e = src[i] >> 3, f = src[i] * 9, g = ~src[i], h = src[i] | 0x100;
        u64 k = a ^ b ^ c ^ d ^ e ^ f ^ g ^ h;
        u64 o1 = __atomic_fetch_and(p, k | 1, __ATOMIC_SEQ_CST);
        u64 o2 = __atomic_fetch_or(p, a ^ e, __ATOMIC_SEQ_CST);
        u64 o3 = __atomic_fetch_nand(p, b + f, __ATOMIC_SEQ_CST);
        u64 o4 = __atomic_fetch_xor(p, c * g + h, __ATOMIC_SEQ_CST);
        acc += o1 * 1 + o2 * 3 + o3 * 5 + o4 * 7 + a + b + c + d + e + f + g + h;
    }
    return acc;
}
static u64 pressure_ref(u64 *p, const u64 *src, int n) {
    u64 acc = 0;
    for (int i = 0; i < n; ++i) {
        u64 a = src[i] * 3, b = src[i] ^ 0x55, c = src[i] + 7, d = src[i] << 2;
        u64 e = src[i] >> 3, f = src[i] * 9, g = ~src[i], h = src[i] | 0x100;
        u64 k = a ^ b ^ c ^ d ^ e ^ f ^ g ^ h;
        u64 o1 = *p; *p &= (k | 1);
        u64 o2 = *p; *p |= (a ^ e);
        u64 o3 = *p; *p = ~(*p & (b + f));
        u64 o4 = *p; *p ^= (c * g + h);
        acc += o1 * 1 + o2 * 3 + o3 * 5 + o4 * 7 + a + b + c + d + e + f + g + h;
    }
    return acc;
}

/* ---- register-homed atomic pointers --------------------------------- */
struct shared { u32 pad; u32 flags; u32 seq; u64 ctr; };
NI static u64 pointer_homes(struct shared *sh, const u32 *src, int n) {
    u64 acc = 0;
    for (int i = 0; i < n; ++i) {
        __atomic_fetch_add(&sh->ctr, src[i], __ATOMIC_RELAXED);
        acc += __atomic_fetch_or(&sh->flags, src[i] & 0xF0F0u, __ATOMIC_ACQ_REL);
        u32 exp = __atomic_load_n(&sh->flags, __ATOMIC_ACQUIRE);
        __atomic_compare_exchange_n(&sh->flags, &exp, exp ^ src[i], 0, __ATOMIC_SEQ_CST,
                                    __ATOMIC_SEQ_CST);
        __atomic_store_n(&sh->seq, (u32)i, __ATOMIC_SEQ_CST);
        acc += __atomic_load_n(&sh->seq, __ATOMIC_RELAXED);
        acc += __atomic_fetch_nand(&sh->pad, src[i], __ATOMIC_RELAXED);
    }
    return acc + __atomic_load_n(&sh->ctr, __ATOMIC_SEQ_CST);
}
static u64 pointer_homes_ref(struct shared *sh, const u32 *src, int n) {
    u64 acc = 0;
    for (int i = 0; i < n; ++i) {
        sh->ctr += src[i];
        acc += sh->flags; sh->flags |= src[i] & 0xF0F0u;
        u32 exp = sh->flags; sh->flags = exp ^ src[i];
        sh->seq = (u32)i;
        acc += sh->seq;
        acc += sh->pad; sh->pad = ~(sh->pad & src[i]);
    }
    return acc + sh->ctr;
}

static u64 lcg(u64 *s) { *s = *s * 6364136223846793005ULL + 1442695040888963407ULL; return *s; }

#define CHECK_REG(T, S, OPEXPR)                                              \
    do {                                                                     \
        T x = (T)o, v = (T)w;                                                \
        T r = S(&x, v);                                                      \
        T want = (T)(OPEXPR);                                                \
        if (r != (T)o || x != want) {                                        \
            printf("FAIL %s old=%llx v=%llx got r=%llx x=%llx want %llx\n",  \
                   #S, (u64)(T)o, (u64)v, (u64)r, (u64)x, (u64)want);        \
            return 1;                                                        \
        }                                                                    \
    } while (0)
#define CHECK_WIDTH(T, SUF)                                     \
    CHECK_REG(T, and_##SUF, (T)o & v);                          \
    CHECK_REG(T, or_##SUF, (T)o | v);                           \
    CHECK_REG(T, xor_##SUF, (T)o ^ v);                          \
    CHECK_REG(T, nand_##SUF, ~((T)o & v))

int main(void) {
    u64 s = 0x9E3779B97F4A7C15ULL, sum = 0;
    for (int it = 0; it < 4000; ++it) {
        u64 o = lcg(&s), w = lcg(&s);
        if (it < 16) { o = it & 1 ? ~0ULL : 0; w = it & 2 ? 0x8000000080008080ULL : ~0ULL; }
        CHECK_WIDTH(i8, i8);
        CHECK_WIDTH(u8, u8);
        CHECK_WIDTH(i16, i16);
        CHECK_WIDTH(u16, u16);
        CHECK_WIDTH(i32, i32);
        CHECK_WIDTH(u32, u32);
        CHECK_WIDTH(i64, i64);
        CHECK_WIDTH(u64, u64);
        { u64 x = o; if (or_swapped(w, &x) != o || x != (o | w)) { puts("FAIL or_swapped"); return 2; } }
        { u32 x = (u32)o; if (nand_swapped((u32)w, &x) != (u32)o || x != (u32)~((u32)o & (u32)w)) { puts("FAIL nand_swapped"); return 3; } }
        { u32 x = (u32)o; if (and_u32_hi(&x) != (u32)o || x != ((u32)o & 0xF0F0F0F0u)) { puts("FAIL and_u32_hi"); return 4; } }
        { u32 x = (u32)o; if (or_u32_top(&x) != (u32)o || x != ((u32)o | 0x80000000u)) { puts("FAIL or_u32_top"); return 5; } }
        { u16 x = (u16)o; if (xor_u16_hi(&x) != (u16)o || x != (u16)((u16)o ^ 0xFF00)) { puts("FAIL xor_u16_hi"); return 6; } }
        { u8 x = (u8)o; if (nand_u8_hi(&x) != (u8)o || x != (u8)~((u8)o & 0xA5)) { puts("FAIL nand_u8_hi"); return 7; } }
        { i64 x = (i64)o; if (and_i64_max32(&x) != (i64)o || x != ((i64)o & 0x7FFFFFFFLL)) { puts("FAIL and_i64_max32"); return 8; } }
        { i64 x = (i64)o; if (or_i64_min32(&x) != (i64)o || x != ((i64)o | -0x80000000LL)) { puts("FAIL or_i64_min32"); return 9; } }
        { u64 x = o; if (xor_u64_ones(&x) != o || x != ~o) { puts("FAIL xor_u64_ones"); return 10; } }
        { u64 x = o; if (and_u64_wide(&x) != o || x != (o & 0xFFFFFFFF00000000ULL)) { puts("FAIL and_u64_wide"); return 11; } }
        { u64 x = o; if (or_u64_2p31(&x) != o || x != (o | 0x80000000ULL)) { puts("FAIL or_u64_2p31"); return 12; } }
        { u64 x = o; if (nand_u64_wide(&x) != o || x != ~(o & 0x123456789ULL)) { puts("FAIL nand_u64_wide"); return 13; } }
        {
            u64 x = o, y = o, a = lcg(&s), b = lcg(&s), c = lcg(&s), d = lcg(&s);
            u64 got = keep_args(&x, w, a, b, c, d);
            u64 old = y; y ^= w;
            u64 want = old + w * 3 + a * 5 + b * 7 + c * 11 + d * 13 + y;
            if (got != want || x != y) { puts("FAIL keep_args"); return 14; }
            sum += got;
        }
        sum += o ^ w;
    }
    u64 src[64];
    for (int i = 0; i < 64; ++i) src[i] = lcg(&s);
    u64 x = 0xDEADBEEFCAFEF00DULL, y = x;
    u64 got = pressure(&x, src, 64), want = pressure_ref(&y, src, 64);
    if (got != want || x != y) { puts("FAIL pressure"); return 15; }
    u32 s32[64];
    for (int i = 0; i < 64; ++i) s32[i] = (u32)lcg(&s);
    struct shared a = {0x12345678u, 0x0F0F0F0Fu, 0, 99}, b = a;
    u64 pg = pointer_homes(&a, s32, 64), pw = pointer_homes_ref(&b, s32, 64);
    if (pg != pw || a.pad != b.pad || a.flags != b.flags || a.seq != b.seq || a.ctr != b.ctr) {
        puts("FAIL pointer_homes");
        return 16;
    }
    got ^= pg;
    printf("atomic bitwise operand homes ok %016llx %016llx\n", sum, got ^ x);
    return 0;
}
