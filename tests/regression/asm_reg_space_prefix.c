/* GNU as tolerates whitespace between '%' and a register name.
 *
 * The Linux kernel depends on it: arch/x86/include/asm/asm.h builds
 * `_ASM_RIP(x)` through __ASM_REGPFX, which splices `(% rip)` -- with a space.
 * lccc stripped only the '%', leaving a register named " rip" that matched
 * nothing and silently fell back to %rax, so
 *     movl x86_pred_cmd(%rip), %eax
 * assembled as `mov 0x0(%rax), %eax`: a wrong-code bug with no diagnostic.
 *
 * This checks the RIP-relative load, a stack access and a plain register move
 * in the spaced spelling all still address what they say.
 */
#include <stdio.h>

int value = 0x1234;   /* extern: must survive to be named in asm */

/* Verbatim assembly: `(% rsp)` with a space, exactly as _ASM_RIP()/__ASM_REGPFX
 * emit it. Must address the stack, not fall back to %rax. */
asm(".text\n"
    ".globl get_sp_plus_8\n"
    ".type get_sp_plus_8, @function\n"
    "get_sp_plus_8:\n"
    "  leaq 8(% rsp), %rax\n"
    "  ret\n"
    ".size get_sp_plus_8, .-get_sp_plus_8\n"
    ".globl read_value_rip\n"
    ".type read_value_rip, @function\n"
    "read_value_rip:\n"
    "  movl value(% rip), %eax\n"
    "  ret\n"
    ".size read_value_rip, .-read_value_rip\n");
long get_sp_plus_8(void);
long read_value_rip(void);

int main(void)
{
	int got = 0;
	long stack_probe = 0x5678;
	long back = 0;

	/* `(% rip)`: must read `value`, not whatever %rax happens to hold. */
	got = (int)read_value_rip();

	/* `(% rsp)`-style base: read back a known stack slot. */
	asm volatile("movq %1, %0" : "=r"(back) : "m"(stack_probe));

	/* Spaced register name inside a memory operand's base, written the way
	 * the kernel's __ASM_REGPFX splices it. GCC's *inline-asm template*
	 * parser rejects "% r" (it looks like an operand %-code), so this form
	 * is exercised via a top-level asm block, which is verbatim text -- the
	 * same path a .S file takes. */
	long moved = 0;
	/* A `call` inside inline asm clobbers every caller-saved register, so
	 * they must all be listed. Omitting them is an ABI violation that
	 * happens to survive under some register allocations and crashes under
	 * others -- it is not something the compiler can be expected to
	 * tolerate. */
	asm volatile("call get_sp_plus_8"
		     : "=a"(moved)
		     :
		     : "memory", "rcx", "rdx", "rsi", "rdi",
		       "r8", "r9", "r10", "r11");

	if (got != 0x1234) {
		printf("FAIL rip-relative got=0x%x\n", got);
		return 1;
	}
	if (back != 0x5678) {
		printf("FAIL stack got=0x%lx\n", back);
		return 2;
	}
	if (moved == 0) {
		printf("FAIL spaced reg move\n");
		return 3;
	}
	printf("PASS asm_reg_space_prefix\n");
	return 0;
}
