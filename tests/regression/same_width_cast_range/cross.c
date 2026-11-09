/* Freestanding four-ISA execution fixture for same-width signedness casts.
 *
 * The range-gated `cast_sign_range` simplify rule may delete a same-width
 * signedness Cast ONLY when the source's sign bit is provably clear.  These
 * functions pair every foldable producer with a sign-bit-set control and an
 * opaque volatile-Load control at each supported width.  These sign-bit
 * controls are end-to-end ABI/execution checks for the four backends'
 * canonicalisation (`sext.w`, `sxtw`/`mov w0,w0`, `cast_normalize_eax`,
 * `movl`), NOT an architecture-safe unsoundness detector: a deliberately
 * unconditional fold removes all 17 guard Casts yet three host compilers and
 * all four ISAs still return the oracle because the ABI/backend repairs the
 * upper bits.  The per-function IR census in
 * `scripts/check_same_width_cast_range.py` is the load-bearing assertion;
 * `CCC_SIMPLIFY_SKIP=cast_sign_range` proves the five folded sections have
 * teeth of their own.
 *
 * No libc, no headers: the harness links a per-ISA `_start` and executes the
 * exit status natively (x86-64) or under qemu-user (i686/AArch64/RISC-V64).
 */
static volatile unsigned int g_u;
static volatile int g_v;
static volatile int g_s;
static volatile unsigned long long g_w;

/* Full-width sign-bit-set sources.  Each pair covers BOTH directions. */
static volatile unsigned char g_u8;
static volatile signed char g_i8;
static volatile unsigned short g_u16;
static volatile short g_i16;
static volatile unsigned long long g_u64;
static volatile long long g_i64;

/* Non-proven, sign-bit-CLEAR sources: a volatile Load is opaque to the proof.
 * These must keep the Cast even though runtime values are non-negative. */
static volatile unsigned char g_c8;
static volatile unsigned short g_c16;
static volatile unsigned int g_c32;
static volatile unsigned long long g_c64;
static volatile signed char g_s8;
static volatile short g_s16;
static volatile int g_s32;
static volatile long long g_s64;

/* U32 & 1 -> I32: sign bit clear, the Cast is representation-neutral. */
__attribute__((noinline)) static int masked(void) {
    return (int)(g_u & 1U);
}

/* U32 | 0x80000000 -> I32: sign bit SET, the Cast MUST canonicalize. */
__attribute__((noinline)) static int signbit(void) {
    return (int)(g_u | 0x80000000U);
}

/* I32 & 0x7fffffff -> U32: sign bit clear, reverse direction. */
__attribute__((noinline)) static unsigned int masked_u(void) {
    return (unsigned int)(g_v & 0x7fffffff);
}

/* I32 -> U32 of a negative value: sign bit SET, MUST zero-extend. */
__attribute__((noinline)) static unsigned int signed_to_unsigned(void) {
    return (unsigned int)g_s;
}

/* U64 >> 1 -> I64: a logical shift clears the sign bit. */
__attribute__((noinline)) static long long shifted(void) {
    return (long long)(g_w >> 1);
}

/* Narrow homes and a boolean result remain correct under either decision. */
__attribute__((noinline)) static signed char narrow_i8(void) {
    return (signed char)(g_u & 0x7fU);
}

__attribute__((noinline)) static int bool_cast(void) {
    return (int)(unsigned char)(g_u == 3U);
}

/* JOIN TEETH: the frontend omits its own Cast for a plain mask, but it does
 * NOT propagate non-negativity through a ternary merge, so these functions
 * contain a REAL U32->I32 Cast on a Select.  `select_masked` and `phi_masked`
 * must fold; `select_signbit` has one ambiguous arm and must keep the Cast. */
__attribute__((noinline)) static int select_masked(unsigned int x, int c) {
    unsigned int u = c ? (x & 0x7fffffffU) : (x & 1U);
    return (int)u;
}

__attribute__((noinline)) static int phi_masked(unsigned int x, int c) {
    unsigned int u;
    if (c) u = x & 0x7fffffffU; else u = x & 1U;
    return (int)u;
}

__attribute__((noinline)) static int select_signbit(unsigned int x, int c) {
    unsigned int u = c ? (x | 0x80000000U) : (x & 1U);
    return (int)u;
}

