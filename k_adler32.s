	.file	"k_adler32.c"
	.text
	.p2align 4
	.globl	bench_setup
	.type	bench_setup, @function
bench_setup:
.LFB0:
	.cfi_startproc
	movl	$8, %ecx
	leaq	buf(%rip), %rax
	vmovdqa	.LC0(%rip), %ymm2
	vmovd	%ecx, %xmm9
	movl	$32, %ecx
	leaq	4096(%rax), %rdx
	vmovd	%ecx, %xmm8
	movl	$65535, %ecx
	vpbroadcastd	%xmm9, %ymm9
	vmovd	%ecx, %xmm1
	movl	$16, %ecx
	vpbroadcastd	%xmm8, %ymm8
	vmovd	%ecx, %xmm7
	movl	$24, %ecx
	vpbroadcastd	%xmm1, %ymm1
	vmovd	%ecx, %xmm6
	movl	$16711935, %ecx
	vpbroadcastd	%xmm7, %ymm7
	vmovd	%ecx, %xmm3
	movl	$-522133280, %ecx
	vpbroadcastd	%xmm6, %ymm6
	vmovd	%ecx, %xmm5
	movl	$117901063, %ecx
	vpbroadcastd	%xmm3, %ymm3
	vmovd	%ecx, %xmm4
	vpbroadcastd	%xmm5, %ymm5
	vpbroadcastd	%xmm4, %ymm4
	.p2align 4
	.p2align 3
.L2:
	vmovdqa	%ymm2, %ymm11
	vpaddd	%ymm9, %ymm2, %ymm0
	vpaddd	%ymm8, %ymm2, %ymm2
	addq	$32, %rax
	vpand	%ymm11, %ymm1, %ymm10
	vpand	%ymm0, %ymm1, %ymm0
	vpackusdw	%ymm0, %ymm10, %ymm10
	vpaddd	%ymm7, %ymm11, %ymm0
	vpaddd	%ymm6, %ymm11, %ymm11
	vpand	%ymm0, %ymm1, %ymm0
	vpand	%ymm11, %ymm1, %ymm11
	vpermq	$216, %ymm10, %ymm10
	vpackusdw	%ymm11, %ymm0, %ymm0
	vpand	%ymm10, %ymm3, %ymm10
	vpermq	$216, %ymm0, %ymm0
	vpand	%ymm0, %ymm3, %ymm0
	vpackuswb	%ymm0, %ymm10, %ymm0
	vpermq	$216, %ymm0, %ymm0
	vpsllw	$5, %ymm0, %ymm10
	vpand	%ymm10, %ymm5, %ymm10
	vpsubb	%ymm0, %ymm10, %ymm0
	vpaddb	%ymm4, %ymm0, %ymm0
	vmovdqa	%ymm0, -32(%rax)
	cmpq	%rax, %rdx
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
.LFB2:
	.cfi_startproc
	leaq	buf(%rip), %rdx
	xorl	%eax, %eax
	movl	$1, %ecx
	leaq	4096(%rdx), %rdi
	.p2align 4
	.p2align 4
	.p2align 3
.L6:
	movzbl	(%rdx), %esi
	addq	$1, %rdx
	addl	%esi, %ecx
	addl	%ecx, %eax
	cmpq	%rdi, %rdx
	jne	.L6
	movl	$2147975281, %esi
	movl	%eax, %edx
	imulq	%rsi, %rdx
	shrq	$47, %rdx
	imull	$65521, %edx, %edx
	subl	%edx, %eax
	movl	%ecx, %edx
	imulq	%rsi, %rdx
	sall	$16, %eax
	shrq	$47, %rdx
	imull	$65521, %edx, %edx
	subl	%edx, %ecx
	orl	%ecx, %eax
	ret
	.cfi_endproc
.LFE2:
	.size	bench_run, .-bench_run
	.local	buf
	.comm	buf,4096,32
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
