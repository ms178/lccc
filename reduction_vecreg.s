	.file	"reduction_vecreg.c"
	.text
	.p2align 4
	.type	dot_f32.constprop.0, @function
dot_f32.constprop.0:
.LFB25:
	.cfi_startproc
	xorl	%eax, %eax
	vxorps	%xmm0, %xmm0, %xmm0
	leaq	input_b(%rip), %rcx
	leaq	input_a(%rip), %rdx
	.p2align 4
	.p2align 3
.L2:
	vmovaps	(%rcx,%rax), %ymm1
	vmulps	(%rdx,%rax), %ymm1, %ymm2
	addq	$32, %rax
	vaddss	%xmm2, %xmm0, %xmm0
	vshufps	$85, %xmm2, %xmm2, %xmm3
	vextractf128	$0x1, %ymm2, %xmm1
	vaddss	%xmm0, %xmm3, %xmm0
	vunpckhps	%xmm2, %xmm2, %xmm3
	vaddss	%xmm0, %xmm3, %xmm3
	vshufps	$255, %xmm2, %xmm2, %xmm0
	vshufps	$85, %xmm1, %xmm1, %xmm2
	vaddss	%xmm3, %xmm0, %xmm0
	vaddss	%xmm1, %xmm0, %xmm0
	vaddss	%xmm2, %xmm0, %xmm0
	vunpckhps	%xmm1, %xmm1, %xmm2
	vshufps	$255, %xmm1, %xmm1, %xmm1
	vaddss	%xmm2, %xmm0, %xmm0
	vaddss	%xmm1, %xmm0, %xmm0
	cmpq	$262144, %rax
	jne	.L2
	vzeroupper
	ret
	.cfi_endproc
.LFE25:
	.size	dot_f32.constprop.0, .-dot_f32.constprop.0
	.p2align 4
	.type	sum_f32.constprop.0, @function
sum_f32.constprop.0:
.LFB26:
	.cfi_startproc
	leaq	input_a(%rip), %rax
	vxorps	%xmm0, %xmm0, %xmm0
	leaq	262144(%rax), %rdx
	.p2align 6
	.p2align 4
	.p2align 3
.L6:
	vaddss	(%rax), %xmm0, %xmm0
	addq	$32, %rax
	vaddss	-28(%rax), %xmm0, %xmm0
	vaddss	-24(%rax), %xmm0, %xmm0
	vaddss	-20(%rax), %xmm0, %xmm0
	vaddss	-16(%rax), %xmm0, %xmm0
	vaddss	-12(%rax), %xmm0, %xmm0
	vaddss	-8(%rax), %xmm0, %xmm0
	vaddss	-4(%rax), %xmm0, %xmm0
	cmpq	%rax, %rdx
	jne	.L6
	ret
	.cfi_endproc
.LFE26:
	.size	sum_f32.constprop.0, .-sum_f32.constprop.0
	.section	.rodata.str1.1,"aMS",@progbits,1
.LC9:
	.string	"%.0f\n"
	.section	.text.startup,"ax",@progbits
	.p2align 4
	.globl	main
	.type	main, @function
main:
.LFB24:
	.cfi_startproc
	leaq	8(%rsp), %r10
	.cfi_def_cfa 10, 0
	andq	$-32, %rsp
	movl	%edi, %eax
	movl	$5000, %edi
	pushq	-8(%r10)
	pushq	%rbp
	movq	%rsp, %rbp
	.cfi_escape 0x10,0x6,0x2,0x76,0
	pushq	%r10
	.cfi_escape 0xf,0x3,0x76,0x78,0x6
	subq	$8, %rsp
	cmpl	$1, %eax
	jle	.L9
	movq	8(%rsi), %rdi
	movl	$10, %edx
	xorl	%esi, %esi
	call	strtol@PLT
	movl	%eax, %edi
.L9:
	movl	$8, %edx
	xorl	%eax, %eax
	vmovdqa	.LC1(%rip), %ymm2
	vbroadcastss	.LC5(%rip), %ymm7
	vmovd	%edx, %xmm5
	movl	$15, %edx
	vbroadcastss	.LC8(%rip), %ymm6
	leaq	input_a(%rip), %rsi
	vmovd	%edx, %xmm4
	movl	$7, %edx
	leaq	input_b(%rip), %rcx
	vmovd	%edx, %xmm3
	vpbroadcastd	%xmm5, %ymm5
	vpbroadcastd	%xmm4, %ymm4
	vpbroadcastd	%xmm3, %ymm3
	.p2align 6
	.p2align 4
	.p2align 3
.L10:
	vmovdqa	%ymm2, %ymm0
	vpaddd	%ymm5, %ymm2, %ymm2
	vpand	%ymm4, %ymm0, %ymm1
	vpand	%ymm3, %ymm0, %ymm0
	vcvtdq2ps	%ymm1, %ymm1
	vcvtdq2ps	%ymm0, %ymm0
	vmulps	%ymm7, %ymm1, %ymm1
	vmulps	%ymm6, %ymm0, %ymm0
	vmovaps	%ymm1, (%rsi,%rax)
	vmovaps	%ymm0, (%rcx,%rax)
	addq	$32, %rax
	cmpq	$262144, %rax
	jne	.L10
	testl	%edi, %edi
	jle	.L16
	xorl	%esi, %esi
	vzeroupper
	.p2align 4
	.p2align 3
.L12:
	call	sum_f32.constprop.0
	addl	$1, %esi
	vmovss	%xmm0, sink(%rip)
	call	dot_f32.constprop.0
	vmovaps	%xmm0, %xmm1
	vmovss	sink(%rip), %xmm0
	vaddss	%xmm1, %xmm0, %xmm0
	vmovss	%xmm0, sink(%rip)
	cmpl	%esi, %edi
	jne	.L12
.L11:
	vmovss	sink(%rip), %xmm0
	leaq	.LC9(%rip), %rdi
	movl	$1, %eax
	vcvtss2sd	%xmm0, %xmm0, %xmm0
	call	printf@PLT
	xorl	%eax, %eax
	movl	$1, %edx
	vmovss	sink(%rip), %xmm0
	vucomiss	.LC10(%rip), %xmm0
	setp	%al
	cmovne	%edx, %eax
	addq	$8, %rsp
	popq	%r10
	.cfi_remember_state
	.cfi_def_cfa 10, 0
	popq	%rbp
	leaq	-8(%r10), %rsp
	.cfi_def_cfa 7, 8
	ret
.L16:
	.cfi_restore_state
	vzeroupper
	jmp	.L11
	.cfi_endproc
.LFE24:
	.size	main, .-main
	.local	sink
	.comm	sink,4,4
	.local	input_b
	.comm	input_b,262144,32
	.local	input_a
	.comm	input_a,262144,32
	.section	.rodata.cst32,"aM",@progbits,32
	.align 32
.LC1:
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
	.align 4
.LC8:
	.long	1040187392
	.align 4
.LC10:
	.long	1211564032
	.ident	"GCC: (Debian 14.2.0-19) 14.2.0"
	.section	.note.GNU-stack,"",@progbits
