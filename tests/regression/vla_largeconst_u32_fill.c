/* MachInst large-immediate narrow-store width regression.
 *
 * A VLA element fill that stores unsigned 32-bit values OUTSIDE the signed
 * 32-bit range (b[i] = 3041712678u + i > INT32_MAX) lowers on the MachInst
 * path to `Mov { src: Imm(i64), dst: mem, size: S32 }`. The emitter used to
 * take the "large immediate (>i32) to memory" branch and emit
 *
 *     movabsq $3041712678, %rax
 *     movq    %rax, (%rcx)          <- 8-byte store into a 4-byte element
 *
 * The final element of a 4-byte-strided VLA therefore overran its allocation
 * by 4 bytes and clobbered the adjacent stack data — the VLA fill below
 * `a[n]` smashed a[0]'s low word (a[0] printed wrong), and in a minimal
 * single-VLA variant the overrun hit the saved VLA base pointer (SIGSEGV).
 *
 * Correct lowering (the mov{b,w,l} immediate field is a raw {8,16,32}-bit
 * value, no sign-extension) is a single sized store:
 *
 *     movl $3041712678, (%rax)
 *
 * Reproducer that FAILS pre-fix (wrong a[0]) and matches gcc post-fix.
 */
#include <stdio.h>

__attribute__((noinline)) static void barrier(void) { asm volatile("" ::: "memory"); }

int main(void) {
    unsigned n = 4;
    unsigned long long a[n]; /* 8-byte elements, allocated just above b       */
    unsigned int b[n];       /* 4-byte elements with values > INT32_MAX       */
    for (unsigned i = 0; i < n; i++) {
        a[i] = 5ull * 0x9e3779b97f4a7c15ULL + 7ULL + i;
        b[i] = 6u * 2654435761u + i; /* 3041712678..3041712681 */
    }
    barrier(); /* force real memory: kill any register promotion */

    /* Re-read everything.  Pre-fix the b[3] 8-byte store had zeroed a[0]'s
     * low 32 bits (adjacent VLA above), so a0 (and the checksum) mismatch. */
    unsigned long long acc = 0;
    for (unsigned i = 0; i < n; i++) {
        acc = acc * 33 + a[i];
        acc = acc * 33 + (unsigned long long)b[i];
    }
    unsigned long long a0 = a[0];
    unsigned int b3 = b[3];
    printf("vla_largeconst acc=%llu a0=%llu b3=%u\n", acc, a0, b3);
    return (acc == 0) ? 1 : 0;
}
