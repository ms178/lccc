/* Layout invariants of the kernel's ALTERNATIVE machinery.
 *
 * `.skip` padding, `.p2align`, jump relaxation and `.byte`/`.word` symbol
 * differences are all MUTUALLY dependent, and getting the order wrong produces
 * objects that assemble without a diagnostic but are rejected by objtool -- or
 * worse, execute garbage. Four distinct ordering bugs were found here:
 *
 *   1. `.skip` resolved AFTER jump relaxation: the padding moved targets out
 *      from under already-patched displacements, so a `jmp` landed 3 bytes
 *      early, in the MIDDLE of an instruction.
 *      -> objtool "can't find jump dest instruction"
 *   2. `.skip` sized BEFORE relaxation: locks in long displacements and
 *      over-pads, so the recorded `orig_len` describes code that is no longer
 *      there (measured 0x1b where GAS says 0x1e => orig_len < repl_len).
 *      -> objtool "weirdly overlapping alternative"
 *   3. `.p2align` markers not shifted when a `.skip` resizes: a function
 *      symbol landed unaligned (0x2043 where GAS puts 0x2050).
 *      -> objtool "can't find starting instruction"
 *   4. `.byte`/`.word` symbol differences folded before relaxation: they
 *      MEASURE code that relaxation then shrinks.
 *
 * The correct order (what GNU as does) is: relax to the smallest fixed point,
 * then size the padding, then fold the measurements.
 *
 * Everything here is checked at RUNTIME against values the assembler had to
 * compute, so a wrong layout is a test failure rather than a silent warning.
 */
#include <stdio.h>

/* Mirrors struct alt_instr's length fields. */
struct alt_len {
       unsigned char orig_len;
       unsigned char repl_len;
};

extern struct alt_len alt_a, alt_b, alt_c;
extern unsigned short jmp_field_off;
extern unsigned int region_a_len, region_b_len;

/* --- Case 1+2: a jump inside the measured region that relaxation shrinks.
 * The original is padded up to the replacement's length. orig_len must equal
 * the ACTUAL post-relaxation extent, and must be >= repl_len. */
asm(".text\n"
    ".globl probe_a\n"
    ".type probe_a, @function\n"
    "probe_a:\n"
    "740:\n"
    "  movl $0x49, %ecx\n"
    "  jmp 1f\n"                 /* long -> short after relaxation */
    "  nop;nop;nop;nop;nop;nop\n"
    "  nop;nop;nop;nop;nop;nop\n"
    "741:\n"
    "  .skip -(((744f-743f)-(741b-740b)) > 0) * ((744f-743f)-(741b-740b)),0x90\n"
    "742:\n"
    "  .pushsection .altinstr_replacement,\"ax\"\n"
    "743:\n"
    "  " /* 30 bytes of replacement */
    "nop;nop;nop;nop;nop;nop;nop;nop;nop;nop;"
    "nop;nop;nop;nop;nop;nop;nop;nop;nop;nop;"
    "nop;nop;nop;nop;nop;nop;nop;nop;nop;nop\n"
    "744:\n"
    "  .popsection\n"
    /* the two length bytes, exactly as struct alt_instr stores them.
     * Emitted INSIDE the same asm block so the numeric labels are still in
     * scope (a separate asm() statement gets a fresh label namespace). */
    "  .pushsection .data\n"
    "  .globl alt_a\n"
    "alt_a:\n"
    "  .byte 742b-740b\n"
    "  .byte 744b-743b\n"
    "  .popsection\n"
    "1:\n"
    "  ret\n"
    ".size probe_a, .-probe_a\n");

/* --- Case 3: a `.p2align 4` AFTER a resolved `.skip`. The following symbol
 * must be 16-byte aligned; a stale marker offset silently mis-pads it. */
