# Regression: the same GOT-slot reference on i686 (`q@GOT(%ebx)`), where the
# GOT base comes from the i386 PC thunk.  `main` returns 3*5 + 5*2 = 25.
#
# i686 has two spellings for this operator: GAS emits `R_386_GOT32X` for the
# `mov`-load form (the linker may relax it to `q@GOTOFF(%ebx)`, removing the
# GOT entry and the load) and `R_386_GOT32` elsewhere.  LCCC currently emits
# `R_386_GOT32` in both cases: the relocation is correct and the pipelines
# below all return 25, but the relaxable spelling is not offered.  The gate
# asserts the values and reports the spelling difference explicitly against
# the GAS oracle; the follow-up ledger carries the analysis
# (engineering/FOLLOWUP-2026-10-13-audit-fix-session.md).
	.data
	.local	q1
	.p2align 2
q1:	.long	3
	.local	q2
	.p2align 2
q2:	.long	5
	.text
	.globl	main
main:
	pushl	%ebx
	call	__x86.get_pc_thunk.bx
	addl	$_GLOBAL_OFFSET_TABLE_, %ebx
	movl	q1@GOT(%ebx), %ecx
	movl	(%ecx), %eax
	movl	q2@GOT(%ebx), %edx
	movl	(%edx), %edx
	leal	(%eax,%eax,4), %eax
	addl	%edx, %edx
	addl	%edx, %eax
	popl	%ebx
	ret
