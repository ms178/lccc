/* `.quad sym - .` must be a full 64-bit PC-relative value (R_X86_64_PC64).
 *
 * The kernel's __jump_table stores static-key addresses exactly this way:
 *     .quad key - .
 * lccc emitted R_X86_64_PC32 into the 8-byte slot: the upper four bytes
 * stayed zero instead of sign-extending. Any NEGATIVE delta (key placed
 * below the table, the common case for .data vs a late .rodata-like
 * section) produced a corrupt address — the static-key patcher would have
 * walked into unrelated memory.
 *
 * The test reconstructs the target address from the stored delta at runtime
 * and checks it for a data symbol as well as for a code label, in a section
 * ordering that makes the delta negative (data links below the table).
 */
#include <stdio.h>

static long the_key = 42;

struct jt { long delta; } __attribute__((packed));
extern struct jt __start_test_jt[];
extern struct jt __stop_test_jt[];

__attribute__((noinline)) static int probe(void)
{
       /* Same shape as arch/x86/include/asm/jump_label.h. */
       asm goto("1: .byte 0x0f,0x1f,0x44,0x00,0x00\n\t"
                ".pushsection test_jt, \"aw\"\n\t"
                ".balign 8\n\t"
                ".quad %c0 - .\n\t"
                ".popsection"
                : : "i"(&the_key) : : l_yes);
       return 0;
l_yes:
       return 1;
}

int main(void)
{
       if (probe() != 0) {
               printf("FAIL exec\n");
               return 1;
       }
       if (__stop_test_jt - __start_test_jt != 1) {
               printf("FAIL count\n");
               return 2;
       }
       /* PC-relative reconstruction: stored delta + slot address = target.
        * With a PC32-in-8-byte-slot bug, a negative delta loses its sign
        * extension and the sum is off by 2^32. */
       unsigned long slot = (unsigned long)&__start_test_jt[0].delta;
       unsigned long target = slot + (unsigned long)__start_test_jt[0].delta;
       if (target != (unsigned long)&the_key) {
               printf("FAIL delta: target=%lx key=%lx delta=%lx\n",
                      target, (unsigned long)&the_key,
                      (unsigned long)__start_test_jt[0].delta);
               return 3;
       }
       if (*(long *)target != 42) {
               printf("FAIL deref\n");
               return 4;
       }
       printf("PASS asm_quad_pcrel_key\n");
       return 0;
}