asm(".text\n"
    "750:\n"
    "  nop;nop;nop\n"
    "751:\n"
    "  .skip -(((753f-752f)-(751b-750b)) > 0) * ((753f-752f)-(751b-750b)),0x90\n"
    "  .pushsection .altinstr_replacement,\"ax\"\n"
    "752:\n"
    "  nop;nop;nop;nop;nop;nop;nop\n"
    "753:\n"
    "  .popsection\n"
    "  .p2align 4\n"
    ".globl aligned_after_skip\n"
    ".type aligned_after_skip, @function\n"
    "aligned_after_skip:\n"
    "  xorl %eax, %eax\n"
    "  ret\n"
    ".size aligned_after_skip, .-aligned_after_skip\n");
extern void aligned_after_skip(void);

/* --- Case 4: a `.word` that MEASURES a region containing a relaxable jump,
 * plus the left-side-addend form (`label + 1 - base`) used by the kernel's
 * la57 trampoline to locate the immediate field inside a far jump. */
asm(".text\n"
    ".Lmark_a:\n"
    "  jmp 2f\n"                 /* relaxes */
    "  nop;nop;nop;nop\n"
    ".Lmark_b:\n"
    "  jmp 2f\n"                 /* relaxes */
    "  nop;nop\n"
    "2:\n"
    ".Lmark_end:\n"
    "  ret\n"
    "  .pushsection .data\n"
    "  .globl jmp_field_off\n"
    "jmp_field_off:\n"
    /* Left-side addend, exactly as arch/x86/boot/startup/la57toggle.S writes
     * it to locate the immediate field inside a far jump. NAMED labels: this
     * is the form the kernel actually uses (SYM_* macros expand to named
     * local labels), and it is what must not regress. */
    "  .word .Lmark_b + 1 - .Lmark_a\n"
    "  .globl region_a_len\n"
    "region_a_len:\n"
    "  .long .Lmark_b - .Lmark_a\n"
    "  .globl region_b_len\n"
    "region_b_len:\n"
    "  .long .Lmark_end - .Lmark_b\n"
    "  .popsection\n");

int main(void)
{
       int fail = 0;

       /* orig_len must cover the replacement: this is the invariant objtool
        * enforces as "weirdly overlapping alternative". */
       if (alt_a.orig_len < alt_a.repl_len) {
               printf("FAIL alt_a orig_len=%u < repl_len=%u\n",
                      alt_a.orig_len, alt_a.repl_len);
               fail = 1;
       }
       /* The replacement is exactly 30 nops. */
       if (alt_a.repl_len != 30) {
               printf("FAIL alt_a repl_len=%u expected 30\n", alt_a.repl_len);
               fail = 1;
       }
       /* Padding brings the original up to -- not past -- the replacement. */
       if (alt_a.orig_len != 30) {
               printf("FAIL alt_a orig_len=%u expected 30\n", alt_a.orig_len);
               fail = 1;
       }

       /* A `.p2align 4` after a resolved `.skip` must really align. */
       if (((unsigned long)(void *)aligned_after_skip & 0xf) != 0) {
               printf("FAIL aligned_after_skip=%p not 16-byte aligned\n",
                      (void *)aligned_after_skip);
               fail = 1;
       }

       /* `770b + 1 - 760b` == (770b - 760b) + 1. If the left-side addend were
        * dropped, or the measurement taken before relaxation, this breaks. */
       if (jmp_field_off != region_a_len + 1) {
               printf("FAIL jmp_field_off=%u expected %u\n",
                      jmp_field_off, region_a_len + 1);
               fail = 1;
       }
       /* Both regions start with a jmp that must have relaxed to 2 bytes:
        * region_a = jmp(2) + 4 nops = 6, region_b = jmp(2) + 2 nops = 4. */
       if (region_a_len != 6) {
               printf("FAIL region_a_len=%u expected 6 (jmp not relaxed?)\n",
                      region_a_len);
               fail = 1;
       }
       if (region_b_len != 4) {
               printf("FAIL region_b_len=%u expected 4 (jmp not relaxed?)\n",
                      region_b_len);
               fail = 1;
       }

       if (fail)
               return 1;
       printf("PASS kernel_altinstr_layout\n");
       return 0;
}
