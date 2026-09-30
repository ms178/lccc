	.file	"k_memchr.c"
	.text
	.p2align 4
	.globl	bench_setup
	.type	bench_setup, @function
bench_setup:
.LFB0:
	.cfi_startproc
	movl	$8, %ecx
	leaq	buf(%rip), %rax
	vmovdqa	.LC0(%rip), %ymm3
	vmovd	%ecx, %xmm9
	movl	$32, %ecx
	leaq	8192(%rax), %rdx
	vmovd	%ecx, %xmm8
	movl	$65535, %ecx
	vpbroadcastd	%xmm9, %ymm9
	vmovd	%ecx, %xmm2
	movl	$16, %ecx
	vpbroadcastd	%xmm8, %ymm8
	vmovd	%ecx, %xmm7
	movl	$24, %ecx
	vpbroadcastd	%xmm2, %ymm2
	vmovd	%ecx, %xmm6
	movl	$16711935, %ecx
	vpbroadcastd	%xmm7, %ymm7
	vmovd	%ecx, %xmm4
	movl	$16843009, %ecx
	vpbroadcastd	%xmm6, %ymm6
	vmovd	%ecx, %xmm5
	vpbroadcastd	%xmm4, %ymm4
	vpbroadcastd	%xmm5, %ymm5
	.p2align 4
	.p2align 3
.L2:
	vmovdqa	%ymm3, %ymm10
	vpaddd	%ymm9, %ymm3, %ymm1
	vpaddd	%ymm8, %ymm3, %ymm3
	addq	$32, %rax
	vpand	%ymm10, %ymm2, %ymm0
	vpand	%ymm1, %ymm2, %ymm1
	vpackusdw	%ymm1, %ymm0, %ymm0
	vpaddd	%ymm7, %ymm10, %ymm1
	vpaddd	%ymm6, %ymm10, %ymm10
	vpand	%ymm1, %ymm2, %ymm1
	vpand	%ymm10, %ymm2, %ymm10
	vpermq	$216, %ymm0, %ymm0
	vpackusdw	%ymm10, %ymm1, %ymm1
	vpand	%ymm0, %ymm4, %ymm0
	vpermq	$216, %ymm1, %ymm1
	vpand	%ymm1, %ymm4, %ymm1
	vpackuswb	%ymm1, %ymm0, %ymm0
	vpermq	$216, %ymm0, %ymm0
	vpor	%ymm0, %ymm5, %ymm0
	vmovdqa	%ymm0, -32(%rax)
	cmpq	%rax, %rdx
	jne	.L2
	movb	$0, 8175+buf(%rip)
	vzeroupper
	ret
	.cfi_endproc
.LFE0:
	.size	bench_setup, .-bench_setup
	.p2align 4
	.globl	bench_run
	.type	bench_run, @function
bench_run:
.LFB2:
	.cfi_startproc
	leaq	buf(%rip), %rcx
	movq	%rcx, %rax
	leaq	8192(%rcx), %rdx
	.p2align 4
	.p2align 4
	.p2align 3
.L7:
	cmpb	$0, (%rax)
	je	.L6
	addq	$1, %rax
	cmpq	%rax, %rdx
	jne	.L7
	xorl	%eax, %eax
	ret
	.p2align 4,,10
	.p2align 3
.L6:
	subq	%rcx, %rax
	ret
	.cfi_endproc
.LFE2:
	.size	bench_run, .-bench_run
	.local	buf
	.comm	buf,8192,32
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
