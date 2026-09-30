	.file	"k_hashmix.c"
	.text
	.p2align 4
	.globl	bench_setup
	.type	bench_setup, @function
bench_setup:
.LFB0:
	.cfi_startproc
	pushq	%rbp
	.cfi_def_cfa_offset 16
	.cfi_offset 6, -16
	movl	$8, %edx
	leaq	keys(%rip), %rax
	vmovd	%edx, %xmm7
	movl	$32, %edx
	leaq	4096(%rax), %rcx
	vpbroadcastd	%xmm7, %ymm7
	movq	%rsp, %rbp
	.cfi_def_cfa_register 6
	andq	$-32, %rsp
	subq	$328, %rsp
	vmovdqa	%ymm7, 168(%rsp)
	vmovdqa	.LC0(%rip), %ymm5
	vmovdqa	%ymm5, 264(%rsp)
	vmovd	%edx, %xmm5
	movl	$65535, %edx
	vpbroadcastd	%xmm5, %ymm5
	vmovd	%edx, %xmm7
	movl	$16, %edx
	vmovdqa	%ymm5, 136(%rsp)
	vpbroadcastd	%xmm7, %ymm7
	vmovd	%edx, %xmm5
	movl	$24, %edx
	vpbroadcastd	%xmm5, %ymm5
	vmovdqa	%ymm7, 104(%rsp)
	vmovd	%edx, %xmm7
	movl	$16711935, %edx
	vpbroadcastd	%xmm7, %ymm7
	vmovdqa	%ymm5, 72(%rsp)
	vmovd	%edx, %xmm5
	movl	$-117901064, %edx
	vmovdqa	%ymm7, 40(%rsp)
	vmovd	%edx, %xmm7
	movl	$218959117, %edx
	vpbroadcastd	%xmm5, %ymm5
	vmovd	%edx, %xmm4
	movl	$437918234, %edx
	vpbroadcastd	%xmm7, %ymm7
	vmovdqa	%ymm5, 8(%rsp)
	vmovd	%edx, %xmm5
	movl	$656877351, %edx
	vmovdqa	%ymm7, -24(%rsp)
	vpbroadcastd	%xmm4, %ymm4
	vmovd	%edx, %xmm7
	vpbroadcastd	%xmm5, %ymm5
	vmovdqa	%ymm4, -56(%rsp)
	vpbroadcastd	%xmm7, %ymm7
	vmovdqa	%ymm5, -88(%rsp)
	vmovdqa	%ymm7, -120(%rsp)
