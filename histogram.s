	.file	"histogram.c"
	.text
	.section	.text.startup,"ax",@progbits
	.p2align 4
	.globl	main
	.type	main, @function
main:
.LFB0:
	.cfi_startproc
	movl	$8, %ecx
	leaq	bytes(%rip), %rax
	vmovdqa	.LC0(%rip), %ymm2
	vmovd	%ecx, %xmm9
	movl	$16, %ecx
	leaq	262144(%rax), %rsi
	movq	%rax, %rdx
	vmovd	%ecx, %xmm8
	movl	$24, %ecx
	vpbroadcastd	%xmm9, %ymm9
	vmovd	%ecx, %xmm7
	movl	$32, %ecx
	vpbroadcastd	%xmm8, %ymm8
	vmovd	%ecx, %xmm6
	movl	$-1640531535, %ecx
	vpbroadcastd	%xmm7, %ymm7
	vmovd	%ecx, %xmm4
	movl	$65535, %ecx
	vpbroadcastd	%xmm6, %ymm6
	vmovd	%ecx, %xmm3
	movl	$16711935, %ecx
	vpbroadcastd	%xmm4, %ymm4
	vmovd	%ecx, %xmm5
	vpbroadcastd	%xmm3, %ymm3
	vpbroadcastd	%xmm5, %ymm5
	.p2align 4
	.p2align 3
.L2:
	vpaddd	%ymm9, %ymm2, %ymm11
	vpaddd	%ymm8, %ymm2, %ymm0
	vmovdqa	%ymm2, %ymm1
	addq	$32, %rdx
	vpmulld	%ymm4, %ymm1, %ymm1
	vpmulld	%ymm4, %ymm11, %ymm11
	vpaddd	%ymm7, %ymm2, %ymm10
	vpmulld	%ymm4, %ymm0, %ymm0
	vpmulld	%ymm4, %ymm10, %ymm10
	vpaddd	%ymm6, %ymm2, %ymm2
	vpsrld	$24, %ymm1, %ymm1
	vpsrld	$24, %ymm11, %ymm11
	vpsrld	$24, %ymm0, %ymm0
	vpsrld	$24, %ymm10, %ymm10
	vpand	%ymm1, %ymm3, %ymm1
	vpand	%ymm11, %ymm3, %ymm11
	vpand	%ymm0, %ymm3, %ymm0
	vpand	%ymm10, %ymm3, %ymm10
	vpackusdw	%ymm11, %ymm1, %ymm1
	vpackusdw	%ymm10, %ymm0, %ymm0
	vpermq	$216, %ymm1, %ymm1
	vpermq	$216, %ymm0, %ymm0
	vpand	%ymm1, %ymm5, %ymm1
	vpand	%ymm0, %ymm5, %ymm0
	vpackuswb	%ymm0, %ymm1, %ymm1
	vpermq	$216, %ymm1, %ymm1
	vmovdqa	%ymm1, -32(%rdx)
	cmpq	%rsi, %rdx
	jne	.L2
	leaq	bins(%rip), %rcx
	.p2align 5
	.p2align 4
	.p2align 3
.L3:
	movzbl	(%rax), %edx
	addq	$1, %rax
	addq	$1, (%rcx,%rdx,8)
	cmpq	%rsi, %rax
	jne	.L3
	leaq	bins(%rip), %rdx
	xorl	%eax, %eax
	xorl	%edi, %edi
	leaq	2048(%rdx), %r8
	.p2align 5
	.p2align 4
	.p2align 3
.L4:
	movq	%rax, %rcx
	movq	(%rdx), %rsi
	addq	$8, %rdx
	salq	$6, %rcx
	addq	%rax, %rcx
	addq	%rsi, %rdi
	leaq	(%rax,%rcx,2), %rax
	addq	%rsi, %rax
	cmpq	%r8, %rdx
	jne	.L4
	movl	$1, %edx
	cmpq	$262144, %rdi
	jne	.L1
	movabsq	$-7388273516099151790, %rdx
	cmpq	%rdx, %rax
	setne	%dl
	movzbl	%dl, %edx
	addl	%edx, %edx
.L1:
	movl	%edx, %eax
	vzeroupper
	ret
	.cfi_endproc
.LFE0:
	.size	main, .-main
	.local	bins
	.comm	bins,2048,32
	.local	bytes
	.comm	bytes,262144,32
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
	.ident	"GCC: (Debian 14.2.0-19) 14.2.0"
	.section	.note.GNU-stack,"",@progbits
