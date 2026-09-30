	.file	"tce_sum.c"
	.text
	.section	.rodata.str1.1,"aMS",@progbits,1
.LC0:
	.string	"sum(10000000) = %ld\n"
	.section	.text.startup,"ax",@progbits
	.p2align 4
	.globl	main
	.type	main, @function
main:
.LFB12:
	.cfi_startproc
	subq	$24, %rsp
	.cfi_def_cfa_offset 32
	leaq	.LC0(%rip), %rdi
	movabsq	$50000005000000, %rax
	movq	%rax, 8(%rsp)
	movq	8(%rsp), %rsi
	xorl	%eax, %eax
	call	printf@PLT
	xorl	%eax, %eax
	addq	$24, %rsp
	.cfi_def_cfa_offset 8
	ret
	.cfi_endproc
.LFE12:
	.size	main, .-main
	.ident	"GCC: (Debian 14.2.0-19) 14.2.0"
	.section	.note.GNU-stack,"",@progbits