.L2:
	vmovdqa	264(%rsp), %ymm7
	vmovdqa	104(%rsp), %ymm6
	movl	$875836468, %edx
	addq	$512, %rax
	vpaddd	168(%rsp), %ymm7, %ymm1
	vmovd	%edx, %xmm9
	movl	$1094795585, %edx
	vpand	%ymm6, %ymm7, %ymm0
	vpaddd	40(%rsp), %ymm7, %ymm2
	vpbroadcastd	%xmm9, %ymm9
	vpaddd	136(%rsp), %ymm7, %ymm4
	vpand	%ymm6, %ymm1, %ymm1
	vpackusdw	%ymm1, %ymm0, %ymm0
	vpaddd	72(%rsp), %ymm7, %ymm1
	vmovd	%edx, %xmm7
	movl	$1313754702, %edx
	vmovd	%edx, %xmm11
	movl	$1532713819, %edx
	vpand	%ymm6, %ymm2, %ymm2
	vmovdqa	%ymm4, 264(%rsp)
	vpand	%ymm6, %ymm1, %ymm1
	vmovd	%edx, %xmm3
	movl	$1751672936, %edx
	vmovdqa	8(%rsp), %ymm4
	vpackusdw	%ymm2, %ymm1, %ymm1
	vmovd	%edx, %xmm5
	movl	$1970632053, %edx
	vmovd	%edx, %xmm2
	vpermq	$216, %ymm0, %ymm0
	movl	$-2105376126, %edx
	vpermq	$216, %ymm1, %ymm1
	vpand	%ymm4, %ymm0, %ymm0
	vmovd	%edx, %xmm8
	movl	$-1886417009, %edx
	vpand	%ymm4, %ymm1, %ymm1
	vmovd	%edx, %xmm6
	movl	$-1667457892, %edx
	vpackuswb	%ymm1, %ymm0, %ymm0
	vmovd	%edx, %xmm14
	movl	$-1448498775, %edx
	vpermq	$216, %ymm0, %ymm0
	vmovd	%edx, %xmm13
	movl	$-1229539658, %edx
	vpsllw	$3, %ymm0, %ymm1
	vpand	-24(%rsp), %ymm1, %ymm1
	vmovd	%edx, %xmm12
	movl	$-1010580541, %edx
	vpbroadcastd	%xmm5, %ymm5
	vpbroadcastd	%xmm2, %ymm2
	vpbroadcastd	%xmm7, %ymm7
	vpsubb	%ymm0, %ymm1, %ymm1
	vmovd	%edx, %xmm0
	vpaddb	-56(%rsp), %ymm1, %ymm4
	vpbroadcastd	%xmm0, %ymm0
	vpaddb	%ymm5, %ymm1, %ymm5
	vpaddb	-88(%rsp), %ymm1, %ymm10
	vpaddb	%ymm0, %ymm1, %ymm0
	vpaddb	%ymm2, %ymm1, %ymm2
	vpbroadcastd	%xmm11, %ymm11
	vpbroadcastd	%xmm3, %ymm3
	vpbroadcastd	%xmm8, %ymm8
	vpbroadcastd	%xmm6, %ymm6
	vmovdqa	%ymm0, 296(%rsp)
	vpbroadcastd	%xmm14, %ymm14
	vpunpcklbw	%ymm5, %ymm1, %ymm0
	vpbroadcastd	%xmm13, %ymm13
	vpunpckhbw	%ymm5, %ymm1, %ymm5
	vpbroadcastd	%xmm12, %ymm12
	vpaddb	-120(%rsp), %ymm1, %ymm15
	vpaddb	%ymm8, %ymm1, %ymm8
	vpaddb	%ymm6, %ymm1, %ymm6
	vpaddb	%ymm14, %ymm1, %ymm14
	vpaddb	%ymm13, %ymm1, %ymm13
	vpaddb	%ymm12, %ymm1, %ymm12
	vpaddb	%ymm9, %ymm1, %ymm9
	vpaddb	%ymm7, %ymm1, %ymm7
	vpaddb	%ymm11, %ymm1, %ymm11
	vpaddb	%ymm3, %ymm1, %ymm3
	vperm2i128	$32, %ymm5, %ymm0, %ymm1
	vperm2i128	$49, %ymm5, %ymm0, %ymm0
	vpunpcklbw	%ymm2, %ymm4, %ymm5
	vpunpckhbw	%ymm2, %ymm4, %ymm4
	vperm2i128	$32, %ymm4, %ymm5, %ymm2
	vperm2i128	$49, %ymm4, %ymm5, %ymm5
	vpunpcklbw	%ymm8, %ymm10, %ymm4
	vpunpckhbw	%ymm8, %ymm10, %ymm10
	vperm2i128	$32, %ymm10, %ymm4, %ymm8
	vperm2i128	$49, %ymm10, %ymm4, %ymm4
	vpunpcklbw	%ymm6, %ymm15, %ymm10
	vpunpckhbw	%ymm6, %ymm15, %ymm15
	vperm2i128	$32, %ymm15, %ymm10, %ymm6
	vperm2i128	$49, %ymm15, %ymm10, %ymm15
	vpunpcklbw	%ymm14, %ymm9, %ymm10
	vmovdqa	%ymm15, 232(%rsp)
	vpunpckhbw	%ymm14, %ymm9, %ymm9
	vperm2i128	$32, %ymm9, %ymm10, %ymm14
	vperm2i128	$49, %ymm9, %ymm10, %ymm10
	vpunpcklbw	%ymm13, %ymm7, %ymm9
	vpunpckhbw	%ymm13, %ymm7, %ymm7
	vperm2i128	$32, %ymm7, %ymm9, %ymm13
	vperm2i128	$49, %ymm7, %ymm9, %ymm9
	vpunpcklbw	%ymm12, %ymm11, %ymm7
	vpunpckhbw	%ymm12, %ymm11, %ymm11
	vperm2i128	$32, %ymm11, %ymm7, %ymm12
	vperm2i128	$49, %ymm11, %ymm7, %ymm7
	vmovdqa	296(%rsp), %ymm11
	vpunpcklbw	%ymm11, %ymm3, %ymm15
	vpunpckhbw	%ymm11, %ymm3, %ymm3
	vperm2i128	$32, %ymm3, %ymm15, %ymm11
	vperm2i128	$49, %ymm3, %ymm15, %ymm15
	vpunpcklbw	%ymm14, %ymm1, %ymm3
	vpunpckhbw	%ymm14, %ymm1, %ymm1
	vperm2i128	$32, %ymm1, %ymm3, %ymm14
	vperm2i128	$49, %ymm1, %ymm3, %ymm1
	vpunpcklbw	%ymm10, %ymm0, %ymm3
	vpunpckhbw	%ymm10, %ymm0, %ymm0
	vperm2i128	$32, %ymm0, %ymm3, %ymm10
	vperm2i128	$49, %ymm0, %ymm3, %ymm0
	vpunpcklbw	%ymm13, %ymm2, %ymm3
	vpunpckhbw	%ymm13, %ymm2, %ymm2
	vperm2i128	$32, %ymm2, %ymm3, %ymm13
	vperm2i128	$49, %ymm2, %ymm3, %ymm2
	vpunpcklbw	%ymm9, %ymm5, %ymm3
	vpunpckhbw	%ymm9, %ymm5, %ymm5
	vperm2i128	$32, %ymm5, %ymm3, %ymm9
	vperm2i128	$49, %ymm5, %ymm3, %ymm5
	vpunpcklbw	%ymm7, %ymm4, %ymm3
	vmovdqa	%ymm5, 296(%rsp)
	vpunpckhbw	%ymm7, %ymm4, %ymm4
	vpunpcklbw	%ymm12, %ymm8, %ymm5
	vpunpckhbw	%ymm12, %ymm8, %ymm8
	vperm2i128	$32, %ymm4, %ymm3, %ymm7
	vperm2i128	$49, %ymm4, %ymm3, %ymm4
	vpunpcklbw	%ymm11, %ymm6, %ymm3
	vpunpckhbw	%ymm11, %ymm6, %ymm6
	vperm2i128	$32, %ymm8, %ymm5, %ymm12
	vperm2i128	$49, %ymm8, %ymm5, %ymm5
	vperm2i128	$32, %ymm6, %ymm3, %ymm8
	vperm2i128	$49, %ymm6, %ymm3, %ymm3
	vmovdqa	232(%rsp), %ymm6
	vpunpcklbw	%ymm15, %ymm6, %ymm11
	vpunpckhbw	%ymm15, %ymm6, %ymm6
	vperm2i128	$32, %ymm6, %ymm11, %ymm15
	vmovdqa	%ymm15, 232(%rsp)
	vperm2i128	$49, %ymm6, %ymm11, %ymm15
	vpunpcklbw	%ymm12, %ymm14, %ymm6
	vpunpckhbw	%ymm12, %ymm14, %ymm14
	vpunpcklbw	%ymm5, %ymm1, %ymm12
	vpunpckhbw	%ymm5, %ymm1, %ymm1
	vperm2i128	$32, %ymm14, %ymm6, %ymm11
	vperm2i128	$32, %ymm1, %ymm12, %ymm5
	vperm2i128	$49, %ymm1, %ymm12, %ymm1
	vpunpcklbw	%ymm7, %ymm10, %ymm12
	vpunpckhbw	%ymm7, %ymm10, %ymm10
	vperm2i128	$49, %ymm14, %ymm6, %ymm6
	vperm2i128	$32, %ymm10, %ymm12, %ymm7
	vperm2i128	$49, %ymm10, %ymm12, %ymm10
	vpunpcklbw	%ymm4, %ymm0, %ymm12
	vpunpckhbw	%ymm4, %ymm0, %ymm0
	vperm2i128	$32, %ymm0, %ymm12, %ymm4
	vperm2i128	$49, %ymm0, %ymm12, %ymm0
	vpunpcklbw	%ymm8, %ymm13, %ymm12
	vmovdqa	%ymm0, 200(%rsp)
	vpunpckhbw	%ymm8, %ymm13, %ymm8
	vmovdqa	232(%rsp), %ymm13
	vmovdqa	296(%rsp), %ymm0
	vperm2i128	$32, %ymm8, %ymm12, %ymm14
	vperm2i128	$49, %ymm8, %ymm12, %ymm8
	vpunpcklbw	%ymm3, %ymm2, %ymm12
	vpunpckhbw	%ymm3, %ymm2, %ymm2
	vperm2i128	$32, %ymm2, %ymm12, %ymm3
	vperm2i128	$49, %ymm2, %ymm12, %ymm12
	vpunpcklbw	%ymm13, %ymm9, %ymm2
	vpunpckhbw	%ymm13, %ymm9, %ymm9
	vperm2i128	$32, %ymm9, %ymm2, %ymm13
	vperm2i128	$49, %ymm9, %ymm2, %ymm9
	vpunpcklbw	%ymm15, %ymm0, %ymm2
	vpunpckhbw	%ymm15, %ymm0, %ymm15
	vperm2i128	$32, %ymm15, %ymm2, %ymm0
	vperm2i128	$49, %ymm15, %ymm2, %ymm15
	vpunpcklbw	%ymm14, %ymm11, %ymm2
	vpunpckhbw	%ymm14, %ymm11, %ymm11
	vperm2i128	$32, %ymm11, %ymm2, %ymm14
	vperm2i128	$49, %ymm11, %ymm2, %ymm2
	vmovdqa	%ymm2, -480(%rax)
	vpunpcklbw	%ymm8, %ymm6, %ymm2
	vpunpckhbw	%ymm8, %ymm6, %ymm6
	vperm2i128	$32, %ymm6, %ymm2, %ymm8
	vperm2i128	$49, %ymm6, %ymm2, %ymm2
	vmovdqa	%ymm14, -512(%rax)
	vmovdqa	%ymm2, -416(%rax)
	vpunpcklbw	%ymm3, %ymm5, %ymm2
	vpunpckhbw	%ymm3, %ymm5, %ymm5
	vperm2i128	$32, %ymm5, %ymm2, %ymm3
	vperm2i128	$49, %ymm5, %ymm2, %ymm2
	vmovdqa	%ymm8, -448(%rax)
	vmovdqa	%ymm2, -352(%rax)
	vpunpcklbw	%ymm12, %ymm1, %ymm2
	vpunpckhbw	%ymm12, %ymm1, %ymm1
	vmovdqa	%ymm3, -384(%rax)
	vperm2i128	$32, %ymm1, %ymm2, %ymm3
	vperm2i128	$49, %ymm1, %ymm2, %ymm2
	vpunpcklbw	%ymm13, %ymm7, %ymm1
	vpunpckhbw	%ymm13, %ymm7, %ymm7
	vmovdqa	%ymm2, -288(%rax)
	vperm2i128	$32, %ymm7, %ymm1, %ymm2
	vperm2i128	$49, %ymm7, %ymm1, %ymm1
	vmovdqa	%ymm3, -320(%rax)
	vmovdqa	%ymm1, -224(%rax)
	vpunpcklbw	%ymm9, %ymm10, %ymm1
	vpunpckhbw	%ymm9, %ymm10, %ymm10
	vmovdqa	%ymm2, -256(%rax)
	vperm2i128	$32, %ymm10, %ymm1, %ymm2
	vperm2i128	$49, %ymm10, %ymm1, %ymm1
	vmovdqa	%ymm2, -192(%rax)
	vmovdqa	%ymm1, -160(%rax)
	vpunpcklbw	%ymm0, %ymm4, %ymm1
	vpunpckhbw	%ymm0, %ymm4, %ymm4
	vperm2i128	$32, %ymm4, %ymm1, %ymm2
	vperm2i128	$49, %ymm4, %ymm1, %ymm1
	vmovdqa	200(%rsp), %ymm4
	vmovdqa	%ymm1, -96(%rax)
	vpunpckhbw	%ymm15, %ymm4, %ymm0
	vpunpcklbw	%ymm15, %ymm4, %ymm1
	vmovdqa	%ymm2, -128(%rax)
	vperm2i128	$32, %ymm0, %ymm1, %ymm2
	vperm2i128	$49, %ymm0, %ymm1, %ymm1
	vmovdqa	%ymm2, -64(%rax)
	vmovdqa	%ymm1, -32(%rax)
	cmpq	%rcx, %rax
	jne	.L2
	vzeroupper
	leave
	.cfi_def_cfa 7, 8
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
	pushq	%rbp
	.cfi_def_cfa_offset 16
	.cfi_offset 6, -16
	movl	$16711935, %ecx
	vpxor	%xmm5, %xmm5, %xmm5
	leaq	keys(%rip), %rax
	vmovd	%ecx, %xmm14
	leaq	4096(%rax), %rdx
	movabsq	$1469598103934665603, %rcx
	vpbroadcastd	%xmm14, %ymm14
	movq	%rsp, %rbp
	.cfi_def_cfa_register 6
	andq	$-32, %rsp
	subq	$1608, %rsp
	vmovdqa	%ymm5, 392(%rsp)
	vmovq	%rcx, %xmm5
	vpbroadcastq	%xmm5, %ymm7
	vmovdqa	%ymm7, 360(%rsp)