__attribute__((noinline)) static int select_is_negative(unsigned int x, int c) {
    unsigned int u = c ? (x | 0x80000000U) : (x & 1U);
    int y = (int)u;
    return y < 0;
}

/* OBSERVATION TEETH: compare the sign-bit-set result INSIDE the function,
 * before any call/return ABI canonicalization can repair the upper bits.
 * On RISC-V the U32->I32 Cast must emit sext.w; without it x is a
 * zero-extended positive 64-bit home and `x < 0` is false. */
__attribute__((noinline)) static int signbit_is_negative(void) {
    int x = (int)(g_u | 0x80000000U);
    return x < 0;
}

/* On x86-64 the I32->U32 Cast emits `movl` (zero upper half).  Without it
 * the U32->U64 widen below assumes an already zero-extended home and leaves
 * 0xfffffffffffffffb, so the 64-bit comparison fails. */
__attribute__((noinline)) static int s2u_wide_ok(void) {
    unsigned int u = (unsigned int)g_s;
    unsigned long long w = u;
    return w == 0xfffffffbULL;
}

/* ===== Full-width sign-bit-set controls, observed before the ABI ===== */
__attribute__((noinline)) static int u8_signbit(void) {
    unsigned char u = (unsigned char)(g_u8 | 0x80U);   /* 0x85 */
    signed char s = (signed char)u;                     /* MUST be negative */
    return s < 0 && (unsigned char)s == u
        && ((unsigned int)s & 0xffU) == (unsigned int)u;
}

__attribute__((noinline)) static int i8_signbit(void) {
    signed char s = (signed char)g_i8;                  /* -5 */
    unsigned char u = (unsigned char)s;                 /* 0xfb */
    unsigned int widened = u;
    return (unsigned int)u == 0xfbU && widened == 0xfbU;
}

__attribute__((noinline)) static int u16_signbit(void) {
    unsigned short u = (unsigned short)(g_u16 | 0x8000U); /* 0x8005 */
    short s = (short)u;                                    /* MUST be negative */
    return s < 0 && (unsigned short)s == u
        && ((unsigned int)s & 0xffffU) == (unsigned int)u;
}

__attribute__((noinline)) static int i16_signbit(void) {
    short s = (short)g_i16;                             /* -5 */
    unsigned short u = (unsigned short)s;               /* 0xfffb */
    unsigned int widened = u;
    return (unsigned int)u == 0xfffbU && widened == 0xfffbU;
}

__attribute__((noinline)) static int u64_signbit(void) {
    unsigned long long u = g_u64 | 0x8000000000000000ULL;
    long long s = (long long)u;                          /* MUST be negative */
    return s < 0 && (unsigned long long)s == u;
}

__attribute__((noinline)) static int i64_signbit(void) {
    long long s = g_i64;                                 /* -5 */
    unsigned long long u = (unsigned long long)s;        /* 0xffff...fffb */
    unsigned int lo = (unsigned int)u;
    unsigned int hi = (unsigned int)(u >> 32);
    return u == 0xfffffffffffffffbULL && lo == 0xfffffffbU
        && hi == 0xffffffffU;
}

/* ===== Opaque volatile-Load controls: sign bit CLEAR, proof MUST refuse ===== */
__attribute__((noinline)) static signed char u8_unproven(void) {
    return (signed char)g_c8;
}
__attribute__((noinline)) static unsigned char i8_unproven(void) {
    return (unsigned char)g_s8;
}
__attribute__((noinline)) static short u16_unproven(void) {
    return (short)g_c16;
}
__attribute__((noinline)) static unsigned short i16_unproven(void) {
    return (unsigned short)g_s16;
}
__attribute__((noinline)) static int u32_unproven(void) {
    return (int)g_c32;
}
__attribute__((noinline)) static unsigned int i32_unproven(void) {
    return (unsigned int)g_s32;
}
__attribute__((noinline)) static long long u64_unproven(void) {
    return (long long)g_c64;
}
__attribute__((noinline)) static unsigned long long i64_unproven(void) {
    return (unsigned long long)g_s64;
}

