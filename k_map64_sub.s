	.file	"k_map64_sub.c"
	.text
	.p2align 4
	.globl	bench_setup
	.type	bench_setup, @function
bench_setup:
.LFB0:
	.cfi_startproc
	movl	$8, %esi
	leaq	a(%rip), %rax
	movabsq	$320255973501901, %rdi
	movl	$0, t(%rip)
	vmovd	%esi, %xmm8
	vmovq	%rdi, %xmm3
	movl	$19088743, %esi
	vmovdqa	.LC0(%rip), %ymm4
	vpbroadcastq	%xmm3, %ymm3
	vmovd	%esi, %xmm6
	vpbroadcastq	.LC5(%rip), %ymm5
	leaq	b(%rip), %rdx
	vpbroadcastd	%xmm8, %ymm8
	vpsrlq	$32, %ymm3, %ymm7
	vpbroadcastd	%xmm6, %ymm6
	leaq	8192(%rax), %rcx
	.p2align 4
	.p2align 3
.L2:
	vmovdqa	%ymm4, %ymm0
	addq	$64, %rax
	vpaddd	%ymm8, %ymm4, %ymm4
	addq	$64, %rdx
	vpmovzxdq	%xmm0, %ymm2
	vpsrlq	$32, %ymm2, %ymm1
	vpmuludq	%ymm3, %ymm2, %ymm9
	vpmuludq	%ymm3, %ymm1, %ymm1
	vpmuludq	%ymm2, %ymm7, %ymm2
	vpaddq	%ymm2, %ymm1, %ymm1
	vextracti128	$0x1, %ymm0, %xmm2
	vpermq	$216, %ymm0, %ymm0
	vpsllq	$32, %ymm1, %ymm1
	vpmovzxdq	%xmm2, %ymm2
	vpaddq	%ymm1, %ymm9, %ymm1
	vpmuludq	%ymm3, %ymm2, %ymm9
	vmovdqa	%ymm1, -64(%rax)
	vpsrlq	$32, %ymm2, %ymm1
	vpmuludq	%ymm2, %ymm7, %ymm2
	vpmuludq	%ymm3, %ymm1, %ymm1
	vpaddq	%ymm2, %ymm1, %ymm1
	vpsllq	$32, %ymm1, %ymm1
	vpaddq	%ymm1, %ymm9, %ymm1
	vmovdqa	%ymm1, -32(%rax)
	vpshufd	$80, %ymm0, %ymm1
	vpshufd	$250, %ymm0, %ymm0
	vpmuludq	%ymm6, %ymm1, %ymm1
	vpmuludq	%ymm6, %ymm0, %ymm0
	vpaddq	%ymm5, %ymm1, %ymm1
	vpaddq	%ymm5, %ymm0, %ymm0
	vmovdqa	%ymm1, -64(%rdx)
	vmovdqa	%ymm0, -32(%rdx)
	cmpq	%rcx, %rax
	jne	.L2
	vzeroupper
	ret
	.cfi_endproc
.LFE0:
	.size	bench_setup, .-bench_setup
	.p2align 4
	.globl	bench_run
	.type	bench_run, @function
bench_run:
.LFB1:
	.cfi_startproc
	xorl	%eax, %eax
	leaq	d(%rip), %rcx
	leaq	a(%rip), %rdx
	leaq	b(%rip), %rsi
	.p2align 5
	.p2align 4
	.p2align 3
.L6:
	vmovdqa	(%rdx,%rax), %ymm0
	vpsubq	(%rsi,%rax), %ymm0, %ymm0
	vmovdqa	%ymm0, (%rcx,%rax)
	addq	$32, %rax
	cmpq	$8192, %rax
	jne	.L6
	movl	t(%rip), %eax
	movl	%eax, %esi
	movl	%eax, %edi
	addl	$1, %eax
	andl	$1023, %esi
	orl	$1, %edi
	movl	%eax, t(%rip)
	addq	%rdi, (%rdx,%rsi,8)
	movq	(%rcx,%rsi,8), %rcx
	movq	%rcx, %rax
	vzeroupper
	ret
	.cfi_endproc
.LFE1:
	.size	bench_run, .-bench_run
	.local	t
	.comm	t,4,4
	.local	d
	.comm	d,8192,32
	.local	b
	.comm	b,8192,32
	.local	a
	.comm	a,8192,32
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
	.section	.rodata.cst8,"aM",@progbits,8
	.align 8
.LC5:
	.quad	133621201
	.ident	"GCC: (Debian 14.2.0-19) 14.2.0"
	.section	.note.GNU-stack,"",@progbits
