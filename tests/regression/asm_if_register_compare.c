/* Assembler `.if` over register names.
 *
 * The kernel's UNWIND_HINT_REGS macro (arch/x86/include/asm/unwind_hints.h)
 * declares `base=%rsp` and branches on `.if \base == %rsp` / `.elseif
 * \base == %rbp`. GNU as compares the register spellings as tokens; lccc's
 * conditional-head validator used to reject a `%` as a non-constant token and
 * so refused every such macro (linux-6.18.55 entry_64.S).
 *
 * Each probe below returns the constant chosen by its `.if` chain; the values
 * and the branch selected must match GNU as (the GCC oracle of this test). */
#include <stdio.h>

__asm__(
	".text\n"
	".macro probe base=%rsp\n"
	"\t.if \\base == %rsp\n"
	"\t\tmovl $5, %eax\n"
	"\t.elseif \\base == %rbp\n"
	"\t\tmovl $4, %eax\n"
	"\t.else\n"
	"\t\tmovl $7, %eax\n"
	"\t.endif\n"
	".endm\n"
	".globl probe_default\n"
	"probe_default:\n"
	"\tprobe\n"
	"\tret\n"
	".globl probe_rbp\n"
	"probe_rbp:\n"
	"\tprobe base=%rbp\n"
	"\tret\n"
	".globl probe_rdi\n"
	"probe_rdi:\n"
	"\tprobe base=%rdi\n"
	"\tret\n"
	/* Modulo inside a `.if` is an operator, not a register spelling. */
	".globl probe_mod\n"
	"probe_mod:\n"
	"\t.if 17 % 5 == 2\n"
	"\t\tmovl $11, %eax\n"
	"\t.else\n"
	"\t\tmovl $12, %eax\n"
	"\t.endif\n"
	"\tret\n");

int probe_default(void);
int probe_rbp(void);
int probe_rdi(void);
int probe_mod(void);

int main(void)
{
	printf("%d %d %d %d\n", probe_default(), probe_rbp(), probe_rdi(), probe_mod());
	return 0;
}