.L7:
	vmovdqa	(%rax), %ymm2
	vmovdqa	32(%rax), %ymm12
	addq	$512, %rax
	vmovdqa	-448(%rax), %ymm4
	vmovdqa	-416(%rax), %ymm7
	vmovdqa	-384(%rax), %ymm1
	vmovdqa	-352(%rax), %ymm11
	vpand	%ymm12, %ymm14, %ymm13
	vpand	%ymm2, %ymm14, %ymm8
	vpsrlw	$8, %ymm12, %ymm12
	vpsrlw	$8, %ymm2, %ymm2
	vpackuswb	%ymm13, %ymm8, %ymm8
	vmovdqa	-320(%rax), %ymm3
	vmovdqa	-288(%rax), %ymm6
	vpand	%ymm7, %ymm14, %ymm13
	vpackuswb	%ymm12, %ymm2, %ymm2
	vpand	%ymm4, %ymm14, %ymm12
	vpsrlw	$8, %ymm7, %ymm7
	vpackuswb	%ymm13, %ymm12, %ymm12
	vpand	%ymm11, %ymm14, %ymm13
	vmovdqa	-256(%rax), %ymm0
	vpsrlw	$8, %ymm4, %ymm4
	vmovdqa	-224(%rax), %ymm10
	vpsrlw	$8, %ymm11, %ymm11
	vmovdqa	-192(%rax), %ymm5
	vpackuswb	%ymm7, %ymm4, %ymm4
	vpand	%ymm1, %ymm14, %ymm7
	vmovdqa	-160(%rax), %ymm9
	vpermq	$216, %ymm8, %ymm8
	vpsrlw	$8, %ymm1, %ymm1
	vpackuswb	%ymm13, %ymm7, %ymm7
	vpand	%ymm6, %ymm14, %ymm13
	vmovdqa	-32(%rax), %ymm15
	vpackuswb	%ymm11, %ymm1, %ymm1
	vpsrlw	$8, %ymm6, %ymm6
	vpand	%ymm3, %ymm14, %ymm11
	vpsrlw	$8, %ymm3, %ymm3
	vpackuswb	%ymm13, %ymm11, %ymm11
	vpand	%ymm10, %ymm14, %ymm13
	vpackuswb	%ymm6, %ymm3, %ymm3
	vpsrlw	$8, %ymm10, %ymm10
	vpand	%ymm0, %ymm14, %ymm6
	vpsrlw	$8, %ymm0, %ymm0
	vpackuswb	%ymm13, %ymm6, %ymm6
	vpand	%ymm9, %ymm14, %ymm13
	vpackuswb	%ymm10, %ymm0, %ymm0
	vpsrlw	$8, %ymm9, %ymm9
	vpand	%ymm5, %ymm14, %ymm10
	vpsrlw	$8, %ymm5, %ymm5
	vpackuswb	%ymm13, %ymm10, %ymm10
	vmovdqa	-96(%rax), %ymm13
	vpermq	$216, %ymm12, %ymm12
	vpackuswb	%ymm9, %ymm5, %ymm5
	vpand	-96(%rax), %ymm14, %ymm9
	vpermq	$216, %ymm7, %ymm7
	vpermq	$216, %ymm5, %ymm5
	vpsrlw	$8, %ymm13, %ymm13
	vpermq	$216, %ymm11, %ymm11
	vpermq	$216, %ymm6, %ymm6
	vpermq	$216, %ymm10, %ymm10
	vpermq	$216, %ymm2, %ymm2
	vmovdqa	%ymm5, 1544(%rsp)
	vpand	-128(%rax), %ymm14, %ymm5
	vpermq	$216, %ymm4, %ymm4
	vpermq	$216, %ymm1, %ymm1
	vpermq	$216, %ymm3, %ymm3
	vpermq	$216, %ymm0, %ymm0
	vpackuswb	%ymm9, %ymm5, %ymm5
	vmovdqa	-128(%rax), %ymm9
	vpermq	$216, %ymm5, %ymm5
	vpsrlw	$8, %ymm9, %ymm9
	vpackuswb	%ymm13, %ymm9, %ymm9
	vpermq	$216, %ymm9, %ymm13
	vpand	-64(%rax), %ymm14, %ymm9
	vmovdqa	%ymm13, 1512(%rsp)
	vpand	%ymm15, %ymm14, %ymm13
	vpsrlw	$8, %ymm15, %ymm15
	vpackuswb	%ymm13, %ymm9, %ymm9
	vmovdqa	-64(%rax), %ymm13
	vpermq	$216, %ymm9, %ymm9
	vpsrlw	$8, %ymm13, %ymm13
	vpackuswb	%ymm15, %ymm13, %ymm13
	vpermq	$216, %ymm13, %ymm15
	vpand	%ymm8, %ymm14, %ymm13
	vmovdqa	%ymm15, 1576(%rsp)
	vpsrlw	$8, %ymm8, %ymm8
	vpand	%ymm12, %ymm14, %ymm15
	vpsrlw	$8, %ymm12, %ymm12
	vpackuswb	%ymm15, %ymm13, %ymm13
	vpackuswb	%ymm12, %ymm8, %ymm8
	vpand	%ymm11, %ymm14, %ymm12
	vpermq	$216, %ymm13, %ymm13
	vpermq	$216, %ymm8, %ymm15
	vpsrlw	$8, %ymm11, %ymm11
	vpand	%ymm7, %ymm14, %ymm8
	vpsrlw	$8, %ymm7, %ymm7
	vpackuswb	%ymm12, %ymm8, %ymm8
	vpackuswb	%ymm11, %ymm7, %ymm7
	vpermq	$216, %ymm8, %ymm12
	vpand	%ymm10, %ymm14, %ymm8
	vpermq	$216, %ymm7, %ymm11
	vpsrlw	$8, %ymm10, %ymm10
	vpand	%ymm6, %ymm14, %ymm7
	vpsrlw	$8, %ymm6, %ymm6
	vpackuswb	%ymm8, %ymm7, %ymm7
	vpand	%ymm9, %ymm14, %ymm8
	vpackuswb	%ymm10, %ymm6, %ymm6
	vpsrlw	$8, %ymm9, %ymm9
	vpermq	$216, %ymm7, %ymm7
	vpermq	$216, %ymm6, %ymm10
	vpand	%ymm5, %ymm14, %ymm6
	vpsrlw	$8, %ymm5, %ymm5
	vpackuswb	%ymm8, %ymm6, %ymm6
	vpand	%ymm4, %ymm14, %ymm8
	vpackuswb	%ymm9, %ymm5, %ymm5
	vpsrlw	$8, %ymm4, %ymm4
	vpermq	$216, %ymm6, %ymm6
	vpermq	$216, %ymm5, %ymm9
	vpand	%ymm2, %ymm14, %ymm5
	vpsrlw	$8, %ymm2, %ymm2
	vpackuswb	%ymm8, %ymm5, %ymm5
	vpackuswb	%ymm4, %ymm2, %ymm2
	vpand	%ymm3, %ymm14, %ymm4
	vpermq	$216, %ymm5, %ymm5
	vpermq	$216, %ymm2, %ymm2
	vpsrlw	$8, %ymm3, %ymm3
	vmovdqa	%ymm2, 1480(%rsp)
	vpand	%ymm1, %ymm14, %ymm2
	vpsrlw	$8, %ymm1, %ymm1
	vpackuswb	%ymm4, %ymm2, %ymm2
	vpackuswb	%ymm3, %ymm1, %ymm1
	vmovdqa	1544(%rsp), %ymm4
	vpermq	$216, %ymm1, %ymm8
	vpand	%ymm0, %ymm14, %ymm1
	vpermq	$216, %ymm2, %ymm2
	vpand	%ymm4, %ymm14, %ymm3
	vpsrlw	$8, %ymm0, %ymm0
	vpackuswb	%ymm3, %ymm1, %ymm1
	vpsrlw	$8, %ymm4, %ymm3
	vmovdqa	1512(%rsp), %ymm4
	vpackuswb	%ymm3, %ymm0, %ymm0
	vpermq	$216, %ymm1, %ymm1
	vpermq	$216, %ymm0, %ymm3
	vpand	%ymm4, %ymm14, %ymm0
	vmovdqa	%ymm3, 1448(%rsp)
	vpand	1576(%rsp), %ymm14, %ymm3
	vpackuswb	%ymm3, %ymm0, %ymm0
	vpsrlw	$8, %ymm4, %ymm3
	vmovdqa	1576(%rsp), %ymm4
	vpermq	$216, %ymm0, %ymm0
	vpsrlw	$8, %ymm4, %ymm4
	vpackuswb	%ymm4, %ymm3, %ymm3
	vpermq	$216, %ymm3, %ymm4
	vpand	%ymm13, %ymm14, %ymm3
	vmovdqa	%ymm4, 1576(%rsp)
	vpand	%ymm12, %ymm14, %ymm4
	vpsrlw	$8, %ymm12, %ymm12
	vpackuswb	%ymm4, %ymm3, %ymm3
	vpermq	$216, %ymm3, %ymm4
	vpsrlw	$8, %ymm13, %ymm3
	vmovdqa	%ymm4, 1544(%rsp)
	vpackuswb	%ymm12, %ymm3, %ymm3
	vpand	%ymm6, %ymm14, %ymm4
	vpermq	$216, %ymm3, %ymm13
	vpsrlw	$8, %ymm6, %ymm6
	vpand	%ymm7, %ymm14, %ymm3
	vpackuswb	%ymm4, %ymm3, %ymm3
	vpsrlw	$8, %ymm7, %ymm7
	vpand	%ymm2, %ymm14, %ymm4
	vpermq	$216, %ymm3, %ymm12
	vpsrlw	$8, %ymm2, %ymm2
	vpand	%ymm5, %ymm14, %ymm3
	vpsrlw	$8, %ymm5, %ymm5
	vpackuswb	%ymm4, %ymm3, %ymm3
	vpand	%ymm0, %ymm14, %ymm4
	vpackuswb	%ymm2, %ymm5, %ymm5
	vpsrlw	$8, %ymm0, %ymm0
	vpand	%ymm1, %ymm14, %ymm2
	vpsrlw	$8, %ymm1, %ymm1
	vpackuswb	%ymm6, %ymm7, %ymm7
	vpackuswb	%ymm4, %ymm2, %ymm2
	vpackuswb	%ymm0, %ymm1, %ymm1
	vpermq	$216, %ymm7, %ymm6
	vpand	%ymm15, %ymm14, %ymm0
	vpermq	$216, %ymm1, %ymm7
	vpand	%ymm11, %ymm14, %ymm1
	vpand	%ymm9, %ymm14, %ymm4
	vpackuswb	%ymm1, %ymm0, %ymm0
	vpsrlw	$8, %ymm11, %ymm11
	vmovdqa	%ymm7, 1512(%rsp)
	vpand	%ymm8, %ymm14, %ymm7
	vpsrlw	$8, %ymm15, %ymm1
	vpsrlw	$8, %ymm9, %ymm9
	vpermq	$216, %ymm3, %ymm3
	vmovdqa	1480(%rsp), %ymm15
	vpackuswb	%ymm11, %ymm1, %ymm1
	vpsrlw	$8, %ymm8, %ymm8
	vpermq	$216, %ymm2, %ymm2
	vpermq	$216, %ymm1, %ymm11
	vpand	%ymm10, %ymm14, %ymm1
	vpermq	$216, %ymm0, %ymm0
	vpackuswb	%ymm4, %ymm1, %ymm1
	vpsrlw	$8, %ymm10, %ymm4
	vpermq	$216, %ymm5, %ymm5
	vpackuswb	%ymm9, %ymm4, %ymm4
	vpermq	$216, %ymm1, %ymm1
	vmovdqa	1448(%rsp), %ymm9
	vpermq	$216, %ymm4, %ymm10
	vpand	%ymm15, %ymm14, %ymm4
	vpackuswb	%ymm7, %ymm4, %ymm4
	vpsrlw	$8, %ymm15, %ymm7
	vpackuswb	%ymm8, %ymm7, %ymm7
	vpermq	$216, %ymm4, %ymm4
	vpand	1576(%rsp), %ymm14, %ymm8
	vpermq	$216, %ymm7, %ymm15
	vpand	%ymm9, %ymm14, %ymm7
	vpackuswb	%ymm8, %ymm7, %ymm7
	vpsrlw	$8, %ymm9, %ymm8
	vmovdqa	1576(%rsp), %ymm9
	vpermq	$216, %ymm7, %ymm7
	vpsrlw	$8, %ymm9, %ymm9
	vpackuswb	%ymm9, %ymm8, %ymm8
	vpermq	$216, %ymm8, %ymm9
	vpand	1544(%rsp), %ymm14, %ymm8
	vmovdqa	%ymm9, 1576(%rsp)
	vpand	%ymm12, %ymm14, %ymm9
	vpsrlw	$8, %ymm12, %ymm12
	vpackuswb	%ymm9, %ymm8, %ymm8
	vmovdqa	1544(%rsp), %ymm9
	vpermq	$216, %ymm8, %ymm8
	vmovdqa	%ymm8, 1480(%rsp)
	vpsrlw	$8, %ymm9, %ymm8
	vpand	%ymm2, %ymm14, %ymm9
	vpackuswb	%ymm12, %ymm8, %ymm8
	vpsrlw	$8, %ymm2, %ymm2
	vpermq	$216, %ymm8, %ymm12
	vpand	%ymm3, %ymm14, %ymm8
	vpsrlw	$8, %ymm3, %ymm3
	vpackuswb	%ymm9, %ymm8, %ymm8
	vpackuswb	%ymm2, %ymm3, %ymm3
	vpand	%ymm0, %ymm14, %ymm2
	vpermq	$216, %ymm8, %ymm8
	vpermq	$216, %ymm3, %ymm9
	vpsrlw	$8, %ymm0, %ymm0
	vpand	%ymm1, %ymm14, %ymm3
	vpsrlw	$8, %ymm1, %ymm1
	vpackuswb	%ymm3, %ymm2, %ymm2
	vpackuswb	%ymm1, %ymm0, %ymm0
	vpand	%ymm7, %ymm14, %ymm1
	vpermq	$216, %ymm2, %ymm2
	vpermq	$216, %ymm0, %ymm3
	vpsrlw	$8, %ymm7, %ymm7
	vpand	%ymm4, %ymm14, %ymm0
	vmovdqa	%ymm3, 1032(%rsp)
	vpackuswb	%ymm1, %ymm0, %ymm0
	vpand	%ymm6, %ymm14, %ymm3
	vpand	%ymm13, %ymm14, %ymm1
	vpackuswb	%ymm3, %ymm1, %ymm1
	vpsrlw	$8, %ymm6, %ymm6
	vpermq	$216, %ymm0, %ymm0
	vpsrlw	$8, %ymm13, %ymm3
	vpsrlw	$8, %ymm4, %ymm4
	vpermq	$216, %ymm1, %ymm1
	vpackuswb	%ymm6, %ymm3, %ymm3
	vpackuswb	%ymm7, %ymm4, %ymm4
	vmovdqa	1512(%rsp), %ymm6
	vpermq	$216, %ymm4, %ymm7
	vpermq	$216, %ymm3, %ymm13
	vpand	%ymm5, %ymm14, %ymm3
	vpand	%ymm6, %ymm14, %ymm4
	vpackuswb	%ymm4, %ymm3, %ymm3
	vpsrlw	$8, %ymm5, %ymm4
	vpsrlw	$8, %ymm6, %ymm5
	vpermq	$216, %ymm3, %ymm3
	vpackuswb	%ymm5, %ymm4, %ymm4
	vpermq	$216, %ymm4, %ymm5
	vpand	%ymm11, %ymm14, %ymm4
	vmovdqa	%ymm5, 712(%rsp)
	vpand	%ymm10, %ymm14, %ymm5
	vpsrlw	$8, %ymm10, %ymm10
	vpackuswb	%ymm5, %ymm4, %ymm4
	vpsrlw	$8, %ymm11, %ymm5
	vmovdqa	1576(%rsp), %ymm11
	vpackuswb	%ymm10, %ymm5, %ymm5
	vpermq	$216, %ymm4, %ymm4
	vpermq	$216, %ymm5, %ymm10
	vpand	%ymm11, %ymm14, %ymm6
	vpand	%ymm15, %ymm14, %ymm5
	vmovdqa	%ymm10, 584(%rsp)
	vpackuswb	%ymm6, %ymm5, %ymm5
	vpsrlw	$8, %ymm11, %ymm10
	vpsrlw	$8, %ymm15, %ymm6
	vpermq	$216, %ymm5, %ymm5
	vpackuswb	%ymm10, %ymm6, %ymm6
	vpermq	$216, %ymm6, %ymm15
	vmovdqa	1480(%rsp), %ymm6
	vpmovzxbw	%xmm6, %ymm10
	vextracti128	$0x1, %ymm6, %xmm6
	vpmovzxwd	%xmm10, %ymm11
	vextracti128	$0x1, %ymm10, %xmm10
	vpmovzxbw	%xmm6, %ymm6
	vpmovzxwd	%xmm10, %ymm10
	vmovdqa	%ymm10, 104(%rsp)
	vpmovzxwd	%xmm6, %ymm10
	vextracti128	$0x1, %ymm6, %xmm6
	vmovdqa	%ymm10, 136(%rsp)
	vpmovzxwd	%xmm6, %ymm6
	vpmovzxbw	%xmm8, %ymm10
	vmovdqa	%ymm6, 168(%rsp)
	vextracti128	$0x1, %ymm8, %xmm6
	vpmovzxwd	%xmm10, %ymm8
	vextracti128	$0x1, %ymm10, %xmm10
	vpmovzxbw	%xmm6, %ymm6
	vpmovzxwd	%xmm10, %ymm10
	vmovdqa	%ymm8, 1576(%rsp)
	vpmovzxwd	%xmm6, %ymm8
	vextracti128	$0x1, %ymm6, %xmm6
	vmovdqa	%ymm8, 200(%rsp)
	vpmovzxwd	%xmm6, %ymm6
	vmovdqa	%ymm6, 232(%rsp)
	vpmovzxbw	%xmm2, %ymm6
	vextracti128	$0x1, %ymm2, %xmm2
	vpmovzxwd	%xmm6, %ymm8
	vextracti128	$0x1, %ymm6, %xmm6
	vpmovzxbw	%xmm2, %ymm2
	vpmovzxwd	%xmm6, %ymm6
	vmovdqa	%ymm6, 264(%rsp)
	vpmovzxwd	%xmm2, %ymm6
	vextracti128	$0x1, %ymm2, %xmm2
	vmovdqa	%ymm6, 296(%rsp)
	vpmovzxwd	%xmm2, %ymm2
	vmovdqa	%ymm2, 328(%rsp)
	vpmovzxbw	%xmm0, %ymm2
	vextracti128	$0x1, %ymm0, %xmm0
	vpmovzxwd	%xmm2, %ymm6
	vextracti128	$0x1, %ymm2, %xmm2
	vpmovzxbw	%xmm0, %ymm0
	vpmovzxwd	%xmm2, %ymm2
	vmovdqa	%ymm2, 72(%rsp)
	vpmovzxwd	%xmm0, %ymm2
	vextracti128	$0x1, %ymm0, %xmm0
	vmovdqa	%ymm2, 40(%rsp)
	vpmovzxwd	%xmm0, %ymm0
	vpmovzxbw	%xmm1, %ymm2
	vmovdqa	%ymm0, 1544(%rsp)
	vextracti128	$0x1, %ymm1, %xmm0
	vpmovzxwd	%xmm2, %ymm1
	vextracti128	$0x1, %ymm2, %xmm2
	vmovdqa	%ymm1, 1512(%rsp)
	vpmovzxbw	%xmm0, %ymm0
	vpmovzxwd	%xmm2, %ymm1
	vmovdqa	%ymm1, 8(%rsp)
	vpmovzxwd	%xmm0, %ymm1
	vextracti128	$0x1, %ymm0, %xmm0
	vpmovzxwd	%xmm0, %ymm0
	vmovdqa	%ymm1, -24(%rsp)
	vpmovzxbw	%xmm3, %ymm1
	vmovdqa	%ymm0, 1480(%rsp)
	vextracti128	$0x1, %ymm3, %xmm0
	vpmovzxwd	%xmm1, %ymm3
	vextracti128	$0x1, %ymm1, %xmm1
	vpmovzxbw	%xmm0, %ymm0
	vpmovzxwd	%xmm1, %ymm1
	vpmovzxwd	%xmm0, %ymm2
	vextracti128	$0x1, %ymm0, %xmm0
	vmovdqa	%ymm1, -56(%rsp)
	vpmovzxbw	%xmm4, %ymm1
	vmovdqa	%ymm2, 1448(%rsp)
	vpmovzxwd	%xmm0, %ymm0
	vmovdqa	%ymm0, 1416(%rsp)
	vextracti128	$0x1, %ymm4, %xmm0
	vpmovzxwd	%xmm1, %ymm4
	vextracti128	$0x1, %ymm1, %xmm1
	vpmovzxbw	%xmm0, %ymm0
	vpmovzxwd	%xmm1, %ymm1
	vmovdqa	%ymm1, -88(%rsp)
	vpmovzxwd	%xmm0, %ymm1
	vextracti128	$0x1, %ymm0, %xmm0
	vmovdqa	%ymm1, 1384(%rsp)
	vpmovzxwd	%xmm0, %ymm0
	vpmovzxbw	%xmm5, %ymm1
	vmovdqa	%ymm0, 1352(%rsp)
	vextracti128	$0x1, %ymm5, %xmm0
	vpmovzxwd	%xmm1, %ymm5
	vextracti128	$0x1, %ymm1, %xmm1
	vpmovzxbw	%xmm0, %ymm0
	vpmovzxwd	%xmm1, %ymm2
	vmovdqa	%ymm2, 1320(%rsp)
	vpmovzxwd	%xmm0, %ymm1
	vextracti128	$0x1, %ymm0, %xmm0
	vmovdqa	%ymm1, 1288(%rsp)
	vpmovzxwd	%xmm0, %ymm0
	vpmovzxbw	%xmm12, %ymm1
	vmovdqa	%ymm0, 1256(%rsp)
	vextracti128	$0x1, %ymm12, %xmm0
	vpmovzxwd	%xmm1, %ymm12
	vextracti128	$0x1, %ymm1, %xmm1
	vpmovzxwd	%xmm1, %ymm2
	vpmovzxbw	%xmm0, %ymm0
	vpmovzxbw	%xmm9, %ymm1
	vmovdqa	%ymm2, 1224(%rsp)
	vpmovzxwd	%xmm0, %ymm2
	vextracti128	$0x1, %ymm0, %xmm0
	vmovdqa	%ymm2, 1192(%rsp)
	vpmovzxwd	%xmm0, %ymm0
	vmovdqa	%ymm0, 1160(%rsp)
	vextracti128	$0x1, %ymm9, %xmm0
	vpmovzxwd	%xmm1, %ymm9
	vextracti128	$0x1, %ymm1, %xmm1
	vpmovzxbw	%xmm0, %ymm0
	vpmovzxwd	%xmm1, %ymm1
	vmovdqa	%ymm1, 1128(%rsp)
	vpmovzxwd	%xmm0, %ymm2
	vextracti128	$0x1, %ymm0, %xmm0
	vmovdqa	%ymm2, 1096(%rsp)
	vpmovzxwd	%xmm0, %ymm0
	vmovdqa	1032(%rsp), %ymm2
	vmovdqa	%ymm0, 1064(%rsp)
	vpmovzxbw	%xmm2, %ymm1
	vextracti128	$0x1, %ymm2, %xmm0
	vpmovzxbw	%xmm0, %ymm0
	vpmovzxwd	%xmm1, %ymm2
	vextracti128	$0x1, %ymm1, %xmm1
	vmovdqa	%ymm2, 1032(%rsp)
	vpmovzxwd	%xmm1, %ymm2
	vpmovzxwd	%xmm0, %ymm1
	vextracti128	$0x1, %ymm0, %xmm0
	vmovdqa	%ymm1, 968(%rsp)
	vpmovzxwd	%xmm0, %ymm0
	vpmovzxbw	%xmm7, %ymm1
	vmovdqa	%ymm0, 936(%rsp)
	vextracti128	$0x1, %ymm7, %xmm0
	vpmovzxwd	%xmm1, %ymm7
	vextracti128	$0x1, %ymm1, %xmm1
	vmovdqa	%ymm2, 1000(%rsp)
	vpmovzxbw	%xmm0, %ymm0
	vpmovzxwd	%xmm1, %ymm2
	vmovdqa	%ymm2, 904(%rsp)
	vpmovzxwd	%xmm0, %ymm1
	vextracti128	$0x1, %ymm0, %xmm0
	vmovdqa	%ymm1, 872(%rsp)
	vpmovzxwd	%xmm0, %ymm0
	vpmovzxbw	%xmm13, %ymm1
	vmovdqa	%ymm0, 840(%rsp)
	vextracti128	$0x1, %ymm13, %xmm0
	vpmovzxwd	%xmm1, %ymm13
	vextracti128	$0x1, %ymm1, %xmm1
	vpmovzxwd	%xmm1, %ymm2
	vpmovzxbw	%xmm0, %ymm0
	vmovdqa	%ymm2, 808(%rsp)
	vpmovzxwd	%xmm0, %ymm2
	vextracti128	$0x1, %ymm0, %xmm0
	vmovdqa	%ymm2, 776(%rsp)
	vpmovzxwd	%xmm0, %ymm0
	vmovdqa	712(%rsp), %ymm2
	vmovdqa	%ymm0, 744(%rsp)
	vpmovzxbw	%xmm2, %ymm1
	vextracti128	$0x1, %ymm2, %xmm0
	vpmovzxbw	%xmm0, %ymm0
	vpmovzxwd	%xmm1, %ymm2
	vextracti128	$0x1, %ymm1, %xmm1
	vmovdqa	%ymm2, 712(%rsp)
	vpmovzxwd	%xmm0, %ymm2
	vextracti128	$0x1, %ymm0, %xmm0
	vpmovzxwd	%xmm1, %ymm1
	vmovdqa	%ymm2, 648(%rsp)
	vpmovzxwd	%xmm0, %ymm0
	vmovdqa	584(%rsp), %ymm2
	vmovdqa	%ymm1, 680(%rsp)
	vmovdqa	%ymm0, 616(%rsp)
	vpmovzxbw	%xmm2, %ymm1
	vextracti128	$0x1, %ymm2, %xmm0
	vpmovzxbw	%xmm0, %ymm0
	vpmovzxwd	%xmm1, %ymm2
	vextracti128	$0x1, %ymm1, %xmm1
	vmovdqa	%ymm2, 584(%rsp)
	vpmovzxwd	%xmm1, %ymm2
	vpmovzxwd	%xmm0, %ymm1
	vextracti128	$0x1, %ymm0, %xmm0
	vmovdqa	%ymm1, 520(%rsp)
	vpmovzxwd	%xmm0, %ymm0
	vpmovzxbw	%xmm15, %ymm1
	vmovdqa	%ymm2, 552(%rsp)
	vpmovzxwd	%xmm1, %ymm2
	vextracti128	$0x1, %ymm1, %xmm1
	vmovdqa	%ymm0, 488(%rsp)
	vextracti128	$0x1, %ymm15, %xmm0
	vpmovzxwd	%xmm1, %ymm15
	vpmovzxdq	%xmm11, %ymm1
	vmovdqa	%ymm15, 456(%rsp)
	vpmovzxbw	%xmm0, %ymm0
	vpxor	360(%rsp), %ymm1, %ymm1
	vpmovzxwd	%xmm0, %ymm15
	vextracti128	$0x1, %ymm0, %xmm0
	vpmovzxwd	%xmm0, %ymm0
	vmovdqa	%ymm0, 424(%rsp)
	vpsllq	$31, %ymm1, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$3, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpmovzxdq	1576(%rsp), %ymm1
	vpxor	%ymm1, %ymm0, %ymm0
	vpsllq	$31, %ymm0, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$3, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm0
	vpmovzxdq	%xmm8, %ymm1
	vpxor	%ymm1, %ymm0, %ymm0
	vpsllq	$31, %ymm0, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$3, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm0
	vpmovzxdq	%xmm6, %ymm1
	vpxor	%ymm1, %ymm0, %ymm0
	vpsllq	$31, %ymm0, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$3, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm0
	vpmovzxdq	1512(%rsp), %ymm1
	vpxor	%ymm1, %ymm0, %ymm0
	vpsllq	$31, %ymm0, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$3, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm0
	vpmovzxdq	%xmm3, %ymm1
	vpxor	%ymm1, %ymm0, %ymm0
	vpsllq	$31, %ymm0, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$3, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm0
	vpmovzxdq	%xmm4, %ymm1
	vpxor	%ymm1, %ymm0, %ymm0
	vpsllq	$31, %ymm0, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$3, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm0
	vpmovzxdq	%xmm5, %ymm1
	vpxor	%ymm1, %ymm0, %ymm0
	vpsllq	$31, %ymm0, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$3, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm0
	vpmovzxdq	%xmm12, %ymm1
	vpxor	%ymm1, %ymm0, %ymm0
	vpsllq	$31, %ymm0, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$3, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm0
	vpmovzxdq	%xmm9, %ymm1
	vpxor	%ymm1, %ymm0, %ymm0
	vpsllq	$31, %ymm0, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$3, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm0
	vpmovzxdq	1032(%rsp), %ymm1
	vpxor	%ymm1, %ymm0, %ymm0
	vpsllq	$31, %ymm0, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$3, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm0
	vpmovzxdq	%xmm7, %ymm1
	vpxor	%ymm1, %ymm0, %ymm0
	vpsllq	$31, %ymm0, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$3, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm0
	vpmovzxdq	%xmm13, %ymm1
	vpxor	%ymm1, %ymm0, %ymm0
	vpsllq	$31, %ymm0, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$3, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm0
	vpmovzxdq	712(%rsp), %ymm1
	vpxor	%ymm1, %ymm0, %ymm0
	vpsllq	$31, %ymm0, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$3, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm0
	vpmovzxdq	584(%rsp), %ymm1
	vpxor	%ymm1, %ymm0, %ymm0
	vpsllq	$31, %ymm0, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$3, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm0
	vpmovzxdq	%xmm2, %ymm1
	vpxor	%ymm1, %ymm0, %ymm0
	vpsllq	$31, %ymm0, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$3, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm0
	vextracti128	$0x1, %ymm11, %xmm1
	vmovdqa	360(%rsp), %ymm11
	vpmovzxdq	%xmm1, %ymm1
	vmovdqa	%ymm0, -120(%rsp)
	vpxor	%ymm11, %ymm1, %ymm1
	vpsllq	$31, %ymm1, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$3, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vmovdqa	1576(%rsp), %ymm1
	vextracti128	$0x1, %ymm1, %xmm1
	vpmovzxdq	%xmm1, %ymm1
	vpxor	%ymm1, %ymm0, %ymm1
	vpsllq	$31, %ymm1, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$3, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vextracti128	$0x1, %ymm8, %xmm1
	vpmovzxdq	%xmm1, %ymm1
	vpxor	%ymm1, %ymm0, %ymm1
	vpsllq	$31, %ymm1, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$3, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vextracti128	$0x1, %ymm6, %xmm1
	vmovdqa	168(%rsp), %ymm6
	vpmovzxdq	%xmm1, %ymm1
	vpxor	%ymm1, %ymm0, %ymm1
	vpsllq	$31, %ymm1, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$3, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vmovdqa	1512(%rsp), %ymm1
	vextracti128	$0x1, %ymm1, %xmm1
	vpmovzxdq	%xmm1, %ymm1
	vpxor	%ymm1, %ymm0, %ymm1
	vpsllq	$31, %ymm1, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$3, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vextracti128	$0x1, %ymm3, %xmm1
	vpmovzxdq	%xmm1, %ymm1
	vpxor	%ymm1, %ymm0, %ymm1
	vpsllq	$31, %ymm1, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$3, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vextracti128	$0x1, %ymm4, %xmm1
	vmovdqa	136(%rsp), %ymm4
	vpmovzxdq	%xmm1, %ymm1
	vpxor	%ymm1, %ymm0, %ymm1
	vpsllq	$31, %ymm1, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$3, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vextracti128	$0x1, %ymm5, %xmm1
	vpmovzxdq	%xmm1, %ymm1
	vpxor	%ymm1, %ymm0, %ymm1
	vpsllq	$31, %ymm1, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$3, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vextracti128	$0x1, %ymm12, %xmm1
	vpmovzxdq	%xmm1, %ymm1
	vpxor	%ymm1, %ymm0, %ymm1
	vpsllq	$31, %ymm1, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$3, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vextracti128	$0x1, %ymm9, %xmm1
	vmovdqa	1032(%rsp), %ymm9
	vpmovzxdq	%xmm1, %ymm1
	vpxor	%ymm1, %ymm0, %ymm1
	vpsllq	$31, %ymm1, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$3, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vextracti128	$0x1, %ymm9, %xmm1
	vmovdqa	328(%rsp), %ymm9
	vpmovzxdq	%xmm1, %ymm1
	vpxor	%ymm1, %ymm0, %ymm1
	vpsllq	$31, %ymm1, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$3, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vextracti128	$0x1, %ymm7, %xmm1
	vmovdqa	712(%rsp), %ymm7
	vpmovzxdq	%xmm1, %ymm1
	vpxor	%ymm1, %ymm0, %ymm1
	vpsllq	$31, %ymm1, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$3, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vextracti128	$0x1, %ymm13, %xmm1
	vpmovzxdq	%xmm1, %ymm1
	vpxor	%ymm1, %ymm0, %ymm1
	vpsllq	$31, %ymm1, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$3, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vextracti128	$0x1, %ymm7, %xmm1
	vmovdqa	584(%rsp), %ymm7
	vpmovzxdq	%xmm1, %ymm1
	vpxor	%ymm1, %ymm0, %ymm1
	vpsllq	$31, %ymm1, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$3, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vextracti128	$0x1, %ymm7, %xmm1
	vmovdqa	%ymm11, %ymm7
	vpmovzxdq	%xmm1, %ymm1
	vpxor	%ymm1, %ymm0, %ymm1
	vpsllq	$31, %ymm1, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$3, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vextracti128	$0x1, %ymm2, %xmm1
	vpmovzxdq	%xmm1, %ymm1
	vpxor	%ymm1, %ymm0, %ymm1
	vpsllq	$31, %ymm1, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$3, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpmovzxdq	%xmm6, %ymm1
	vpxor	-120(%rsp), %ymm0, %ymm2
	vpxor	%ymm11, %ymm1, %ymm1
	vmovdqa	232(%rsp), %ymm11
	vpsllq	$31, %ymm1, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$3, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpmovzxdq	%xmm11, %ymm1
	vpxor	%ymm1, %ymm0, %ymm0
	vpsllq	$31, %ymm0, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$3, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm0
	vpmovzxdq	%xmm9, %ymm1
	vpxor	%ymm1, %ymm0, %ymm0
	vpsllq	$31, %ymm0, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$3, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm0
	vpmovzxdq	1544(%rsp), %ymm1
	vpxor	%ymm1, %ymm0, %ymm0
	vpsllq	$31, %ymm0, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$3, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm0
	vpmovzxdq	1480(%rsp), %ymm1
	vpxor	%ymm1, %ymm0, %ymm0
	vpsllq	$31, %ymm0, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$3, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm0
	vpmovzxdq	1416(%rsp), %ymm1
	vpxor	%ymm1, %ymm0, %ymm0
	vpsllq	$31, %ymm0, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$3, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm0
	vpmovzxdq	1352(%rsp), %ymm1
	vpxor	%ymm1, %ymm0, %ymm0
	vpsllq	$31, %ymm0, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$3, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm0
	vpmovzxdq	1256(%rsp), %ymm1
	vpxor	%ymm1, %ymm0, %ymm0
	vpsllq	$31, %ymm0, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$3, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm0
	vpmovzxdq	1160(%rsp), %ymm1
	vpxor	%ymm1, %ymm0, %ymm0
	vpsllq	$31, %ymm0, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$3, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm0
	vpmovzxdq	1064(%rsp), %ymm1
	vpxor	%ymm1, %ymm0, %ymm0
	vpsllq	$31, %ymm0, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$3, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm0
	vpmovzxdq	936(%rsp), %ymm1
	vpxor	%ymm1, %ymm0, %ymm0
	vpsllq	$31, %ymm0, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$3, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm0
	vpmovzxdq	840(%rsp), %ymm1
	vpxor	%ymm1, %ymm0, %ymm0
	vpsllq	$31, %ymm0, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$3, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm0
	vpmovzxdq	744(%rsp), %ymm1
	vpxor	%ymm1, %ymm0, %ymm0
	vpsllq	$31, %ymm0, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$3, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm0
	vpmovzxdq	616(%rsp), %ymm1
	vpxor	%ymm1, %ymm0, %ymm0
	vpsllq	$31, %ymm0, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$3, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm0
	vpmovzxdq	488(%rsp), %ymm1
	vpxor	%ymm1, %ymm0, %ymm0
	vpsllq	$31, %ymm0, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$3, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm0
	vpmovzxdq	424(%rsp), %ymm1
	vmovdqa	200(%rsp), %ymm12
	vmovdqa	296(%rsp), %ymm13
	vmovdqa	40(%rsp), %ymm8
	vpxor	%ymm1, %ymm0, %ymm0
	vmovdqa	-24(%rsp), %ymm5
	vpsllq	$31, %ymm0, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$3, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpxor	%ymm1, %ymm2, %ymm3
	vpmovzxdq	%xmm4, %ymm1
	vpxor	%ymm7, %ymm1, %ymm1
	vpsllq	$31, %ymm1, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$3, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpmovzxdq	%xmm12, %ymm1
	vpxor	%ymm1, %ymm0, %ymm0
	vpsllq	$31, %ymm0, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$3, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm0
	vpmovzxdq	%xmm13, %ymm1
	vpxor	%ymm1, %ymm0, %ymm0
	vpsllq	$31, %ymm0, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$3, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm0
	vpmovzxdq	%xmm8, %ymm1
	vpxor	%ymm1, %ymm0, %ymm0
	vpsllq	$31, %ymm0, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$3, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm0
	vpmovzxdq	%xmm5, %ymm1
	vpxor	%ymm1, %ymm0, %ymm0
	vpsllq	$31, %ymm0, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$3, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm0
	vpmovzxdq	1448(%rsp), %ymm1
	vpxor	%ymm1, %ymm0, %ymm0
	vpsllq	$31, %ymm0, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$3, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm0
	vpmovzxdq	1384(%rsp), %ymm1
	vpxor	%ymm1, %ymm0, %ymm0
	vpsllq	$31, %ymm0, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$3, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm0
	vpmovzxdq	1288(%rsp), %ymm1
	vpxor	%ymm1, %ymm0, %ymm0
	vpsllq	$31, %ymm0, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$3, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm0
	vpmovzxdq	1192(%rsp), %ymm1
	vpxor	%ymm1, %ymm0, %ymm0
	vpsllq	$31, %ymm0, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$3, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm0
	vpmovzxdq	1096(%rsp), %ymm1
	vpxor	%ymm1, %ymm0, %ymm0
	vpsllq	$31, %ymm0, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$3, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm0
	vpmovzxdq	968(%rsp), %ymm1
	vpxor	%ymm1, %ymm0, %ymm0
	vpsllq	$31, %ymm0, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$3, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm0
	vpmovzxdq	872(%rsp), %ymm1
	vpxor	%ymm1, %ymm0, %ymm0
	vpsllq	$31, %ymm0, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$3, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm0
	vpmovzxdq	776(%rsp), %ymm1
	vpxor	%ymm1, %ymm0, %ymm0
	vpsllq	$31, %ymm0, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$3, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm0
	vpmovzxdq	648(%rsp), %ymm1
	vpxor	%ymm1, %ymm0, %ymm0
	vpsllq	$31, %ymm0, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$3, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm0
	vpmovzxdq	520(%rsp), %ymm1
	vpxor	%ymm1, %ymm0, %ymm0
	vpsllq	$31, %ymm0, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$3, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm0
	vpmovzxdq	%xmm15, %ymm1
	vpxor	%ymm1, %ymm0, %ymm0
	vpsllq	$31, %ymm0, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$3, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm2
	vextracti128	$0x1, %ymm4, %xmm1
	vmovdqa	1384(%rsp), %ymm4
	vpmovzxdq	%xmm1, %ymm1
	vpxor	%ymm7, %ymm1, %ymm1
	vpsllq	$31, %ymm1, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$3, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vextracti128	$0x1, %ymm12, %xmm1
	vmovdqa	1288(%rsp), %ymm12
	vpmovzxdq	%xmm1, %ymm1
	vpxor	%ymm1, %ymm0, %ymm1
	vpsllq	$31, %ymm1, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$3, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vextracti128	$0x1, %ymm13, %xmm1
	vmovdqa	-56(%rsp), %ymm13
	vpmovzxdq	%xmm1, %ymm1
	vpxor	%ymm1, %ymm0, %ymm1
	vpsllq	$31, %ymm1, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$3, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vextracti128	$0x1, %ymm8, %xmm1
	vmovdqa	1448(%rsp), %ymm8
	vpmovzxdq	%xmm1, %ymm1
	vpxor	%ymm1, %ymm0, %ymm1
	vpsllq	$31, %ymm1, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$3, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vextracti128	$0x1, %ymm5, %xmm1
	vmovdqa	104(%rsp), %ymm5
	vpmovzxdq	%xmm1, %ymm1
	vpxor	%ymm1, %ymm0, %ymm1
	vpsllq	$31, %ymm1, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$3, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vextracti128	$0x1, %ymm8, %xmm1
	vmovdqa	968(%rsp), %ymm8
	vpmovzxdq	%xmm1, %ymm1
	vpxor	%ymm1, %ymm0, %ymm1
	vpsllq	$31, %ymm1, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$3, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vextracti128	$0x1, %ymm4, %xmm1
	vmovdqa	264(%rsp), %ymm4
	vpmovzxdq	%xmm1, %ymm1
	vpxor	%ymm1, %ymm0, %ymm1
	vpsllq	$31, %ymm1, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$3, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vextracti128	$0x1, %ymm12, %xmm1
	vmovdqa	1192(%rsp), %ymm12
	vpmovzxdq	%xmm1, %ymm1
	vpxor	%ymm1, %ymm0, %ymm1
	vpsllq	$31, %ymm1, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$3, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vextracti128	$0x1, %ymm12, %xmm1
	vmovdqa	1096(%rsp), %ymm12
	vpmovzxdq	%xmm1, %ymm1
	vpxor	%ymm1, %ymm0, %ymm1
	vpsllq	$31, %ymm1, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$3, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vextracti128	$0x1, %ymm12, %xmm1
	vmovdqa	8(%rsp), %ymm12
	vpmovzxdq	%xmm1, %ymm1
	vpxor	%ymm1, %ymm0, %ymm1
	vpsllq	$31, %ymm1, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$3, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vextracti128	$0x1, %ymm8, %xmm1
	vmovdqa	872(%rsp), %ymm8
	vpmovzxdq	%xmm1, %ymm1
	vpxor	%ymm1, %ymm0, %ymm1
	vpsllq	$31, %ymm1, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$3, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vextracti128	$0x1, %ymm8, %xmm1
	vmovdqa	776(%rsp), %ymm8
	vpmovzxdq	%xmm1, %ymm1
	vpxor	%ymm1, %ymm0, %ymm1
	vpsllq	$31, %ymm1, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$3, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vextracti128	$0x1, %ymm8, %xmm1
	vmovdqa	648(%rsp), %ymm8
	vpmovzxdq	%xmm1, %ymm1
	vpxor	%ymm1, %ymm0, %ymm1
	vpsllq	$31, %ymm1, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$3, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vextracti128	$0x1, %ymm8, %xmm1
	vmovdqa	520(%rsp), %ymm8
	vpmovzxdq	%xmm1, %ymm1
	vpxor	%ymm1, %ymm0, %ymm1
	vpsllq	$31, %ymm1, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$3, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vextracti128	$0x1, %ymm8, %xmm1
	vmovdqa	-88(%rsp), %ymm8
	vpmovzxdq	%xmm1, %ymm1
	vpxor	%ymm1, %ymm0, %ymm1
	vpsllq	$31, %ymm1, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$3, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vextracti128	$0x1, %ymm15, %xmm1
	vmovdqa	72(%rsp), %ymm15
	vpmovzxdq	%xmm1, %ymm1
	vpxor	%ymm1, %ymm0, %ymm1
	vpsllq	$31, %ymm1, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$3, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpmovzxdq	%xmm5, %ymm1
	vpxor	%ymm7, %ymm1, %ymm1
	vpxor	%ymm0, %ymm2, %ymm0
	vpxor	%ymm0, %ymm3, %ymm3
	vpsllq	$31, %ymm1, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$3, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpmovzxdq	%xmm10, %ymm1
	vpxor	%ymm1, %ymm0, %ymm0
	vpsllq	$31, %ymm0, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$3, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm0
	vpmovzxdq	%xmm4, %ymm1
	vpxor	%ymm1, %ymm0, %ymm0
	vpsllq	$31, %ymm0, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$3, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm0
	vpmovzxdq	%xmm15, %ymm1
	vpxor	%ymm1, %ymm0, %ymm0
	vpsllq	$31, %ymm0, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$3, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm0
	vpmovzxdq	%xmm12, %ymm1
	vpxor	%ymm1, %ymm0, %ymm0
	vpsllq	$31, %ymm0, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$3, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm0
	vpmovzxdq	%xmm13, %ymm1
	vpxor	%ymm1, %ymm0, %ymm0
	vpsllq	$31, %ymm0, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$3, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm0
	vpmovzxdq	%xmm8, %ymm1
	vpxor	%ymm1, %ymm0, %ymm0
	vpsllq	$31, %ymm0, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$3, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm0
	vpmovzxdq	1320(%rsp), %ymm1
	vpxor	%ymm1, %ymm0, %ymm0
	vpsllq	$31, %ymm0, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$3, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm0
	vpmovzxdq	1224(%rsp), %ymm1
	vpxor	%ymm1, %ymm0, %ymm0
	vpsllq	$31, %ymm0, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$3, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm0
	vpmovzxdq	1128(%rsp), %ymm1
	vpxor	%ymm1, %ymm0, %ymm0
	vpsllq	$31, %ymm0, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$3, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm0
	vpmovzxdq	1000(%rsp), %ymm1
	vpxor	%ymm1, %ymm0, %ymm0
	vpsllq	$31, %ymm0, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$3, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm0
	vpmovzxdq	904(%rsp), %ymm1
	vpxor	%ymm1, %ymm0, %ymm0
	vpsllq	$31, %ymm0, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$3, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm0
	vpmovzxdq	808(%rsp), %ymm1
	vpxor	%ymm1, %ymm0, %ymm0
	vpsllq	$31, %ymm0, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$3, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm0
	vpmovzxdq	680(%rsp), %ymm1
	vpxor	%ymm1, %ymm0, %ymm0
	vpsllq	$31, %ymm0, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$3, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm0
	vpmovzxdq	552(%rsp), %ymm1
	vpxor	%ymm1, %ymm0, %ymm0
	vpsllq	$31, %ymm0, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$3, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm0
	vpmovzxdq	456(%rsp), %ymm1
	vpxor	%ymm1, %ymm0, %ymm0
	vpsllq	$31, %ymm0, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$3, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm2
	vextracti128	$0x1, %ymm5, %xmm1
	vmovdqa	1544(%rsp), %ymm5
	vpmovzxdq	%xmm1, %ymm1
	vpxor	%ymm7, %ymm1, %ymm1
	vpsllq	$31, %ymm1, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$3, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vextracti128	$0x1, %ymm10, %xmm1
	vpmovzxdq	%xmm1, %ymm1
	vpxor	%ymm1, %ymm0, %ymm1
	vpsllq	$31, %ymm1, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$3, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vextracti128	$0x1, %ymm4, %xmm1
	vmovdqa	1352(%rsp), %ymm4
	vpmovzxdq	%xmm1, %ymm1
	vpxor	%ymm1, %ymm0, %ymm1
	vpsllq	$31, %ymm1, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$3, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vextracti128	$0x1, %ymm15, %xmm1
	vmovdqa	456(%rsp), %ymm15
	vpmovzxdq	%xmm1, %ymm1
	vpxor	%ymm1, %ymm0, %ymm1
	vpsllq	$31, %ymm1, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$3, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vextracti128	$0x1, %ymm12, %xmm1
	vmovdqa	1064(%rsp), %ymm12
	vpmovzxdq	%xmm1, %ymm1
	vpxor	%ymm1, %ymm0, %ymm1
	vpsllq	$31, %ymm1, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$3, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vextracti128	$0x1, %ymm13, %xmm1
	vmovdqa	1256(%rsp), %ymm13
	vpmovzxdq	%xmm1, %ymm1
	vpxor	%ymm1, %ymm0, %ymm1
	vpsllq	$31, %ymm1, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$3, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vextracti128	$0x1, %ymm8, %xmm1
	vpmovzxdq	%xmm1, %ymm1
	vpxor	%ymm1, %ymm0, %ymm1
	vpsllq	$31, %ymm1, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$3, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vmovdqa	1320(%rsp), %ymm1
	vextracti128	$0x1, %ymm1, %xmm1
	vpmovzxdq	%xmm1, %ymm1
	vpxor	%ymm1, %ymm0, %ymm1
	vpsllq	$31, %ymm1, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$3, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vmovdqa	1224(%rsp), %ymm1
	vextracti128	$0x1, %ymm1, %xmm1
	vpmovzxdq	%xmm1, %ymm1
	vpxor	%ymm1, %ymm0, %ymm1
	vpsllq	$31, %ymm1, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$3, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vmovdqa	1128(%rsp), %ymm1
	vextracti128	$0x1, %ymm1, %xmm1
	vpmovzxdq	%xmm1, %ymm1
	vpxor	%ymm1, %ymm0, %ymm1
	vpsllq	$31, %ymm1, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$3, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vmovdqa	1000(%rsp), %ymm1
	vextracti128	$0x1, %ymm1, %xmm1
	vpmovzxdq	%xmm1, %ymm1
	vpxor	%ymm1, %ymm0, %ymm1
	vpsllq	$31, %ymm1, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$3, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vmovdqa	904(%rsp), %ymm1
	vextracti128	$0x1, %ymm1, %xmm1
	vpmovzxdq	%xmm1, %ymm1
	vpxor	%ymm1, %ymm0, %ymm1
	vpsllq	$31, %ymm1, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$3, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vmovdqa	808(%rsp), %ymm1
	vextracti128	$0x1, %ymm1, %xmm1
	vpmovzxdq	%xmm1, %ymm1
	vpxor	%ymm1, %ymm0, %ymm1
	vpsllq	$31, %ymm1, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$3, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vmovdqa	680(%rsp), %ymm1
	vextracti128	$0x1, %ymm1, %xmm1
	vpmovzxdq	%xmm1, %ymm1
	vpxor	%ymm1, %ymm0, %ymm1
	vpsllq	$31, %ymm1, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$3, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vmovdqa	552(%rsp), %ymm1
	vextracti128	$0x1, %ymm1, %xmm1
	vpmovzxdq	%xmm1, %ymm1
	vpxor	%ymm1, %ymm0, %ymm1
	vpsllq	$31, %ymm1, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$3, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vextracti128	$0x1, %ymm15, %xmm1
	vpmovzxdq	%xmm1, %ymm1
	vpxor	%ymm1, %ymm0, %ymm1
	vpsllq	$31, %ymm1, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$3, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vextracti128	$0x1, %ymm6, %xmm1
	vpmovzxdq	%xmm1, %ymm1
	vpxor	%ymm0, %ymm2, %ymm2
	vpxor	%ymm7, %ymm1, %ymm1
	vpsllq	$31, %ymm1, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$3, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vextracti128	$0x1, %ymm11, %xmm1
	vpmovzxdq	%xmm1, %ymm1
	vpxor	%ymm1, %ymm0, %ymm1
	vpsllq	$31, %ymm1, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$3, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vextracti128	$0x1, %ymm9, %xmm1
	vpmovzxdq	%xmm1, %ymm1
	vpxor	%ymm1, %ymm0, %ymm1
	vpsllq	$31, %ymm1, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$3, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vextracti128	$0x1, %ymm5, %xmm1
	vmovdqa	1480(%rsp), %ymm5
	vpmovzxdq	%xmm1, %ymm1
	vpxor	%ymm1, %ymm0, %ymm1
	vpsllq	$31, %ymm1, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$3, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vextracti128	$0x1, %ymm5, %xmm1
	vmovdqa	1416(%rsp), %ymm5
	vpmovzxdq	%xmm1, %ymm1
	vpxor	%ymm1, %ymm0, %ymm1
	vpsllq	$31, %ymm1, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$3, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vextracti128	$0x1, %ymm5, %xmm1
	vmovdqa	1160(%rsp), %ymm5
	vpmovzxdq	%xmm1, %ymm1
	vpxor	%ymm1, %ymm0, %ymm1
	vpsllq	$31, %ymm1, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$3, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vextracti128	$0x1, %ymm4, %xmm1
	vmovdqa	840(%rsp), %ymm4
	vpmovzxdq	%xmm1, %ymm1
	vpxor	%ymm1, %ymm0, %ymm0
	vpsllq	$31, %ymm0, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$3, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm0
	vextracti128	$0x1, %ymm13, %xmm1
	vmovdqa	744(%rsp), %ymm13
	vpmovzxdq	%xmm1, %ymm1
	vpxor	%ymm1, %ymm0, %ymm0
	vpsllq	$31, %ymm0, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$3, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm0
	vextracti128	$0x1, %ymm5, %xmm1
	vmovdqa	936(%rsp), %ymm5
	vpmovzxdq	%xmm1, %ymm1
	vpxor	%ymm1, %ymm0, %ymm0
	vpsllq	$31, %ymm0, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$3, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm0
	vextracti128	$0x1, %ymm12, %xmm1
	vmovdqa	488(%rsp), %ymm12
	vpmovzxdq	%xmm1, %ymm1
	vpxor	%ymm1, %ymm0, %ymm0
	vpsllq	$31, %ymm0, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$3, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm0
	vextracti128	$0x1, %ymm5, %xmm1
	vmovdqa	616(%rsp), %ymm5
	vpmovzxdq	%xmm1, %ymm1
	vpxor	%ymm1, %ymm0, %ymm0
	vpsllq	$31, %ymm0, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$3, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm0
	vextracti128	$0x1, %ymm4, %xmm1
	vpmovzxdq	%xmm1, %ymm1
	vpxor	%ymm1, %ymm0, %ymm0
	vpsllq	$31, %ymm0, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$3, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vextracti128	$0x1, %ymm13, %xmm0
	vpmovzxdq	%xmm0, %ymm0
	vpxor	%ymm0, %ymm1, %ymm1
	vpsllq	$31, %ymm1, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$3, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vextracti128	$0x1, %ymm5, %xmm1
	vmovdqa	424(%rsp), %ymm5
	vpmovzxdq	%xmm1, %ymm1
	vpxor	%ymm1, %ymm0, %ymm0
	vpsllq	$31, %ymm0, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$3, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vextracti128	$0x1, %ymm12, %xmm0
	vpmovzxdq	%xmm0, %ymm0
	vpxor	%ymm0, %ymm1, %ymm1
	vpsllq	$31, %ymm1, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$3, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm0
	vpsllq	$2, %ymm0, %ymm0
	vpsubq	%ymm1, %ymm0, %ymm0
	vextracti128	$0x1, %ymm5, %xmm1
	vpmovzxdq	%xmm1, %ymm1
	vpxor	%ymm1, %ymm0, %ymm0
	vpsllq	$31, %ymm0, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$3, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpaddq	%ymm0, %ymm1, %ymm1
	vpsllq	$2, %ymm1, %ymm1
	vpsubq	%ymm0, %ymm1, %ymm0
	vpxor	%ymm0, %ymm2, %ymm2
	vpxor	%ymm2, %ymm3, %ymm3
	vpxor	392(%rsp), %ymm3, %ymm5
	vmovdqa	%ymm5, 392(%rsp)
	cmpq	%rax, %rdx
	jne	.L7
	vextracti128	$0x1, %ymm5, %xmm1
	vpxor	%xmm5, %xmm1, %xmm0
	vpsrldq	$8, %xmm0, %xmm1
	vpxor	%xmm1, %xmm0, %xmm0
	vmovq	%xmm0, %rax
	vzeroupper
	leave
	.cfi_def_cfa 7, 8
	ret
	.cfi_endproc
.LFE2:
	.size	bench_run, .-bench_run
	.local	keys
	.comm	keys,4096,32
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