int lccc_main(void) {
    g_u = 0x80000005U;
    g_v = 0x80000005;
    g_s = -5;
    g_w = 0x8000000000000003ULL;
    g_u8 = 0x85U;
    g_i8 = -5;
    g_u16 = 5U;
    g_i16 = -5;
    g_u64 = 3ULL;
    g_i64 = -5;
    g_c8 = 5U;
    g_c16 = 5U;
    g_c32 = 5U;
    g_c64 = 5ULL;
    g_s8 = 5;
    g_s16 = 5;
    g_s32 = 5;
    g_s64 = 5;

    int a = masked();                         /* 1 */
    int b = signbit();                        /* 0x80000005 as int (negative) */
    unsigned int c = masked_u();              /* 5 */
    unsigned long long wide = (unsigned long long)signed_to_unsigned();
    long long f = shifted();                  /* 0x4000000000000001 */
    int d = narrow_i8();                      /* 5 */
    int e = bool_cast();                      /* 0 */
    int g = signbit_is_negative();            /* 1, pre-ABI home */
    int k = s2u_wide_ok();                    /* 1, pre-ABI home */
    int m = select_masked(g_u, 1);            /* 5, folded join */
    int n = select_masked(g_u, 0);            /* 1, folded join */
    int r = phi_masked(g_u, 1);               /* 5, folded join */
    int q = select_signbit(g_u, 1);           /* 0x80000005, kept Cast */
    int t = select_is_negative(g_u, 1);       /* 1, kept Cast */
    int u8s = u8_signbit();                   /* 1 */
    int i8s = i8_signbit();                   /* 1 */
    int u16s = u16_signbit();                 /* 1 */
    int i16s = i16_signbit();                 /* 1 */
    int u64s = u64_signbit();                 /* 1 */
    int i64s = i64_signbit();                 /* 1 */
    signed char u8u = u8_unproven();          /* 5 */
    unsigned char i8u = i8_unproven();        /* 5 */
    short u16u = u16_unproven();              /* 5 */
    unsigned short i16u = i16_unproven();     /* 5 */
    int u32u = u32_unproven();                /* 5 */
    unsigned int i32u = i32_unproven();       /* 5 */
    long long u64u = u64_unproven();          /* 5 */
    unsigned long long i64u = i64_unproven(); /* 5 */

    int ok = (a == 1) && (b < 0) && ((unsigned int)b == 0x80000005U)
          && (c == 5U) && (wide == 0xfffffffbULL) && (f > 0)
          && (d == 5) && (e == 0) && (g == 1) && (k == 1)
          && (m == 5) && (n == 1) && (r == 5) && (q < 0)
          && ((unsigned int)q == 0x80000005U) && (t == 1)
          && (u8s == 1) && (i8s == 1) && (u16s == 1) && (i16s == 1)
          && (u64s == 1) && (i64s == 1)
          && (u8u == 5) && ((unsigned int)i8u == 5U)
          && (u16u == 5) && ((unsigned int)i16u == 5U)
          && (u32u == 5) && (i32u == 5U)
          && (u64u == 5) && (i64u == 5ULL);
    unsigned int h = (unsigned int)a * 1u + (unsigned int)b * 3u
                   + c * 5u + (unsigned int)wide * 7u
                   + (unsigned int)f * 11u + (unsigned int)d * 13u
                   + (unsigned int)e * 17u + (unsigned int)g * 19u
                   + (unsigned int)k * 23u + (unsigned int)m * 29u
                   + (unsigned int)n * 31u + (unsigned int)r * 37u
                   + (unsigned int)q * 41u + (unsigned int)t * 43u
                   + (unsigned int)u8s * 47u + (unsigned int)i8s * 53u
                   + (unsigned int)u16s * 59u + (unsigned int)i16s * 67u
                   + (unsigned int)u64s * 71u + (unsigned int)i64s * 73u
                   + (unsigned int)u8u * 79u + (unsigned int)i8u * 83u
                   + (unsigned int)u16u * 89u + (unsigned int)i16u * 97u
                   + (unsigned int)u32u * 101u + (unsigned int)i32u * 103u
                   + (unsigned int)u64u * 107u + (unsigned int)i64u * 109u;
    return (int)((h ^ (ok ? 0x5au : 0xffu)) % 61u);
}
