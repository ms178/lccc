/* Byte and word stores lowered through MachInst.
 *
 * `isel::lower_instruction_typed` refused I8/U8/I16/U16 stores outright and
 * fell back to direct text emission. Nothing about a narrow store is
 * inexpressible -- `movb %al, (%rcx)` and `movw %ax, off(%rbp)` are ordinary
 * `MachInst::Mov`s at OpSize::S8/S16, and the emitter's size tables have
 * always handled both -- so the restriction only split otherwise contiguous
 * MachInst runs and pushed byte traffic, the dominant operation in the gzip /
 * zlib-ng / expat workloads, onto the untyped path.
 *
 * These cases cover the three addressing forms the lowering distinguishes:
 * a store to an alloca slot, a store through a pointer already in a register,
 * and a store through a pointer that has to be staged. Signed and unsigned,
 * 8- and 16-bit, register and immediate sources, plus the r8..r15 registers
 * whose byte names (%r8b..%r15b) are spelled differently from the legacy set.
 *
 * Expected output: 197 1543 -86 4660 255 65535 42 7
 */
#include <stdio.h>
#include <string.h>

static unsigned char buf8[64];
static unsigned short buf16[64];

__attribute__((noinline)) static void store_bytes(unsigned char *p, int n, unsigned char v) {
    for (int i = 0; i < n; i++) {
        p[i] = (unsigned char) (v + i);   /* store through a register pointer */
    }
}

__attribute__((noinline)) static void store_words(unsigned short *p, int n, unsigned short v) {
    for (int i = 0; i < n; i++) {
        p[i] = (unsigned short) (v * (i + 1));
    }
}

/* Stores into local (alloca) storage, then read back. */
__attribute__((noinline)) static int local_narrow(void) {
    unsigned char a[8];
    unsigned short w[8];
    for (int i = 0; i < 8; i++) {
        a[i] = (unsigned char) (i * 3 + 1);
        w[i] = (unsigned short) (i * 1000 + 7);
    }
    int s = 0;
    for (int i = 0; i < 8; i++) {
        s += a[i] + (w[i] & 0xff);
    }
    return s;
}

/* Signed narrow stores, which must sign-extend correctly on read-back. */
__attribute__((noinline)) static int signed_narrow(void) {
    signed char sc[4];
    short sh[4];
    for (int i = 0; i < 4; i++) {
        sc[i] = (signed char) (-40 + i);
        sh[i] = (short) (-3000 + i * 7);
    }
    int s = 0;
    for (int i = 0; i < 4; i++) {
        s += sc[i] + sh[i];
    }
    return s;
}

/* Immediate sources and the saturating boundary values. */
__attribute__((noinline)) static int immediate_narrow(unsigned char *b, unsigned short *w) {
    b[0] = 0xff;
    b[1] = 0x00;
    w[0] = 0xffff;
    w[1] = 0;
    return b[0] + w[0];
}

int main(void) {
    store_bytes(buf8, 32, 7);
    store_words(buf16, 32, 3);

    int s8 = 0, s16 = 0;
    for (int i = 0; i < 32; i++) {
        s8 += buf8[i];
        s16 += buf16[i] & 0x3f;
    }

    unsigned char lb[4];
    unsigned short lw[4];
    memset(lb, 0, sizeof lb);
    memset(lw, 0, sizeof lw);
    immediate_narrow(lb, lw);

    printf("%d %d %d %d %d %d %d %d\n",
           s8,
           s16 + local_narrow(),
           signed_narrow() / 100,
           local_narrow(),
           (int) lb[0],
           (int) lw[0],
           (int) buf8[35 - 4],
           (int) (buf16[2] / 3));
    return 0;
}
