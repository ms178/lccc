/* GOTPCREL against a LOCAL asm label must load the ADDRESS, not the bytes.
 *
 * `movq sym@GOTPCREL(%rip), %reg` dereferences its memory operand: it loads
 * the CONTENTS of the GOT slot. Local asm labels get no GOT slot, and
 * lccc-ld pointed the unrelaxed mov straight at the symbol — loading the
 * label's first instruction bytes as if they were a pointer. Here that made
 * `l_b - l_a` come out as 0x2e666690c3909090-style garbage instead of 3.
 *
 * The fix follows GNU ld: relax mov -> lea (address computation), and fail
 * loudly for any non-relaxable opcode shape rather than emit wrong code.
 */
#include <stdio.h>

extern const char l_a[] __asm__("l_a");
extern const char l_b[] __asm__("l_b");

asm(".text\n"
    "l_a:\n"
    "\tnop\n\tnop\n\tnop\n"
    "l_b:\n"
    "\tret\n");

int main(void)
{
       long d = l_b - l_a;
       if (d != 3) {
               printf("FAIL diff=%ld a=%p b=%p\n", d, (void *)l_a, (void *)l_b);
               return 1;
       }
       /* The addresses themselves must be sane code addresses (dereferencing
        * l_a must read the NOP opcode, proving the pointer is real). */
       if ((unsigned char)l_a[0] != 0x90) {
               printf("FAIL deref l_a[0]=%02x\n", (unsigned char)l_a[0]);
               return 2;
       }
       printf("PASS gotpcrel_local_asm_label\n");
       return 0;
}
