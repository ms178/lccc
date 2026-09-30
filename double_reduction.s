	.file	"double_reduction.c"
	.text
	.section	.rodata.str1.1,"aMS",@progbits,1
.LC0:
	.string	"%lld\n"
	.section	.text.startup,"ax",@progbits
	.p2align 4
	.globl	main
	.type	main, @function
main:
.LFB11:
	.cfi_startproc
	pushq	%rbp
	.cfi_def_cfa_offset 16
	.cfi_offset 6, -16
	xorl	%esi, %esi
	movl	$1, %eax
	leaq	a(%rip), %rdx
	leaq	b(%rip), %rcx
	leaq	c(%rip), %rdi
	movq	%rsp, %rbp
	.cfi_def_cfa_register 6
	andq	$-32, %rsp
.L2:
	imull	$1664525, %eax, %eax
	addl	$1013904223, %eax
	movl	%eax, %r8d
	imull	$1664525, %eax, %eax
	shrl	$16, %r8d
	andl	$15, %r8d
	subl	$7, %r8d
	addl	$1013904223, %eax
	movl	%r8d, (%rdx,%rsi)
	movl	%eax, %r8d
	imull	$1664525, %eax, %eax
	shrl	$16, %r8d
	andl	$15, %r8d
	subl	$7, %r8d
	addl	$1013904223, %eax
	movl	%r8d, (%rcx,%rsi)
	movl	%eax, %r8d
	shrl	$16, %r8d
	andl	$15, %r8d
	subl	$7, %r8d
	movl	%r8d, (%rdi,%rsi)
	addq	$4, %rsi
	cmpq	$4194304, %rsi
	jne	.L2
	movl	$128, %r8d
	xorl	%esi, %esi
.L3:
	vpxor	%xmm2, %xmm2, %xmm2
	xorl	%eax, %eax
	vmovdqa	%ymm2, %ymm1
	.p2align 6
	.p2align 4
	.p2align 3
.L4:
	vmovdqa	(%rdx,%rax), %ymm0
	vpmulld	(%rcx,%rax), %ymm0, %ymm3
	vpaddd	%ymm3, %ymm1, %ymm1
	vpmulld	(%rdi,%rax), %ymm0, %ymm0
	addq	$32, %rax
	vpaddd	%ymm0, %ymm2, %ymm2
	cmpq	$4194304, %rax
	jne	.L4
	vextracti128	$0x1, %ymm2, %xmm0
	vpaddd	%xmm2, %xmm0, %xmm0
	vpsrldq	$8, %xmm0, %xmm2
	vpaddd	%xmm2, %xmm0, %xmm0
	vpsrldq	$4, %xmm0, %xmm2
	vpaddd	%xmm2, %xmm0, %xmm0
	vextracti128	$0x1, %ymm1, %xmm2
	vpaddd	%xmm1, %xmm2, %xmm1
	vpsrldq	$8, %xmm1, %xmm2
	vpaddd	%xmm2, %xmm1, %xmm1
	vpsrldq	$4, %xmm1, %xmm2
	vpaddd	%xmm2, %xmm1, %xmm1
	vpaddd	%xmm1, %xmm0, %xmm0
	vpxor	%xmm1, %xmm1, %xmm1
	vmovd	%xmm0, %eax
	vmovdqa	%ymm1, %ymm0
	cltq
	addq	%rax, %rsi
	xorl	%eax, %eax
	.p2align 5
	.p2align 4
	.p2align 3
.L5:
	vpaddd	(%rdx,%rax), %ymm0, %ymm0
	vpaddd	(%rcx,%rax), %ymm1, %ymm1
	addq	$32, %rax
	cmpq	$4194304, %rax
	jne	.L5
	vextracti128	$0x1, %ymm1, %xmm2
	vpaddd	%xmm1, %xmm2, %xmm1
	vpsrldq	$8, %xmm1, %xmm2
	vpaddd	%xmm2, %xmm1, %xmm1
	vpsrldq	$4, %xmm1, %xmm2
	vpaddd	%xmm2, %xmm1, %xmm1
	vextracti128	$0x1, %ymm0, %xmm2
	vpaddd	%xmm0, %xmm2, %xmm0
	vpsrldq	$8, %xmm0, %xmm2
	vpaddd	%xmm2, %xmm0, %xmm0
	vpsrldq	$4, %xmm0, %xmm2
	vpaddd	%xmm2, %xmm0, %xmm0
	vpaddd	%xmm0, %xmm1, %xmm0
	vmovd	%xmm0, %eax
	cltq
	addq	%rax, %rsi
	subl	$1, %r8d
	jne	.L3
	xorl	%eax, %eax
	leaq	.LC0(%rip), %rdi
	vzeroupper
	call	printf@PLT
	xorl	%eax, %eax
	leave
	.cfi_def_cfa 7, 8
	ret
	.cfi_endproc
.LFE11:
	.size	main, .-main
	.local	c
	.comm	c,4194304,32
	.local	b
	.comm	b,4194304,32
	.local	a
	.comm	a,4194304,32
	.ident	"GCC: (Debian 14.2.0-19) 14.2.0"
	.section	.note.GNU-stack,"",@progbits
