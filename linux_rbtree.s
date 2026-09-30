	.file	"linux_rbtree.c"
	.text
	.section	.rodata.str1.1,"aMS",@progbits,1
.LC0:
	.string	"%016llx\n"
	.section	.text.startup,"ax",@progbits
	.p2align 4
	.globl	main
	.type	main, @function
main:
.LFB17:
	.cfi_startproc
	pushq	%rbx
	.cfi_def_cfa_offset 16
	.cfi_offset 3, -16
	leaq	node_pool(%rip), %rsi
	vpxor	%xmm0, %xmm0, %xmm0
	xorl	%r11d, %r11d
	vmovdqu	%xmm0, 8+node_pool(%rip)
	movq	%rsi, %rdi
	xorl	%eax, %eax
	movq	%rsi, %r8
	movl	$1663615696, 24+node_pool(%rip)
	movl	$1663615696, %r10d
	movq	%rsi, %r9
	movl	$0, 28+node_pool(%rip)
	movq	$0, node_pool(%rip)
.L2:
	orq	$1, %rax
	movq	%rax, (%r9)
	.p2align 4
	.p2align 3
.L10:
	addl	$1, %r11d
	addq	$32, %rsi
	cmpl	$16384, %r11d
	je	.L41
	imull	$1664525, %r10d, %r10d
	movl	%r11d, 28(%rsi)
	movq	%rsi, %r9
	movq	%rdi, %rax
	vmovdqu	%xmm0, 8(%rsi)
	addl	$1013904223, %r10d
	movl	%r10d, %ecx
	andl	$2147483647, %ecx
	movl	%ecx, 24(%rsi)
	jmp	.L7
	.p2align 5
	.p2align 4,,10
	.p2align 3
.L74:
	movq	16(%rax), %rdx
	testq	%rdx, %rdx
	je	.L73
.L37:
	movq	%rdx, %rax
.L7:
	cmpl	%ecx, 24(%rax)
	jg	.L74
	movq	8(%rax), %rdx
	testq	%rdx, %rdx
	jne	.L37
	leaq	8(%rax), %rdx
.L6:
	movq	%rax, %rcx
	movq	%rax, (%rsi)
	andq	$-4, %rcx
	movq	%rsi, (%rdx)
	cmpq	$3, %rax
	jbe	.L2
.L9:
	movq	(%rcx), %rdx
	testb	$1, %dl
	jne	.L10
	andq	$-4, %rdx
	movq	8(%rdx), %rax
	cmpq	%rcx, %rax
	je	.L12
	testq	%rax, %rax
	je	.L13
	movq	(%rax), %rbx
	testb	$1, %bl
	je	.L65
.L13:
	movq	8(%rcx), %rax
	cmpq	%r9, %rax
	je	.L15
	movq	%rcx, %r9
.L16:
	movq	%rax, 16(%rdx)
	movq	%rdx, 8(%rcx)
	testq	%rax, %rax
	je	.L26
.L70:
	movq	%rdx, %rbx
	orq	$1, %rbx
	movq	%rbx, (%rax)
.L26:
	movq	(%rdx), %rax
	movq	%rax, (%rcx)
	movq	%r9, (%rdx)
	cmpq	$3, %rax
	jbe	.L40
.L76:
	andq	$-4, %rax
	cmpq	16(%rax), %rdx
	je	.L75
	movq	%rcx, 8(%rax)
	jmp	.L10
.L65:
	orq	$1, %rbx
	movq	%rdx, %r9
	movq	%rbx, (%rax)
	orq	$1, (%rcx)
	movq	(%rdx), %rcx
	movq	%rcx, %rax
	andq	$-4, %rcx
	andq	$-2, %rax
	movq	%rax, (%rdx)
	testq	%rcx, %rcx
	jne	.L9
	jmp	.L2
.L73:
	leaq	16(%rax), %rdx
	jmp	.L6
.L12:
	movq	16(%rdx), %rax
	testq	%rax, %rax
	je	.L22
	movq	(%rax), %rbx
	testb	$1, %bl
	je	.L65
.L22:
	movq	16(%rcx), %rax
	cmpq	%r9, %rax
	je	.L23
	movq	%rcx, %r9
.L24:
	movq	%rax, 8(%rdx)
	movq	%rdx, 16(%rcx)
	testq	%rax, %rax
	jne	.L70
	movq	(%rdx), %rax
	movq	%rax, (%rcx)
	movq	%r9, (%rdx)
	cmpq	$3, %rax
	ja	.L76
.L40:
	movq	%rcx, %rdi
	jmp	.L10
.L15:
	movq	16(%r9), %rax
	movq	%rax, 8(%rcx)
	movq	%rcx, 16(%r9)
	testq	%rax, %rax
	je	.L17
	movq	%rcx, %rbx
	orq	$1, %rbx
	movq	%rbx, (%rax)
.L17:
	movq	%r9, (%rcx)
	movq	8(%r9), %rax
	movq	%r9, %rcx
	jmp	.L16
.L75:
	movq	%rcx, 16(%rax)
	jmp	.L10
.L23:
	movq	8(%r9), %rax
	movq	%rax, 16(%rcx)
	movq	%rcx, 8(%r9)
	testq	%rax, %rax
	je	.L25
	movq	%rcx, %rbx
	orq	$1, %rbx
	movq	%rbx, (%rax)
.L25:
	movq	%r9, (%rcx)
	movq	16(%r9), %rax
	movq	%r9, %rcx
	jmp	.L24
.L41:
	movl	$8, %r10d
	xorl	%r9d, %r9d
.L28:
	xorl	%esi, %esi
	.p2align 4
	.p2align 3
.L33:
	movl	%esi, %eax
	andl	$16383, %eax
	salq	$5, %rax
	movl	24(%r8,%rax), %ecx
	movq	%rdi, %rax
	jmp	.L32
	.p2align 5
	.p2align 4,,10
	.p2align 3
.L77:
	movq	16(%rax), %rax
	testq	%rax, %rax
	je	.L35
.L32:
	movslq	24(%rax), %rdx
	cmpl	%edx, %ecx
	jl	.L77
	jle	.L31
	movq	8(%rax), %rax
	testq	%rax, %rax
	jne	.L32
.L35:
	addl	$104729, %esi
	cmpl	$1715879936, %esi
	jne	.L33
.L78:
	subl	$1, %r10d
	jne	.L28
	movq	%r9, %rsi
	leaq	.LC0(%rip), %rdi
	xorl	%eax, %eax
	call	printf@PLT
	xorl	%eax, %eax
	popq	%rbx
	.cfi_remember_state
	.cfi_def_cfa_offset 8
	ret
	.p2align 4,,10
	.p2align 3
.L31:
	.cfi_restore_state
	movslq	28(%rax), %rax
	salq	$16, %rdx
	addl	$104729, %esi
	xorq	%rax, %rdx
	addq	%rdx, %r9
	cmpl	$1715879936, %esi
	jne	.L33
	jmp	.L78
	.cfi_endproc
.LFE17:
	.size	main, .-main
	.local	node_pool
	.comm	node_pool,524288,32
	.ident	"GCC: (Debian 14.2.0-19) 14.2.0"
	.section	.note.GNU-stack,"",@progbits
