/* Inline-asm scalar operands of every type, in both register classes, keep
 * their value on every target.
 *
 * Each of these was wrong before (all GCC-accepted unless noted):
 *  - x86-64 -O1 and up: the read half of a `"+x"` operand -- the synthetic
 *    input that carries the old value into the output's register -- was
 *    never loaded when the value lived in an XMM register, so
 *    `asm("addsd %0, %0" : "+x"(v))` doubled a stale register
 *    (`double_through_fp` returned 3069 instead of 120).
 *  - x86-64 and i386: an integer in an SSE register (`"+x"(int)`,
 *    `"=x"(long)`) moved 16 bytes (`movdqu`) into and out of its 4- or
 *    8-byte home, overwriting the neighbouring frame slots (the loop
 *    counter: crashes and wrong trip counts).
 *  - i386: a register-homed integer `"x"` input was never loaded, and every
 *    `"x"` constant became 0.0 (`xorpd`).
 *  - i386: `"r"(double)` got one 32-bit register instead of a pair -- only
 *    the low half went in, and the high half of the output was never written.
 *  - AArch64 -O2: `"r"(double)` whose value lived in an FP register hit an
 *    internal compiler error (a general-register name asked of `d8`).
 *  - RISC-V -O1 and up: an `"f"` operand holding an integer with a register
 *    home hit an internal compiler error (that one is an LCCC extension:
 *    Clang refuses `"f"(long)`, so it is checked on RISC-V only).
 *
 * The cross harness runs this on AArch64 and RISC-V at every level; the
 * native corpus runs it on x86-64 (and i386, SSE parts under __SSE2__).
 */

#if defined(__aarch64__)
#define FP "w"
#elif defined(__riscv)
#define FP "f"
#elif defined(__x86_64__) || defined(__SSE2__)
#define FP "x"
#endif

#define RW(name, T, C)                                                         \
    __attribute__((noinline)) T name(T x, long k)                              \
    {                                                                          \
        T s = x;                                                               \
        for (long i = 0; i < k; i++) {                                         \
            T v = (T)((long)(s) + 1);                                          \
            __asm__ volatile("" : "+" C(v));                                   \
            s = v;                                                             \
        }                                                                      \
        return s;                                                              \
    }
#define TIE(name, T, C)                                                        \
    __attribute__((noinline)) T name(T x, long k)                              \
    {                                                                          \
        T s = x;                                                               \
        for (long i = 0; i < k; i++) {                                         \
            T o;                                                               \
            __asm__ volatile("" : "=" C(o) : "0"(s));                          \
            s = o;                                                             \
        }                                                                      \
        return s;                                                              \
    }
