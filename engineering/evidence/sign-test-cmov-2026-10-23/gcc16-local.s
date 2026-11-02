	.file	"sign_test_cmov.c"
	.text
	.p2align 4
	.globl	nonnegative_difference
	.type	nonnegative_difference, @function
nonnegative_difference:
.LFB11:
	.cfi_startproc
	subl	%esi, %edi
	movl	$0, %eax
	cmovns	%edi, %eax
	ret
	.cfi_endproc
.LFE11:
	.size	nonnegative_difference, .-nonnegative_difference
	.p2align 4
	.globl	negative_difference
	.type	negative_difference, @function
negative_difference:
.LFB12:
	.cfi_startproc
	movl	%edi, %eax
	xorl	%edx, %edx
	subl	%esi, %eax
	testl	%eax, %eax
	cmovg	%edx, %eax
	ret
	.cfi_endproc
.LFE12:
	.size	negative_difference, .-negative_difference
	.p2align 4
	.globl	positive_difference
	.type	positive_difference, @function
positive_difference:
.LFB13:
	.cfi_startproc
	subl	%esi, %edi
	movl	$0, %eax
	cmovns	%edi, %eax
	ret
	.cfi_endproc
.LFE13:
	.size	positive_difference, .-positive_difference
	.section	.rodata.str1.1,"aMS",@progbits,1
.LC0:
	.string	"%llu\n"
	.section	.text.startup,"ax",@progbits
	.p2align 4
	.globl	main
	.type	main, @function
main:
.LFB14:
	.cfi_startproc
	pushq	%rbx
	.cfi_def_cfa_offset 16
	.cfi_offset 3, -16
	movl	$values.0+28, %r8d
	movl	$values.0, %r9d
	xorl	%esi, %esi
	.p2align 4
	.p2align 3
.L6:
	movl	(%r9), %ecx
	movl	$values.0, %r10d
	.p2align 4
	.p2align 3
.L7:
	movl	(%r10), %r11d
	movq	%rsi, %rdx
	movl	%ecx, %edi
	addq	$4, %r10
	salq	$5, %rdx
	addq	%rsi, %rdx
	movl	%r11d, %esi
	call	nonnegative_difference
	movl	%ecx, %edi
	movl	%eax, %eax
	addq	%rdx, %rax
	movq	%rax, %rbx
	salq	$5, %rbx
	addq	%rax, %rbx
	call	negative_difference
	movl	%eax, %eax
	addq	%rbx, %rax
	movq	%rax, %rdx
	salq	$5, %rdx
	addq	%rax, %rdx
	call	positive_difference
	movl	%eax, %esi
	addq	%rdx, %rsi
	cmpq	%r10, %r8
	jne	.L7
	addq	$4, %r9
	cmpq	%r9, %r8
	jne	.L6
	movl	$.LC0, %edi
	xorl	%eax, %eax
	call	printf
	xorl	%eax, %eax
	popq	%rbx
	.cfi_def_cfa_offset 8
	ret
	.cfi_endproc
.LFE14:
	.size	main, .-main
	.section	.rodata
	.align 16
	.type	values.0, @object
	.size	values.0, 28
values.0:
	.long	-2147483648
	.long	-2147483647
	.long	-1
	.long	0
	.long	1
	.long	2147483646
	.long	2147483647
	.ident	"GCC: (GNU) 16.2.0"
	.section	.note.GNU-stack,"",@progbits
