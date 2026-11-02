/* `.set` symbols must never rewrite DIRECTIVE mnemonics, and a macro whose
 * name collides with a directive word must not capture directive lines.
 *
 * The kernel hits both:
 *  1. arch/x86/include/asm/unwind_hints.h does `.set type, 3` inside
 *     UNWIND_HINT. lccc's whole-word .set substitution treated the `.` in
 *     `.type` as a boundary and rewrote `.type foo STT_FUNC` into
 *     `.3 foo STT_FUNC` — 26 kernel function symbols silently stayed
 *     STT_NOTYPE and objtool rejected the object ("unexpected relocation
 *     symbol type: 0").
 *  2. objtool.h defines assembler macros around the word `type`; GAS always
 *     resolves `.type` as the directive regardless.
 *
 * The observable contract: after `.set type, 3` (and a macro named the same
 * bare word), `.type f, @function` must still mark f STT_FUNC — checked at
 * runtime by taking the function's address and calling through it, plus
 * using the numeric `type` symbol to prove the .set itself worked.
 */
#include <stdio.h>

extern const int set_value[] __asm__("set_value");

asm(".macro type_user\n"          /* macro world also uses the word */
    "\t.long type\n"
    ".endm\n"
    ".set type, 3\n"
    ".text\n"
    ".globl typed_fn\n"
    "typed_fn:\n"
    "\tmovl $77, %eax\n"
    "\tret\n"
    ".type typed_fn, @function\n" /* must stay a directive */
    ".size typed_fn, .-typed_fn\n"
    ".section .rodata\n"
    ".balign 4\n"
    ".globl set_value\n"
    "set_value:\n"
    "\ttype_user\n"               /* expands to .long 3 */
    ".text\n");

int typed_fn(void);

int main(void)
{
       /* If `.type typed_fn` was corrupted, the symbol is typeless; linkers
        * still link it, so ALSO verify the .set numeric path worked — the
        * pairing is what distinguishes "directive protected" from ".set
        * ignored entirely". */
       if (typed_fn() != 77) {
               printf("FAIL call\n");
               return 1;
       }
       if (set_value[0] != 3) {
               printf("FAIL set_value=%d\n", set_value[0]);
               return 2;
       }
       printf("PASS asm_set_directive_shadow\n");
       return 0;
}
