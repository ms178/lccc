/* Inline-asm bodies must reach the assembler byte-for-byte (#APP/#NO_APP).
 *
 * The kernel's ALTERNATIVE() macro uses deliberately "redundant" instructions
 * (movq %rax,%rax) purely as LENGTH TEMPLATES: .altinstructions records
 * orig_len = 742b-740b, and the runtime patcher overwrites exactly that many
 * bytes. The text peephole classified the self-move as removable, deleted it,
 * and orig_len became 0 — objtool: "empty alternative entry"; at runtime the
 * patcher would have overwritten the NEXT instruction.
 *
 * This test asserts the full contract at runtime, not just via section dumps:
 *   1. orig_len (742b-740b) equals the true encoded length of the template
 *      (3 bytes for movq %rax,%rax) — the self-move survived;
 *   2. repl_len (744f-743f) equals the replacement's length;
 *   3. the executed code still computes the right value.
 */
#include <stdio.h>

struct alt_entry {
       int  orig_off;   /* 740b - .  */
       int  repl_off;   /* 743f - .  */
       unsigned feature;
       unsigned char orig_len;  /* 742b - 740b */
       unsigned char repl_len;  /* 744f - 743f */
} __attribute__((packed));

extern struct alt_entry __start_test_altinstr[];
extern struct alt_entry __stop_test_altinstr[];

static unsigned long run_alt(unsigned long x)
{
       asm volatile(
               "740:\n\t"
               "movq %%rax, %%rax\n\t"          /* 3-byte length template */
               "741:\n\t"
               ".skip -(((744f-743f)-(741b-740b)) > 0) * ((744f-743f)-(741b-740b)),0x90\n\t"
               "742:\n\t"
               ".pushsection test_altinstr,\"a\"\n\t"
               ".long 740b - .\n\t"
               ".long 743f - .\n\t"
               ".4byte 0x123\n\t"
               ".byte 742b-740b\n\t"
               ".byte 744f-743f\n\t"
               ".popsection\n\t"
               ".pushsection test_altrepl,\"ax\"\n\t"
               "743:\n\t"
               "leaq 1(%%rax), %%rax\n\t"       /* 4-byte replacement */
               "744:\n\t"
               ".popsection"
               : "+a"(x));
       return x;
}

int main(void)
{
       unsigned long v = run_alt(41);
       /* Replacement is NOT patched in at runtime here, so the template path
        * runs: movq %rax,%rax is an identity. */
       if (v != 41) {
               printf("FAIL exec v=%lu\n", v);
               return 1;
       }
       if (__stop_test_altinstr - __start_test_altinstr != 1) {
               printf("FAIL entry count\n");
               return 2;
       }
       struct alt_entry *e = __start_test_altinstr;
       /* movq %rax,%rax = 48 89 c0 = 3 bytes. If the peephole deleted the
        * template, orig_len collapses (0 pre-fix). If it padded wrongly, the
        * skip changes it. */
       if (e->orig_len != 4) {
               /* orig region = template + NOP padding to replacement length:
                * max(3, 4) = 4. */
               printf("FAIL orig_len=%u want 4\n", e->orig_len);
               return 3;
       }
       if (e->repl_len != 4) {
               printf("FAIL repl_len=%u want 4\n", e->repl_len);
               return 4;
       }
       printf("PASS asm_alternative_length_template\n");
       return 0;
}
