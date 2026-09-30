	.file	"bitops.c"
	.text
	.section	.rodata.str1.8,"aMS",@progbits,1
	.align 8
.LC0:
	.string	"pop=%ld clz=%ld rev=%ld pow2=%ld\n"
	.section	.text.startup,"ax",@progbits
	.p2align 4
	.globl	main
	.type	main, @function
main:
.LFB15:
	.cfi_startproc
	pushq	%rbx
	.cfi_def_cfa_offset 16
	.cfi_offset 3, -16
	movl	$50000000, %ecx
	movl	$1107023190, %r10d
	xorl	%r8d, %r8d
	movl	$18, %r11d
	xorl	%r9d, %r9d
	xorl	%edi, %edi
	xorl	%esi, %esi
	movl	$-559038737, %edx
	jmp	.L9
	.p2align 4,,10
	.p2align 3
.L15:
	sall	$16, %r11d
	movl	$24, %ebx
	movl	$16, %eax
.L3:
	cmpl	$16777215, %r11d
	ja	.L4
	sall	$8, %r11d
	movl	%ebx, %eax
.L4:
	cmpl	$268435455, %r11d
	ja	.L5
	addl	$4, %eax
	sall	$4, %r11d
.L5:
	cmpl	$1073741823, %r11d
	ja	.L6
	addl	$2, %eax
	sall	$2, %r11d
.L6:
	movslq	%eax, %rbx
	addl	$1, %eax
	testl	%r11d, %r11d
	cltq
	cmovs	%rbx, %rax
.L2:
	addq	%rax, %rdi
	movzwl	%dx, %eax
	movzbl	%r10b, %r10d
	subl	$1, %eax
	addq	%r10, %r9
	movl	%eax, %r10d
	shrl	%r10d
	orl	%eax, %r10d
	movl	%r10d, %eax
	shrl	$2, %eax
	orl	%r10d, %eax
	movl	%eax, %r10d
	shrl	$4, %r10d
	orl	%r10d, %eax
	movl	%eax, %r10d
	shrl	$8, %r10d
	orl	%r10d, %eax
	movl	%eax, %r10d
	shrl	$16, %r10d
	orl	%r10d, %eax
	addl	$1, %eax
	addq	%rax, %r8
	subl	$1, %ecx
	je	.L8
	imull	$1664525, %edx, %eax
	xorl	%r11d, %r11d
	addl	$1013904223, %eax
	leal	(%rax,%rax), %r10d
	popcntl	%eax, %r11d
	shrl	%eax
	andl	$1431655765, %eax
	andl	$-1431655766, %r10d
	orl	%eax, %r10d
	movl	%r10d, %eax
	sall	$2, %r10d
	shrl	$2, %eax
	andl	$-858993460, %r10d
	andl	$858993459, %eax
	orl	%r10d, %eax
	movl	%eax, %r10d
	sall	$4, %eax
	shrl	$4, %r10d
	andl	$-252645136, %eax
	andl	$252645135, %r10d
	orl	%eax, %r10d
	bswap	%r10d
.L9:
	imull	$1664525, %edx, %edx
	addq	%r11, %rsi
	movl	$32, %eax
	addl	$1013904223, %edx
	je	.L2
	movl	%edx, %r11d
	cmpl	$65535, %edx
	jbe	.L15
	movl	$8, %ebx
	xorl	%eax, %eax
	jmp	.L3
.L8:
	movq	%rdi, %rdx
	movq	%r9, %rcx
	leaq	.LC0(%rip), %rdi
	xorl	%eax, %eax
	call	printf@PLT
	xorl	%eax, %eax
	popq	%rbx
	.cfi_def_cfa_offset 8
	ret
	.cfi_endproc
.LFE15:
	.size	main, .-main
	.ident	"GCC: (Debian 14.2.0-19) 14.2.0"
	.section	.note.GNU-stack,"",@progbits
