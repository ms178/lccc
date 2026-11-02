/* GNU as macro semantics the Linux kernel depends on.
 *
 * Each block below pins down a bug that produced silently-wrong objects:
 *
 *  1. `\@` (macro-invocation counter) unimplemented -- every `.Lhere_\@:`
 *     collapsed onto ONE label, so the kernel's ANNOTATE pointed all
 *     `.discard.annotate_insn` entries at the same address instead of at the
 *     instruction each annotates.  objtool: "intra_function_call not a direct
 *     call".
 *  2. `.macro` parameter list split on raw whitespace -- a default value
 *     containing spaces, e.g. `ftr2=ALT_NOT(X86_FEATURE_ALWAYS)` expanding to
 *     `(((1 << 0) << 16) | ((3*32+21)))`, was shredded into parameters named
 *     `<<` and `0)`, and the body emitted the fragment `(((1` to `.4byte`.
 *  3. `:req` / `:vararg` qualifiers left in the parameter NAME, so `\ftr`
 *     never substituted.
 *  4. A macro invocation spliced in through a quoted argument
 *     ("nop;ANNOTATE type=7; call 772f") was pushed verbatim because only the
 *     FIRST statement on a line was re-expanded.
 *  5. `.set type, 3` (kernel UNWIND_HINT) rewrote the DIRECTIVE `.type` into
 *     `.3`, because symbol substitution was whole-word and `.` counts as a
 *     word boundary.  26 function symbols silently stayed STT_NOTYPE.
 *  6. Instruction prefixes separated by TAB (`rep\tstosl`) were parsed as a
 *     mnemonic `rep` with a label operand.
 *
 * Values are checked at RUNTIME so a wrong expansion is a failure, not a
 * warning.
 */
#include <stdio.h>

extern unsigned int at_marks[3];   /* one .long per MARK expansion */
extern unsigned int dflt_used, dflt_override;
extern unsigned int req_ok;
extern unsigned int spliced_count;

/* --- 1. `\@` must differ per expansion. Three invocations, three distinct
 * self-relative offsets recorded in a side table. */
asm(".macro MARK\n"
    ".Lmk_\\@:\n"
    "  .pushsection .data\n"
    "  .long .Lmk_\\@ - probe_at\n"
    "  .popsection\n"
    "  nop\n"
    ".endm\n"
    ".text\n"
    ".globl probe_at\n"
    ".type probe_at, @function\n"
    "probe_at:\n"
    "  .pushsection .data\n"
    "  .globl at_marks\n"
    "at_marks:\n"
    "  .popsection\n"
    "  MARK\n"
    "  MARK\n"
    "  MARK\n"
    "  ret\n"
    ".size probe_at, .-probe_at\n");

/* --- 2+3. A default value full of spaces and parentheses, plus `:req`. */
asm(".macro EMIT val:req flags=(((1 << 0) << 16) | ((3*32+21)))\n"
    "  .long \\val\n"
    "  .long \\flags\n"
    ".endm\n"
    ".data\n"
    ".globl dflt_used\n"
    "dflt_used:\n"
    "  EMIT 0x1111\n"           /* uses the default */
    ".globl dflt_override\n"
    "dflt_override:\n"
    "  EMIT 0x2222, (7*32+9)\n" /* overrides it */
    ".globl req_ok\n"
    "req_ok:\n"
    "  .long 1\n");

/* --- 4. A macro invocation that only appears after a `;` once an OUTER macro
 * has substituted a quoted argument. */
asm(".macro COUNTER\n"
    "  .pushsection .data\n"
    "  .long 1\n"
    "  .popsection\n"
    ".endm\n"
    ".macro OUTER body\n"
    "  \\body\n"
    ".endm\n"
    ".data\n"
    ".globl spliced_count\n"
    "spliced_count:\n"
    ".text\n"
    "  OUTER \"nop;COUNTER;nop\"\n");

