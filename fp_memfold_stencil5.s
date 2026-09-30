	.file	"fp_memfold_stencil5.c"
	.text
	.p2align 4
	.type	stencil5.constprop.0, @function
stencil5.constprop.0:
.LFB24:
	.cfi_startproc
	leaq	input(%rip), %rax
	leaq	8+output(%rip), %rdx
	leaq	262112(%rax), %rcx
	.p2align 6
	.p2align 4
	.p2align 3
.L2:
	vmovups	4(%rax), %ymm0
	vaddps	(%rax), %ymm0, %ymm0
	addq	$32, %rax
	addq	$32, %rdx
	vaddps	-24(%rax), %ymm0, %ymm0
	vaddps	-20(%rax), %ymm0, %ymm0
	vaddps	-16(%rax), %ymm0, %ymm0
	vmovups	%ymm0, -32(%rdx)
	cmpq	%rax, %rcx
	jne	.L2
	vmovups	262116+input(%rip), %xmm0
	vaddps	262112+input(%rip), %xmm0, %xmm0
	vaddps	262120+input(%rip), %xmm0, %xmm0
	vaddps	262124+input(%rip), %xmm0, %xmm0
	vaddps	262128+input(%rip), %xmm0, %xmm0
	vmovups	%xmm0, 262120+output(%rip)
	vzeroupper
	ret
	.cfi_endproc
.LFE24:
	.size	stencil5.constprop.0, .-stencil5.constprop.0
	.section	.rodata.str1.1,"aMS",@progbits,1
.LC6:
	.string	"%.0f\n"
	.section	.text.startup,"ax",@progbits
	.p2align 4
	.globl	main
	.type	main, @function
main:
.LFB23:
	.cfi_startproc
	leaq	8(%rsp), %r10
	.cfi_def_cfa 10, 0
	andq	$-32, %rsp
	movl	%edi, %eax
	movl	$1000, %edi
	pushq	-8(%r10)
	pushq	%rbp
	movq	%rsp, %rbp
	.cfi_escape 0x10,0x6,0x2,0x76,0
	pushq	%r10
	.cfi_escape 0xf,0x3,0x76,0x78,0x6
	subq	$40, %rsp
	cmpl	$1, %eax
	jle	.L6
	movq	8(%rsi), %rdi
	movl	$10, %edx
	xorl	%esi, %esi
	call	strtol@PLT
	movl	%eax, %edi
.L6:
	movl	$8, %edx
	leaq	input(%rip), %rax
	vmovdqa	.LC0(%rip), %ymm1
	vbroadcastss	.LC5(%rip), %ymm4
	vmovd	%edx, %xmm3
	movl	$15, %edx
	leaq	262144(%rax), %rcx
	vmovd	%edx, %xmm2
	vpbroadcastd	%xmm3, %ymm3
	vpbroadcastd	%xmm2, %ymm2
	.p2align 6
	.p2align 4
	.p2align 3
.L7:
	vmovdqa	%ymm1, %ymm0
	addq	$32, %rax
	vpaddd	%ymm3, %ymm1, %ymm1
	vpand	%ymm2, %ymm0, %ymm0
	vcvtdq2ps	%ymm0, %ymm0
	vmulps	%ymm4, %ymm0, %ymm0
	vmovaps	%ymm0, -32(%rax)
	cmpq	%rax, %rcx
	jne	.L7
	testl	%edi, %edi
	jle	.L25
	xorl	%esi, %esi
	testb	$1, %dil
	jne	.L23
	vzeroupper
	.p2align 4
	.p2align 3
.L9:
	call	stencil5.constprop.0
	addl	$2, %esi
	call	stencil5.constprop.0
	cmpl	%esi, %edi
	jne	.L9
.L8:
	leaq	output(%rip), %rax
	vxorps	%xmm2, %xmm2, %xmm2
	vxorpd	%xmm1, %xmm1, %xmm1
	leaq	262144(%rax), %rdx
	.p2align 5
	.p2align 4
	.p2align 3
.L10:
	vcvtss2sd	(%rax), %xmm2, %xmm0
	addq	$4, %rax
	vaddsd	%xmm0, %xmm1, %xmm1
	cmpq	%rax, %rdx
	jne	.L10
	vmovapd	%xmm1, %xmm0
	movl	$1, %eax
	vmovsd	%xmm1, -24(%rbp)
	leaq	.LC6(%rip), %rdi
	call	printf@PLT
	vmovsd	-24(%rbp), %xmm1
	movl	$1, %eax
	vcomisd	.LC7(%rip), %xmm1
	jbe	.L5
	vmovsd	.LC8(%rip), %xmm0
	xorl	%eax, %eax
	vcomisd	%xmm1, %xmm0
	setbe	%al
.L5:
	addq	$40, %rsp
	popq	%r10
	.cfi_remember_state
	.cfi_def_cfa 10, 0
	popq	%rbp
	leaq	-8(%r10), %rsp
	.cfi_def_cfa 7, 8
	ret
.L23:
	.cfi_restore_state
	vzeroupper
	call	stencil5.constprop.0
	movl	$1, %esi
	cmpl	$1, %edi
	jne	.L9
	jmp	.L8
.L25:
	vzeroupper
	jmp	.L8
	.cfi_endproc
.LFE23:
	.size	main, .-main
	.local	output
	.comm	output,262144,32
	.local	input
	.comm	input,262144,32
	.section	.rodata.cst32,"aM",@progbits,32
	.align 32
.LC0:
	.long	0
	.long	1
	.long	2
	.long	3
	.long	4
	.long	5
	.long	6
	.long	7
	.section	.rodata.cst4,"aM",@progbits,4
	.align 4
.LC5:
	.long	1048576000
	.section	.rodata.cst8,"aM",@progbits,8
	.align 8
.LC7:
	.long	0
	.long	1092767616
	.align 8
.LC8:
	.long	0
	.long	1092807616
	.ident	"GCC: (Debian 14.2.0-19) 14.2.0"
	.section	.note.GNU-stack,"",@progbits
