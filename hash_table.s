	.file	"hash_table.c"
	.text
	.section	.rodata.str1.1,"aMS",@progbits,1
.LC0:
	.string	"hash_table sum: %ld\n"
	.section	.text.startup,"ax",@progbits
	.p2align 4
	.globl	main
	.type	main, @function
main:
.LFB25:
	.cfi_startproc
	pushq	%r15
	.cfi_def_cfa_offset 16
	.cfi_offset 15, -16
	pushq	%r14
	.cfi_def_cfa_offset 24
	.cfi_offset 14, -24
	pushq	%r13
	.cfi_def_cfa_offset 32
	.cfi_offset 13, -32
	xorl	%r13d, %r13d
	pushq	%r12
	.cfi_def_cfa_offset 40
	.cfi_offset 12, -40
	leaq	table(%rip), %r12
	pushq	%rbp
	.cfi_def_cfa_offset 48
	.cfi_offset 6, -48
	movl	$12345, %ebp
	pushq	%rbx
	.cfi_def_cfa_offset 56
	.cfi_offset 3, -56
	subq	$8, %rsp
	.cfi_def_cfa_offset 64
	.p2align 4
	.p2align 3
.L6:
	imull	$1664525, %ebp, %ebp
	addl	$1013904223, %ebp
	movl	%ebp, %eax
	shrl	$16, %eax
	xorl	%ebp, %eax
	imull	$73244475, %eax, %eax
	movl	%eax, %edx
	shrl	$16, %edx
	xorl	%edx, %eax
	imull	$73244475, %eax, %eax
	movl	%eax, %edx
	shrl	$16, %edx
	xorl	%edx, %eax
	movzwl	%ax, %ebx
	movq	(%r12,%rbx,8), %r14
	testq	%r14, %r14
	je	.L2
	movq	%r14, %rax
	jmp	.L5
	.p2align 4
	.p2align 4,,10
	.p2align 3
.L3:
	movq	8(%rax), %rax
	testq	%rax, %rax
	je	.L2
.L5:
	cmpl	(%rax), %ebp
	jne	.L3
	movl	%r13d, 4(%rax)
.L4:
	addl	$1, %r13d
	cmpl	$2000000, %r13d
	jne	.L6
	movl	$2000000, %edx
	xorl	%r13d, %r13d
	movl	$12345, %ebx
	.p2align 4
	.p2align 3
.L10:
	imull	$1664525, %ebx, %ebx
	addl	$1013904223, %ebx
	movl	%ebx, %eax
	shrl	$16, %eax
	xorl	%ebx, %eax
	imull	$73244475, %eax, %eax
	movl	%eax, %ecx
	shrl	$16, %ecx
	xorl	%ecx, %eax
	imull	$73244475, %eax, %eax
	movl	%eax, %ecx
	shrl	$16, %ecx
	xorl	%ecx, %eax
	movzwl	%ax, %eax
	movq	(%r12,%rax,8), %rax
	testq	%rax, %rax
	jne	.L9
	jmp	.L22
	.p2align 4
	.p2align 4,,10
	.p2align 3
.L8:
	movq	8(%rax), %rax
	testq	%rax, %rax
	je	.L22
.L9:
	cmpl	(%rax), %ebx
	jne	.L8
	movslq	4(%rax), %rax
.L7:
	addq	%rax, %r13
	subl	$1, %edx
	jne	.L10
	xorl	%r15d, %r15d
	.p2align 4
	.p2align 3
.L21:
	imull	$1664525, %ebx, %ebx
	addl	$1013904223, %ebx
	movl	%ebx, %edx
	shrl	$16, %edx
	xorl	%ebx, %edx
	imull	$73244475, %edx, %edx
	movl	%edx, %eax
	shrl	$16, %eax
	xorl	%edx, %eax
	imull	$73244475, %eax, %eax
	movl	%eax, %r14d
	shrl	$16, %r14d
	xorl	%eax, %r14d
	movzwl	%r14w, %r14d
	movq	(%r12,%r14,8), %rbp
	testb	$1, %r15b
	je	.L11
	testq	%rbp, %rbp
	je	.L12
	movq	%rbp, %rax
	jmp	.L15
	.p2align 4
	.p2align 4,,10
	.p2align 3
.L13:
	movq	8(%rax), %rax
	testq	%rax, %rax
	je	.L12
.L15:
	cmpl	(%rax), %ebx
	jne	.L13
	movl	%r15d, 4(%rax)
.L14:
	addl	$1, %r15d
	cmpl	$2000000, %r15d
	jne	.L21
	movq	%r13, %rsi
	leaq	.LC0(%rip), %rdi
	xorl	%eax, %eax
	call	printf@PLT
	addq	$8, %rsp
	.cfi_remember_state
	.cfi_def_cfa_offset 56
	xorl	%eax, %eax
	popq	%rbx
	.cfi_def_cfa_offset 48
	popq	%rbp
	.cfi_def_cfa_offset 40
	popq	%r12
	.cfi_def_cfa_offset 32
	popq	%r13
	.cfi_def_cfa_offset 24
	popq	%r14
	.cfi_def_cfa_offset 16
	popq	%r15
	.cfi_def_cfa_offset 8
	ret
	.p2align 4,,10
	.p2align 3
.L22:
	.cfi_restore_state
	movq	$-1, %rax
	jmp	.L7
	.p2align 4,,10
	.p2align 3
.L2:
	movl	$16, %edi
	call	malloc@PLT
	movl	%ebp, (%rax)
	movl	%r13d, 4(%rax)
	movq	%r14, 8(%rax)
	movq	%rax, (%r12,%rbx,8)
	jmp	.L4
	.p2align 4,,10
	.p2align 3
.L11:
	testq	%rbp, %rbp
	jne	.L18
	jmp	.L39
	.p2align 4
	.p2align 4,,10
	.p2align 3
.L19:
	movq	8(%rbp), %rbp
	testq	%rbp, %rbp
	je	.L40
.L18:
	cmpl	0(%rbp), %ebx
	jne	.L19
	movslq	4(%rbp), %rax
.L20:
	addq	%rax, %r13
	addl	$1, %r15d
	jmp	.L21
	.p2align 4,,10
	.p2align 3
.L12:
	movl	$16, %edi
	call	malloc@PLT
	movl	%ebx, (%rax)
	movl	%r15d, 4(%rax)
	movq	%rbp, 8(%rax)
	movq	%rax, (%r12,%r14,8)
	jmp	.L14
	.p2align 4,,10
	.p2align 3
.L40:
	movq	$-1, %rax
	jmp	.L20
.L39:
	subq	$1, %r13
	addl	$1, %r15d
	jmp	.L21
	.cfi_endproc
.LFE25:
	.size	main, .-main
	.local	table
	.comm	table,524288,32
	.ident	"GCC: (Debian 14.2.0-19) 14.2.0"
	.section	.note.GNU-stack,"",@progbits
