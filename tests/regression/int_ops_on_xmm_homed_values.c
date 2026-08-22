/* Integer ops on XMM-homed values (bit-punned float words).
 *
 * After gnu89 extern-inline bodies behind __asm__ labels became inlinable
 * (glibc hidden_proto), the flt-32 math kernels exposed a backend hole:
 * integer BinOps/compares whose operand or dest the RA homed in an XMM
 * register (GET_FLOAT_WORD keeps the word in the FP domain) reached GPR
 * name lookups and died at unreachable!("invalid x86 register index"):
 *
 *   - emit_int_cmp_insn_typed (s_tanf/e_powf/s_erfcf/...)
 *   - emit_alu_reg_direct rhs + alu.rs dispatch dest (e_logf/e_powf)
 *   - emit_bit_test_reg_direct base (s_nextupf: BitTest on the sign word)
 *   - andn dest / mem-dest rhs / int-madd dest
 *
 * All such paths now filter XMM homes into the accumulator fallbacks
 * (operand_to_rax/rcx + store_rax_to handle the xmm<->GPR movq).
 *
 * This test mirrors the glibc shape: hidden_proto-style extern-inline
 * float helpers + bit-punned word manipulation with live FP pressure.
 */
#include <stdio.h>

typedef unsigned int u32;

extern float wrap_fabsf(float x) __asm__("__GI_wrap_fabsf");
extern __inline __attribute__((__gnu_inline__)) float wrap_fabsf(float x)
{
    return __builtin_fabsf(x);
}

static inline u32 bits(float x)
{
    union { float f; u32 i; } u;
    u.f = x;
    return u.i;
}

static inline float fromb(u32 i)
{
    union { float f; u32 i; } u;
    u.i = i;
    return u.f;
}

__attribute__((noinline)) float my_nextupf(float x)
{
    int hx = (int)bits(x);
    int ix = hx & 0x7fffffff;
    float ax = wrap_fabsf(x); /* keeps the word's producer in the FP domain */
    if (ix == 0)
        return 1.4e-45f;
    if (ix > 0x7f800000)
        return x + x;
    /* BitTest shape: sign bit through a bit-punned word. */
    if (!((u32)hx >> 31 & 1u)) {
        if (ix == 0x7f800000)
            return x;
        hx += 1;
    } else {
        hx -= 1;
    }
    /* Extra FP pressure so the punned word competes with live floats. */
    float guard = ax * 2.0f + wrap_fabsf(x + 1.0f);
    if (guard < 0.0f)
        return guard; /* never taken; keeps guard live across the ALU ops */
    return fromb((u32)hx);
}

int main(void)
{
    int ok = my_nextupf(1.0f) > 1.0f;
    ok &= my_nextupf(0.0f) > 0.0f;
    ok &= my_nextupf(-1.0f) > -1.0f;
    float inf = __builtin_inff();
    ok &= my_nextupf(inf) == inf;
    printf("xmmint:%s\n", ok ? "ok" : "MISMATCH");
    return ok ? 0 : 1;
}
