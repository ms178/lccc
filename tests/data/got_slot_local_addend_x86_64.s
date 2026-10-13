# Regression: a GOT-slot reference to a file-local symbol must keep the symbol
# in the relocation, and the assembler must pick the spelling GAS picks.
#
# Two locals with nonzero offsets in `.data` are read through their GOT slots;
# `main` returns 3*5 + 5*2 = 25.  If the assembler folds the symbol into the
# section (the AUD-CPU-1 GOT32/GOTPCREL defect), the linker computes the slot
# from `.data` and adds the symbol offset a second time: the program then
# returns a garbage value in a static link (measured: 145 with GNU ld) or
# faults under the LCCC linker, because the address it dereferences is
# `slot + offset` instead of the symbol.
#
# Measured against GAS 2.47 on this exact file: both `movq q@GOTPCREL(%rip)`
# references are `R_X86_64_REX_GOTPCRELX` against `q1`/`q2` -- the symbol is
# never folded, and the relaxable spelling is the one the linker may rewrite
# into a direct `lea` (which is what makes the static link return 25).
#
# Every pipeline in tests/regression/check_got_slot_local_addend.sh must give
# exit status 25: LCCC x86-64 (default and -no-pie) and LCCC object + GNU ld.
	.data
	.local	q1
	.p2align 3
q1:	.quad	3
	.local	q2
	.p2align 3
q2:	.quad	5
	.text
	.globl	main
main:
	movq	q1@GOTPCREL(%rip), %rcx
	movq	(%rcx), %rax
	movq	q2@GOTPCREL(%rip), %rdx
	movq	(%rdx), %rdx
	leaq	(%rax,%rax,4), %rax
	addq	%rdx, %rdx
	addq	%rdx, %rax
	ret
