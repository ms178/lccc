/* GAS `\@` — the macro invocation counter — must be unique per expansion.
 *
 * `\@` is only substituted inside `.macro` bodies, so this test defines an
 * ANNOTATE-shaped macro in a top-level asm block (the same path a kernel .S
 * file takes) and expands it three times. lccc left `\@` unsubstituted, so
 * every expansion defined and referenced the SAME `.Lhere_\@` label: all
 * .discard.annotate_insn entries pointed at one address (objtool:
 * "intra_function_call not a direct call") instead of at the instruction
 * each one annotates.
 *
 * The test asserts the recorded self-relative offsets resolve to pairwise
 * distinct, strictly increasing addresses — i.e. each expansion got its own
 * label.
 */
#include <stdio.h>

struct ann { int off; int type; } __attribute__((packed));
extern struct ann __start_test_annotate[];
extern struct ann __stop_test_annotate[];

asm(".macro TESTANNOTATE type:req\n"
    ".Lhere_\\@:\n"
    "\t.pushsection test_annotate,\"a\"\n"
    "\t.long .Lhere_\\@ - .\n"
    "\t.long \\type\n"
    "\t.popsection\n"
    ".endm\n"
    ".text\n"
    ".globl annotated_sites\n"
    ".type annotated_sites, @function\n"
    "annotated_sites:\n"
    "\tmovl $0, %eax\n"
    "\tTESTANNOTATE type=7\n"
    "\taddl $1, %eax\n"
    "\tTESTANNOTATE type=7\n"
    "\taddl $2, %eax\n"
    "\tTESTANNOTATE type=9\n"
    "\tret\n"
    ".size annotated_sites, .-annotated_sites\n");

int annotated_sites(void);

int main(void)
{
       if (annotated_sites() != 3) {
               printf("FAIL exec\n");
               return 1;
       }
       long n = __stop_test_annotate - __start_test_annotate;
       if (n != 3) {
               printf("FAIL count=%ld\n", n);
               return 2;
       }
       /* Recover each annotated address: entry address + recorded offset. */
       unsigned long a0 = (unsigned long)&__start_test_annotate[0] + __start_test_annotate[0].off;
       unsigned long a1 = (unsigned long)&__start_test_annotate[1] + __start_test_annotate[1].off;
       unsigned long a2 = (unsigned long)&__start_test_annotate[2] + __start_test_annotate[2].off;
       if (a0 == a1 || a1 == a2 || a0 == a2) {
               printf("FAIL collapsed labels %lx %lx %lx\n", a0, a1, a2);
               return 3;
       }
       if (!(a0 < a1 && a1 < a2)) {
               printf("FAIL order %lx %lx %lx\n", a0, a1, a2);
               return 4;
       }
       if (__start_test_annotate[2].type != 9) {
               printf("FAIL type\n");
               return 5;
       }
       printf("PASS asm_macro_at_counter\n");
       return 0;
}
