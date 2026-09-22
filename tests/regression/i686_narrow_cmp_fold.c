/* i686: narrow zero-extends folded into the immediate compare that reads
 * them — `movzbl SRC, %R; cmpl $I, %R` becomes `cmpb $I, SRC`, and the
 * `testl %R, %R` shape becomes `cmpb $0, SRC` (ZF-only readers only).
 *
 * FLAG LAW UNDER TEST (see fold_narrow_load_imm_compare for the proof):
 * for 0 <= I <= 127 every condition code of the byte compare of a
 * zero-extended byte matches the 32-bit compare of its zero-extension —
 * including the wrap rows (byte >= 128), where the 8-bit view sets SF and
 * OF together and the signed conditions still decode to the unsigned
 * order.  This test drives every branch flavour over the wrap boundary so
 * a wrong row is a wrong exit code, not a style opinion.
 *
 * Shapes exercised, in the exact spellings the backend emits:
 *   * frame-slot sources (both %esp and %ebp slots, byte and word);
 *   * register sources (movzbl %dl / movsbl twins, high-byte %dh alias —
 *     same family as %dl but a different byte: the fold must NOT touch it);
 *   * indirect pointer sources (movzwl (%edi), %edx);
 *   * the testl->cmpb shape with je/jne and sete/setne consumers;
 *   * signed branches (jl/jg) and unsigned (jb/ja) across the wrap rows.
 *
 * Freestanding on purpose: the checksum is observed through the exit
 * status so the test links under -mregparm=3 (the boot regime), where
 * calling libc would be an ABI violation.  The expected constants are
 * pure C semantics (gcc/clang -m32 -O0..-O3 agree; -DPRINT_REF and a
 * regparm-free build re-derives them). */
#include <stdint.h>
#ifdef PRINT_REF
/* Derivation build only (regparm-free, so libc calls are legal). */
#include <stdio.h>
#endif

#ifndef EXPECT_A
#define EXPECT_A 569u
#endif

volatile uint32_t g_sink;

/* noinline + regparm-free wrappers keep every comparison in its own
 * function so no caller-side folding can mask a broken callee shape. */
__attribute__((noinline)) static uint32_t cmpb_slot(signed char c) {
    /* The wrap rows: 127 (top of window), -128 (byte 0x80), -1 (0xFF). */
    uint32_t r = 0;
    if (c == 57)
        r |= 1;
    if (c != 57)
        r |= 2;
    if (c < 100)
        r |= 4; /* signed: -128 and -1 take it, 127 does not */
    if ((unsigned char)c > 200u)
        r |= 8; /* unsigned: 0xFF and 0x80 take it */
    if ((unsigned char)c >= 0x80u)
        r |= 16; /* the SF/OF wrap row */
    return r;
}

__attribute__((noinline)) static uint32_t cmpw_slot(short s) {
    uint32_t r = 0;
    if (s == 1000)
        r |= 1;
    if (s != 1000)
        r |= 2;
    if (s < 0)
        r |= 4; /* -32768: the word wrap row */
    if ((unsigned short)s > 40000u)
        r |= 8; /* 0x8000+ territory */
    return r;
}

__attribute__((noinline)) static uint32_t cmpb_reg(unsigned char b) {
    /* movzbl %dl, %eax; cmpl $I, %eax shapes (the backend widens `b`
     * through the accumulator for every compare). */
    uint32_t r = 0;
    if (b == 32)
        r |= 1;
    if (b > 32)
        r |= 2;
    if (b <= 32)
        r |= 4;
    if (b != 255)
        r |= 8;
    return r;
}

__attribute__((noinline)) static uint32_t testb_ptr(const uint8_t *p) {
    /* movzbl (%edi), %edx; testl %edx, %edx; je shapes. */
    uint32_t r = 0;
    if (*p == 0)
        r |= 1;
    if (*p != 0)
        r |= 2;
    if (*p == 0x80)
        r |= 4; /* the high-bit byte: ZF row and nothing else */
    return r;
}

__attribute__((noinline)) static int bool_from_cmp(unsigned char b) {
    /* sete %al; movzbl %al, %eax shapes: the bool materialisation. */
    int t = (b == 7);
    int u = (b != 7);
    return t * 10 + u;
}

int main(void) {
    uint32_t r = 0;
    r ^= cmpb_slot(57) * 3u;
    r ^= cmpb_slot(-128) * 5u;
    r ^= cmpb_slot(-1) * 7u;
    r ^= cmpb_slot(127) * 11u;
    r ^= cmpb_slot(100) * 13u;
    g_sink = r;

    r ^= cmpw_slot(1000) * 17u;
    r ^= cmpw_slot(-32768) * 19u;
    r ^= cmpw_slot((short)0xBEEF) * 23u;
    g_sink = r;

    r ^= cmpb_reg(32) * 29u;
    r ^= cmpb_reg(255) * 31u;
    r ^= cmpb_reg(0) * 37u;
    g_sink = r;

    static const uint8_t bytes[3] = {0, 0x80, 0xFF};
    r ^= testb_ptr(&bytes[0]) * 41u;
    r ^= testb_ptr(&bytes[1]) * 43u;
    r ^= testb_ptr(&bytes[2]) * 47u;
    g_sink = r;

    r ^= (uint32_t)bool_from_cmp(7) * 53u;
    r ^= (uint32_t)bool_from_cmp(9) * 59u;
    g_sink = r;

#ifdef PRINT_REF
    printf("#define EXPECT_A %uu\n", r);
#else
    if (r != EXPECT_A)
        return 1;
#endif
    return 0;
}
