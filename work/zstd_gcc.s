	.file	"zstd_count.c"
	.text
	.section	.rodata.str1.1,"aMS",@progbits,1
.LC0:
	.string	"%016llx\n"
	.section	.text.startup,"ax",@progbits
	.p2align 4
	.globl	main
	.type	main, @function
main:
.LFB14:
	.cfi_startproc
	pushq	%r13
	.cfi_def_cfa_offset 16
	.cfi_offset 13, -16
	xorl	%edx, %edx
	movl	$-2058980567, %eax
	leaq	buffer(%rip), %r8
	pushq	%r12
	.cfi_def_cfa_offset 24
	.cfi_offset 12, -24
	pushq	%rbp
	.cfi_def_cfa_offset 32
	.cfi_offset 6, -32
	pushq	%rbx
	.cfi_def_cfa_offset 40
	.cfi_offset 3, -40
	subq	$8, %rsp
	.cfi_def_cfa_offset 48
	jmp	.L4
	.p2align 4,,10
	.p2align 3
.L30:
	cmpq	$63, %rdx
	jbe	.L2
	movl	%eax, %ecx
	shrl	$28, %ecx
	leal	-64(%rcx,%rdx), %ecx
	movzbl	(%r8,%rcx), %ecx
.L3:
	movb	%cl, (%r8,%rdx)
	addq	$1, %rdx
	cmpq	$1048576, %rdx
	je	.L29
.L4:
	imull	$1664525, %eax, %eax
	addl	$1013904223, %eax
	testb	$7, %al
	je	.L30
.L2:
	movl	%eax, %edi
	shrl	$24, %edi
	movl	%edi, %ecx
	jmp	.L3
.L29:
	movl	$16, %r10d
	movl	$324508639, %esi
	xorl	%r9d, %r9d
.L5:
	xorl	%edi, %edi
	jmp	.L13
	.p2align 4,,10
	.p2align 3
.L7:
	xorq	%r12, %rcx
	rep bsfq	%rcx, %rcx
	sarl	$3, %ecx
	addl	%ecx, %eax
	subl	%ebx, %eax
.L9:
	movl	%edi, %ecx
	addl	$1, %edi
	andl	$7, %ecx
	sall	$3, %ecx
	salq	%cl, %rax
	xorq	%r11, %rax
	addq	%rax, %r9
	cmpl	$131072, %edi
	je	.L31
.L13:
	imull	$1664525, %esi, %esi
	addl	$1013904223, %esi
	movl	%esi, %edx
	movl	%esi, %r11d
	movl	%esi, %ebp
	andl	$127, %edx
	andl	$1048319, %r11d
	shrl	$12, %ebp
	addl	$16, %edx
	andl	$1048319, %ebp
	leaq	(%r8,%r11), %rbx
	addq	%r11, %rdx
	addq	%r8, %rbp
	movq	%rbx, %rax
	addq	%r8, %rdx
	leaq	-7(%rdx), %r13
	cmpq	%r13, %rbx
	jnb	.L6
.L8:
	movq	0(%rbp), %rcx
	movq	(%rax), %r12
	cmpq	%r12, %rcx
	jne	.L7
	addq	$8, %rax
	addq	$8, %rbp
	cmpq	%r13, %rax
	jb	.L8
	.p2align 4
	.p2align 3
.L6:
	cmpq	%rdx, %rax
	jb	.L10
	jmp	.L11
	.p2align 5
	.p2align 4,,10
	.p2align 3
.L12:
	addq	$1, %rax
	addq	$1, %rbp
	cmpq	%rax, %rdx
	je	.L11
.L10:
	movzbl	0(%rbp), %ecx
	cmpb	%cl, (%rax)
	je	.L12
.L11:
	subl	%ebx, %eax
	jmp	.L9
.L31:
	subl	$1, %r10d
	jne	.L5
	movq	%r9, %rsi
	leaq	.LC0(%rip), %rdi
	xorl	%eax, %eax
	call	printf@PLT
	addq	$8, %rsp
	.cfi_def_cfa_offset 40
	xorl	%eax, %eax
	popq	%rbx
	.cfi_def_cfa_offset 32
	popq	%rbp
	.cfi_def_cfa_offset 24
	popq	%r12
	.cfi_def_cfa_offset 16
	popq	%r13
	.cfi_def_cfa_offset 8
	ret
	.cfi_endproc
.LFE14:
	.size	main, .-main
	.local	buffer
	.comm	buffer,1048640,32
	.ident	"GCC: (Debian 14.2.0-19) 14.2.0"
	.section	.note.GNU-stack,"",@progbits
