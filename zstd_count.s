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
	leaq	buffer(%rip), %rdi
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
	.p2align 4
	.p2align 3
.L5:
	imull	$1664525, %eax, %eax
	addl	$1013904223, %eax
	movl	%eax, %ecx
	testb	$7, %al
	jne	.L2
	cmpq	$63, %rdx
	jbe	.L2
	shrl	$28, %ecx
	leal	-64(%rcx,%rdx), %ecx
	movzbl	(%rdi,%rcx), %ecx
	movb	%cl, (%rdi,%rdx)
	addq	$1, %rdx
	cmpq	$1048576, %rdx
	jne	.L5
.L4:
	movl	$16, %r9d
	movl	$324508639, %ecx
	xorl	%r8d, %r8d
.L6:
	xorl	%esi, %esi
	jmp	.L16
	.p2align 4,,10
	.p2align 3
.L8:
	xorq	%r12, %rax
	tzcntq	%rax, %rax
	shrq	$3, %rax
	addq	%rbp, %rax
	subl	%ebx, %eax
.L10:
	movl	%esi, %edx
	addl	$1, %esi
	andl	$7, %edx
	sall	$3, %edx
	shlx	%rdx, %rax, %rax
	xorq	%r10, %rax
	addq	%rax, %r8
	cmpl	$131072, %esi
	je	.L44
.L16:
	imull	$1664525, %ecx, %ecx
	addl	$1013904223, %ecx
	movl	%ecx, %edx
	movl	%ecx, %r10d
	movl	%ecx, %r11d
	andl	$127, %edx
	andl	$1048319, %r10d
	shrl	$12, %r11d
	addl	$16, %edx
	andl	$1048319, %r11d
	leaq	(%rdi,%r10), %rbx
	addq	%r10, %rdx
	addq	%rdi, %r11
	movq	%rbx, %rbp
	addq	%rdi, %rdx
	leaq	-7(%rdx), %r13
	cmpq	%r13, %rbx
	jnb	.L7
.L9:
	movq	(%r11), %rax
	movq	0(%rbp), %r12
	cmpq	%r12, %rax
	jne	.L8
	addq	$8, %rbp
	addq	$8, %r11
	cmpq	%r13, %rbp
	jb	.L9
	.p2align 4
	.p2align 3
.L7:
	cmpq	%rdx, %rbp
	jnb	.L11
	movzbl	(%r11), %eax
	cmpb	%al, 0(%rbp)
	jne	.L11
	leaq	1(%rbp), %rax
	cmpq	%rdx, %rax
	jnb	.L30
	movzbl	1(%r11), %r13d
	cmpb	%r13b, 1(%rbp)
	jne	.L30
	leaq	2(%rbp), %rax
	cmpq	%rdx, %rax
	jnb	.L30
	movzbl	2(%rbp), %r13d
	cmpb	%r13b, 2(%r11)
	jne	.L30
	leaq	3(%rbp), %rax
	cmpq	%rdx, %rax
	jnb	.L30
	movzbl	3(%rbp), %r13d
	cmpb	%r13b, 3(%r11)
	jne	.L30
	leaq	4(%rbp), %rax
	cmpq	%rdx, %rax
	jnb	.L30
	movzbl	4(%rbp), %r13d
	cmpb	%r13b, 4(%r11)
	jne	.L30
	leaq	5(%rbp), %rax
	cmpq	%rdx, %rax
	jnb	.L30
	movzbl	5(%rbp), %r13d
	cmpb	%r13b, 5(%r11)
	jne	.L30
	leaq	6(%rbp), %rax
	cmpq	%rdx, %rax
	jnb	.L30
	movzbl	6(%rbp), %r13d
	cmpb	%r13b, 6(%r11)
	jne	.L30
	leaq	7(%rbp), %rax
	cmpq	%rdx, %rax
	jnb	.L30
	movzbl	7(%rbp), %edx
	addq	$8, %rbp
	cmpb	7(%r11), %dl
	cmovne	%rax, %rbp
.L11:
	movl	%ebp, %eax
	subl	%ebx, %eax
	jmp	.L10
	.p2align 4,,10
	.p2align 3
.L30:
	movq	%rax, %rbp
	jmp	.L11
	.p2align 4,,10
	.p2align 3
.L2:
	shrl	$24, %ecx
	movb	%cl, (%rdi,%rdx)
	addq	$1, %rdx
	cmpq	$1048576, %rdx
	jne	.L5
	jmp	.L4
.L44:
	subl	$1, %r9d
	jne	.L6
	movq	%r8, %rsi
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