#define CHECK(bit, name, T)                                                    \
    do {                                                                       \
        volatile T x0 = (T)5;                                                  \
        T want = x0;                                                           \
        for (int i = 0; i < 7; i++)                                            \
            want = (T)((long)(want) + 1);                                      \
        if (rw_##name(x0, 7) != want)                                          \
            bad |= (bit);                                                      \
        if (tie_##name(x0, 7) != x0)                                           \
            bad |= (bit);                                                      \
    } while (0)

/* General registers: every scalar type, including the 8-byte floating
 * types (an IA-32 register pair). */
RW(rw_gp_char, char, "r")
TIE(tie_gp_char, char, "r")
RW(rw_gp_short, short, "r")
TIE(tie_gp_short, short, "r")
RW(rw_gp_int, int, "r")
TIE(tie_gp_int, int, "r")
RW(rw_gp_long, long, "r")
TIE(tie_gp_long, long, "r")
RW(rw_gp_llong, long long, "r")
TIE(tie_gp_llong, long long, "r")
RW(rw_gp_float, float, "r")
TIE(tie_gp_float, float, "r")
RW(rw_gp_double, double, "r")
TIE(tie_gp_double, double, "r")
typedef char *charp;
RW(rw_gp_ptr, charp, "r")
TIE(tie_gp_ptr, charp, "r")

#ifdef FP
/* Floating-point / SIMD registers. */
RW(rw_fp_float, float, FP)
TIE(tie_fp_float, float, FP)
RW(rw_fp_double, double, FP)
TIE(tie_fp_double, double, FP)
#if !defined(__riscv)
/* Integers in FP/SIMD registers (GCC: x86 takes 2-, 4- and 8-byte ones,
 * AArch64 any). */
RW(rw_fp_int, int, FP)
TIE(tie_fp_int, int, FP)
RW(rw_fp_llong, long long, FP)
TIE(tie_fp_llong, long long, FP)
#if !defined(__aarch64__)
RW(rw_fp_short, short, FP)
TIE(tie_fp_short, short, FP)
#endif
#else
RW(rw_fp_long, long, FP)
TIE(tie_fp_long, long, FP)
#endif

/* A template that computes: the old value must be in the register. */
__attribute__((noinline)) double double_through_fp(double d, long n)
{
    double acc = 0;
    for (long i = 0; i < n; i++) {
        double v = d + (double)i;
#if defined(__aarch64__)
        __asm__("fadd %d0, %d0, %d0" : "+w"(v));
#elif defined(__riscv)
        __asm__("fadd.d %0, %0, %0" : "+f"(v));
#else
        __asm__("addsd %0, %0" : "+x"(v));
#endif
        acc += v;
    }
    return acc;
}

/* A constant into an FP-class register: its bits, not zero. */
__attribute__((noinline)) double fp_constant(void)
{
    double r;
#if defined(__aarch64__)
    __asm__("fmov %d0, %d1" : "=w"(r) : "w"(2.5));
#elif defined(__riscv)
    __asm__("fmv.d %0, %1" : "=f"(r) : "f"(2.5));
#else
    __asm__("movapd %1, %0" : "=x"(r) : "x"(2.5));
#endif
    return r;
}
#endif

/* A double's bits through a general register (AArch64: the value lives in
 * an FP register across the loop). */
#if defined(__aarch64__) || defined(__riscv) || defined(__x86_64__)
__attribute__((noinline)) double double_bits_through_gp(double d, long n)
{
    double acc = 0;
    for (long i = 0; i < n; i++) {
        double w = d * (double)i;
        unsigned long bits;
#if defined(__aarch64__)
        __asm__("mov %0, %1" : "=r"(bits) : "r"(w));
#elif defined(__riscv)
        __asm__("mv %0, %1" : "=r"(bits) : "r"(w));
#else
        __asm__("movq %1, %0" : "=r"(bits) : "r"(w));
#endif
        double back;
        __builtin_memcpy(&back, &bits, sizeof back);
        acc += back;
    }
    return acc;
}
#endif

int main(void)
{
    int bad = 0;
    CHECK(1 << 0, gp_char, char);
    CHECK(1 << 1, gp_short, short);
    CHECK(1 << 2, gp_int, int);
    CHECK(1 << 3, gp_long, long);
    CHECK(1 << 4, gp_llong, long long);
    CHECK(1 << 5, gp_float, float);
    CHECK(1 << 6, gp_double, double);
    CHECK(1 << 7, gp_ptr, charp);
#ifdef FP
    CHECK(1 << 8, fp_float, float);
    CHECK(1 << 9, fp_double, double);
#if !defined(__riscv)
    CHECK(1 << 10, fp_int, int);
    CHECK(1 << 11, fp_llong, long long);
#if !defined(__aarch64__)
    CHECK(1 << 12, fp_short, short);
#endif
#else
    CHECK(1 << 10, fp_long, long);
#endif
    if (double_through_fp(1.5, 10) != 120.0)
        bad |= 1 << 13;
    if (fp_constant() != 2.5)
        bad |= 1 << 14;
#endif
#if defined(__aarch64__) || defined(__riscv) || defined(__x86_64__)
    if (double_bits_through_gp(1.5, 4) != 1.5 * (0 + 1 + 2 + 3))
        bad |= 1 << 15;
#endif
    return bad != 0;
}