/* --- 5. `.set type, N` must NOT corrupt the `.type` directive, and the
 * function symbol must come out STT_FUNC. */
asm(".text\n"
    "  .set type, 3\n"
    "  .set signal, 1\n"
    ".globl typed_fn\n"
    "typed_fn:\n"
    "  movl $77, %eax\n"
    "  ret\n"
    ".type typed_fn STT_FUNC\n"
    ".size typed_fn, .-typed_fn\n");
extern int typed_fn(void);

/* --- 6. TAB-separated prefix, and a segment-name prefix (kernel `ds wrmsr`
 * reserves a byte for ALTERNATIVE patching). */
asm(".text\n"
    ".globl tab_prefix\n"
    ".type tab_prefix, @function\n"
    "tab_prefix:\n"
    "  pushq %rdi\n"
    "  pushq %rcx\n"
    "  pushq %rax\n"
    "  leaq buf(%rip), %rdi\n"
    "  movl $4, %ecx\n"
    "  movl $0, %eax\n"
    "  rep\tstosl\n"            /* TAB, not space */
    "  ds nop\n"                /* segment-name prefix on a harmless insn */
    "  popq %rax\n"
    "  popq %rcx\n"
    "  popq %rdi\n"
    "  ret\n"
    ".size tab_prefix, .-tab_prefix\n"
    ".bss\n"
    ".globl buf\n"
    ".align 16\n"
    "buf:\n"
    "  .zero 16\n");
extern void tab_prefix(void);
extern unsigned int buf[4];

int main(void)
{
       int fail = 0;

       /* 1. three DISTINCT offsets => `\@` produced three distinct labels. */
       if (at_marks[0] == at_marks[1] || at_marks[1] == at_marks[2] ||
           at_marks[0] == at_marks[2]) {
               printf("FAIL \\@ collapsed: %u %u %u\n",
                      at_marks[0], at_marks[1], at_marks[2]);
               fail = 1;
       }
       /* They are one `nop` apart, in order. */
       if (at_marks[1] != at_marks[0] + 1 || at_marks[2] != at_marks[1] + 1) {
               printf("FAIL \\@ spacing: %u %u %u\n",
                      at_marks[0], at_marks[1], at_marks[2]);
               fail = 1;
       }

       /* 2. default expands to (((1<<0)<<16) | ((3*32+21))) = 0x10000 | 117. */
       if (dflt_used != 0x1111) {
               printf("FAIL default val=%#x\n", dflt_used);
               fail = 1;
       }
       if (*(&dflt_used + 1) != (0x10000u | 117u)) {
               printf("FAIL default flags=%#x expected %#x\n",
                      *(&dflt_used + 1), 0x10000u | 117u);
               fail = 1;
       }
       /* 3. override still works => `:req`/`=` parsing is sane. */
       if (dflt_override != 0x2222 || *(&dflt_override + 1) != (7u * 32 + 9)) {
               printf("FAIL override %#x/%#x\n",
                      dflt_override, *(&dflt_override + 1));
               fail = 1;
       }

       /* 4. the spliced COUNTER really expanded. */
       if (spliced_count != 1) {
               printf("FAIL spliced_count=%u\n", spliced_count);
               fail = 1;
       }

       /* 5. `.type` survived `.set type, 3`. */
       if (typed_fn() != 77) {
               printf("FAIL typed_fn\n");
               fail = 1;
       }

       /* 6. `rep stosl` zeroed 4 dwords; `ds nop` executed harmlessly. */
       buf[0] = buf[1] = buf[2] = buf[3] = 0xdeadbeef;
       tab_prefix();
       if (buf[0] || buf[1] || buf[2] || buf[3]) {
               printf("FAIL rep stosl: %#x %#x %#x %#x\n",
                      buf[0], buf[1], buf[2], buf[3]);
               fail = 1;
       }

       if (fail)
               return 1;
       printf("PASS kernel_asm_macro_semantics\n");
       return 0;
}
