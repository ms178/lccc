/* `.skip` sizing vs jump relaxation: the fixed point must match GNU as.
 *
 * Two kernel shapes pull the ordering in OPPOSITE directions:
 *  (a) sev.o: a jmp hops OVER two ALTERNATIVE paddings — if displacements
 *      are patched before the skips are sized, the branch lands
 *      mid-instruction.
 *  (b) retpoline.S FILL_RETURN_BUFFER: the padded region ITSELF contains a
 *      relaxable jmp — if the skip is sized against the long form, the
 *      region over-pads by a byte and the recorded orig_len describes code
 *      that no longer exists (objtool: "weirdly overlapping alternative").
 *
 * Both shapes are encoded here and verified at RUNTIME: (a) by executing
 * across the padded region, (b) by checking orig_len against the distance
 * between the region's boundary symbols after final layout.
 */
#include <stdio.h>

struct alt_e {
       int orig_off; int repl_off; unsigned feat;
       unsigned char orig_len; unsigned char repl_len;
} __attribute__((packed));

extern struct alt_e __start_talt[];
extern struct alt_e __stop_talt[];
extern const char t_740[] __asm__("t_740");
extern const char t_742[] __asm__("t_742");

/* Shape (b): oldinstr is a RELAXABLE jmp to a nearby label. */
asm(".text\n"
    ".globl shape_b\n"
    ".type shape_b, @function\n"
    "shape_b:\n"
    "t_740:\n"
    "\tjmp .Lskip_b\n"                 /* relaxes 5 -> 2 bytes */
    "741:\n"
    "\t.skip -(((744f-743f)-(741b-t_740)) > 0) * ((744f-743f)-(741b-t_740)),0x90\n"
    "t_742:\n"
    "\t.pushsection talt,\"a\"\n"
    "\t.long t_740 - .\n"
    "\t.long 743f - .\n"
    "\t.4byte 0x22\n"
    "\t.byte t_742-t_740\n"
    "\t.byte 744f-743f\n"
    "\t.popsection\n"
    "\t.pushsection talt_repl,\"ax\"\n"
    "743:\n"
    "\tmovl $1, %eax\n"                /* 5-byte replacement */
    "\tint3\n"                          /* 6th byte: force padding */
    "744:\n"
    "\t.popsection\n"
    ".Lskip_b:\n"
    "\tmovl $5, %eax\n"
    "\tret\n"
    ".size shape_b, .-shape_b\n");

int shape_b(void);

/* Shape (a): jmp across two padded regions. */
__attribute__((noinline)) static int shape_a(int sel)
{
       int r = 0;
       if (sel != 1)
               goto out;                       /* branch across the asm */
       asm volatile(
               "7401:\n\t"
               "movq %%rax, %%rax\n\t"
               "7411:\n\t"
               ".skip -(((7441f-7431f)-(7411b-7401b)) > 0) * ((7441f-7431f)-(7411b-7401b)),0x90\n\t"
               ".pushsection talt_repl,\"ax\"\n\t"
               "7431:\n\t"
               "movq %%rcx, %%rcx; movq %%rdx, %%rdx; movq %%rsi, %%rsi\n\t"
               "7441:\n\t"
               ".popsection\n\t"
               "7402:\n\t"
               "movq %%rbx, %%rbx\n\t"
               "7412:\n\t"
               ".skip -(((7442f-7432f)-(7412b-7402b)) > 0) * ((7442f-7432f)-(7412b-7402b)),0x90\n\t"
               ".pushsection talt_repl,\"ax\"\n\t"
               "7432:\n\t"
               "movq %%r8, %%r8; movq %%r9, %%r9\n\t"
               "7442:\n\t"
               ".popsection"
               ::: "memory");
       r = 100;
out:
       return r + sel;
}

int main(void)
{
       /* (a) both paths execute correctly across the padding. */
       if (shape_a(0) != 0 || shape_a(1) != 101 || shape_a(2) != 2) {
               printf("FAIL shape_a\n");
               return 1;
       }
       /* (b) the function still works... */
       if (shape_b() != 5) {
               printf("FAIL shape_b exec\n");
               return 2;
       }
       /* ...and orig_len equals the REAL distance t_742 - t_740 after all
        * relaxation: a stale pre-relaxation size differs by the byte the
        * jmp shrank. */
       if (__stop_talt - __start_talt != 1) {
               printf("FAIL count\n");
               return 3;
       }
       long real = t_742 - t_740;
       if (__start_talt[0].orig_len != real) {
               printf("FAIL orig_len=%u real=%ld\n",
                      __start_talt[0].orig_len, real);
               return 4;
       }
       printf("PASS asm_alt_skip_relax_order\n");
       return 0;
}
