/* .macro parameter defaults containing parentheses, spaces and commas.
 *
 * The kernel's FILL_RETURN_BUFFER (arch/x86/include/asm/nospec-branch.h):
 *     .macro FILL_RETURN_BUFFER reg:req nr:req ftr:req ftr2=ALT_NOT(...)
 * After cpp, the default is `(((1 << 0) << 16) | (( 3*32+21)))` — spaces and
 * nested parens included. Three separate parser bugs corrupted it:
 *   1. whitespace-splitting the parameter list shredded the default into
 *      bogus parameters named `<<` and `0)`;
 *   2. the `:req` qualifier was kept in the parameter NAME, so `\reg` never
 *      substituted;
 *   3. a quoted macro argument containing `;` spliced several statements
 *      into one line, and macro invocations after the `;` (the kernel's
 *      ANNOTATE) were never expanded.
 * All three surface here: the macro must expand, its parameters must bind,
 * the parenthesized default must survive into `.long`, and the semicolon-
 * separated inner invocation must also expand.
 */
#include <stdio.h>

extern const int test_vals[] __asm__("test_vals");

asm(".macro EMIT val:req\n"
    "\t.long \\val\n"
    ".endm\n"
    ".macro OUTER reg:req nr:req flags=(((1 << 4) | ( 3*32+21)))\n"
    "\tEMIT val=\\nr ; EMIT val=\\flags\n"
    ".endm\n"
    ".section .rodata\n"
    ".balign 4\n"
    ".globl test_vals\n"
    "test_vals:\n"
    "\tOUTER reg=%rax, nr=7\n"                 /* default flags */
    "\tOUTER reg=%rbx, nr=9, flags=(2 | (1 << 8))\n"  /* explicit flags */
    ".text\n");

int main(void)
{
       /* (((1 << 4) | ( 3*32+21))): 3*32+21 = 117 = 0x75 already has bit 4
        * set, so 117 | 16 = 117. The interesting property is that the WHOLE
        * parenthesized expression survived as one default value. */
       if (test_vals[0] != 7) {
               printf("FAIL nr=%d\n", test_vals[0]);
               return 1;
       }
       if (test_vals[1] != (((1 << 4) | (3*32+21)))) {
               printf("FAIL default flags=%d want %d\n", test_vals[1],
                      (((1 << 4) | (3*32+21))));
               return 2;
       }
       if (test_vals[2] != 9) {
               printf("FAIL nr2=%d\n", test_vals[2]);
               return 3;
       }
       if (test_vals[3] != (2 | (1 << 8))) {
               printf("FAIL explicit flags=%d\n", test_vals[3]);
               return 4;
       }
       printf("PASS asm_macro_paren_default\n");
       return 0;
}
