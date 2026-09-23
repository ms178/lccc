/* i686: SF/OF flag law at the narrow-compare fold boundary.
 *
 * The narrow-compare fold (fold_narrow_load_imm_compare) rewrites
 * `movzbl SLOT, %R; cmpl $I, %R` into `cmpb $I, SLOT`.  The two views agree
 * on ZF/CF for every condition code, but DISAGREE on SF/OF once the value
 * crosses the narrow range (byte >= 128, word >= 32768): the narrow
 * subtraction wraps or sign-crosses where the 32-bit one does not
 * (byte 200 vs I 50: wide 150 -> SF=0 OF=0; narrow 0x96 -> SF=1 OF=0).
 *
 * The historical miscompile (upstream PR #593, fixed by the flag-reader
 * window in this pass): the fold fired under ANY consumer, so
 *
 *     movzbl 16(%esp), %eax
 *     cmpl   $50, %eax
 *     setg   %al
 *
 * became `cmpb $50, %al; setg %al` — for c = 200 the wide view says
 * 200 > 50 (true) while the narrow flags give setg = 0.  The C shape is
 * any promoted unsigned-char relational: `unsigned char c > 50` compiles
 * to a SIGNED int compare (both operands non-negative) over a
 * ZERO-EXTENDED operand — exactly the operand form the fold targets with
 * exactly the consumer (Sgt -> jg/setg) the narrow view gets wrong.
 *
 * Every probe below takes its value as a PARAMETER (noinline) so the
 * comparison lives in the callee and cannot be constant-folded by the
 * caller: a zero-extended u8/u16 operand under a SIGNED ordering
 * comparison with an immediate inside the fold window, driven from both
 * sides of the wrap boundary.  A broken flag row is a wrong checksum,
 * i.e. a wrong exit status.  The *_eq/*_ult/*_uge/*_ne controls are
 * ZF/CF readers: legal under the fold at every value, so this file also
 * pins that the fold's window does not over-reject the safe codes.
 *
 * Freestanding on purpose: the checksum is observed through the exit
 * status so the test links under -mregparm=3 (the boot regime), where
 * calling libc would be an ABI violation.  The expected constant is pure
 * C semantics (gcc/clang -m32 agree; -DPRINT_REF and a regparm-free
 * build re-derives it). */
#include <stdint.h>
#ifdef PRINT_REF
#include <stdio.h>
#endif

#ifndef EXPECT_A
#define EXPECT_A 3813516108u
#endif

volatile uint32_t g_sink;

__attribute__((noinline)) static uint32_t u8_sgt(uint32_t c) { return (uint8_t)c > 50; }
__attribute__((noinline)) static uint32_t u8_slt(uint32_t c) { return (uint8_t)c < 100; }
__attribute__((noinline)) static uint32_t u8_sge(uint32_t c) { return (uint8_t)c >= 100; }
__attribute__((noinline)) static uint32_t u8_sle(uint32_t c) { return (uint8_t)c <= 100; }
__attribute__((noinline)) static uint32_t u16_sgt(uint32_t c) { return (uint16_t)c > 50; }
__attribute__((noinline)) static uint32_t u16_slt(uint32_t c) { return (uint16_t)c < 30000; }
/* ZF/CF controls (fold-safe at every value). */
__attribute__((noinline)) static uint32_t u8_eq(uint32_t c) { return (uint8_t)c == 200; }
__attribute__((noinline)) static uint32_t u8_ne(uint32_t c) { return (uint8_t)c != 50; }
__attribute__((noinline)) static uint32_t u8_ult(uint32_t c) { return (uint8_t)c < 50; }
__attribute__((noinline)) static uint32_t u8_uge(uint32_t c) { return (uint8_t)c >= 200; }

static uint32_t acc(uint32_t r, uint32_t v) { return r * 31u + v; }

/* The arguments ride in a volatile array so every call takes a RUNTIME
 * value: a static callee with constant arguments is a legitimate target
 * for call-folding, which would optimise the comparisons out of existence
 * and make this test vacuous.  Volatile reads defeat that while keeping
 * the callee's own codegen — the shape under test — fully live. */
static volatile uint32_t vals[] = {
    200, 30, 50, 150, 150, 50, 250, 200, 50, 100, 150, 50, 250,
    40000, 30, 33000, 40000, 20000,
    200, 50, 40, 200, 200, 199, 50, 200
};

int main(void) {
    uint32_t r = 7u;
    /* signed orderings over zero-extended bytes, across the wrap */
    r = acc(r, u8_sgt(vals[0]));  /* 200 -> 1 */
    r = acc(r, u8_sgt(vals[1]));  /* 30 -> 0 */
    r = acc(r, u8_sgt(vals[2]));  /* 50 -> 0 */
    r = acc(r, u8_sgt(vals[3]));  /* 150 -> 1 */
    r = acc(r, u8_slt(vals[4]));  /* 150 -> 0 */
    r = acc(r, u8_slt(vals[5]));  /* 50 -> 1 */
    r = acc(r, u8_slt(vals[6]));  /* (u8)250 = 94 < 100 -> 1 */
    r = acc(r, u8_sge(vals[7]));  /* 200 -> 1 */
    r = acc(r, u8_sge(vals[8]));  /* 50 -> 0 */
    r = acc(r, u8_sge(vals[9]));  /* 100 -> 1 */
    r = acc(r, u8_sle(vals[10])); /* 150 -> 0 */
    r = acc(r, u8_sle(vals[11])); /* 50 -> 1 */
    r = acc(r, u8_sle(vals[12])); /* 94 <= 100 -> 1 */
    /* word twin, across 32768 */
    r = acc(r, u16_sgt(vals[13])); /* 40000 -> 1 */
    r = acc(r, u16_sgt(vals[14])); /* 30 -> 0 */
    r = acc(r, u16_sgt(vals[15])); /* 33000 -> 1 */
    r = acc(r, u16_slt(vals[16])); /* 40000 -> 0 */
    r = acc(r, u16_slt(vals[17])); /* 20000 -> 1 */
    /* ZF/CF controls */
    r = acc(r, u8_eq(vals[18]));   /* 200 == 200 -> 1 */
    r = acc(r, u8_eq(vals[19]));   /* 50 == 200 -> 0 */
    r = acc(r, u8_ult(vals[20]));  /* 40 < 50 -> 1 */
    r = acc(r, u8_ult(vals[21]));  /* 200 < 50 -> 0 */
    r = acc(r, u8_uge(vals[22]));  /* 200 >= 200 -> 1 */
    r = acc(r, u8_uge(vals[23]));  /* 199 >= 200 -> 0 */
    r = acc(r, u8_ne(vals[24]));   /* 50 != 50 -> 0 */
    r = acc(r, u8_ne(vals[25]));   /* 200 != 50 -> 1 */
    g_sink = r;
#ifdef PRINT_REF
    printf("%u\n", r);
#endif
    return r == EXPECT_A ? 0 : 1;
}
