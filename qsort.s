	.file	"qsort.c"
	.text
	.p2align 4
	.type	cmp, @function
cmp:
.LFB22:
	.cfi_startproc
	movl	(%rdi), %eax
	subl	(%rsi), %eax
	ret
	.cfi_endproc
.LFE22:
	.size	cmp, .-cmp
	.section	.rodata.str1.1,"aMS",@progbits,1
.LC0:
	.string	"qsort: arr[500000] = %d\n"
	.section	.text.startup,"ax",@progbits
	.p2align 4
	.globl	main
	.type	main, @function
main:
.LFB23:
	.cfi_startproc
	leaq	arr(%rip), %rdi
	subq	$8, %rsp
	.cfi_def_cfa_offset 16
	movl	$42, %eax
	movq	%rdi, %rdx
	leaq	4000000(%rdi), %rsi
	.p2align 5
	.p2align 4
	.p2align 3
.L4:
	imull	$1664525, %eax, %eax
	addq	$4, %rdx
	addl	$1013904223, %eax
	movl	%eax, %ecx
	andl	$2147483647, %ecx
	movl	%ecx, -4(%rdx)
	cmpq	%rsi, %rdx
	jne	.L4
	leaq	cmp(%rip), %rcx
	movl	$4, %edx
	movl	$1000000, %esi
	call	qsort@PLT
	movl	2000000+arr(%rip), %esi
	leaq	.LC0(%rip), %rdi
	xorl	%eax, %eax
	call	printf@PLT
	xorl	%eax, %eax
	addq	$8, %rsp
	.cfi_def_cfa_offset 8
	ret
	.cfi_endproc
.LFE23:
	.size	main, .-main
	.local	arr
	.comm	arr,4000000,32
	.ident	"GCC: (Debian 14.2.0-19) 14.2.0"
	.section	.note.GNU-stack,"",@progbits
