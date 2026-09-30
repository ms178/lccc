	.file	"conv_u8_3x3.c"
	.text
	.section	.rodata.str1.1,"aMS",@progbits,1
.LC8:
	.string	"%llx\n"
	.section	.text.startup,"ax",@progbits
	.p2align 4
	.globl	main
	.type	main, @function
main:
.LFB12:
	.cfi_startproc
	pushq	%rbp
	.cfi_def_cfa_offset 16
	.cfi_offset 6, -16
	leaq	img(%rip), %rdx
	xorl	%ecx, %ecx
	movl	$3, %eax
	movq	%rsp, %rbp
	.cfi_def_cfa_register 6
	pushq	%r15
	pushq	%r14
	pushq	%r13
	pushq	%r12
	pushq	%rbx
	andq	$-32, %rsp
	subq	$64, %rsp
	.cfi_offset 15, -24
	.cfi_offset 14, -32
	.cfi_offset 13, -40
	.cfi_offset 12, -48
	.cfi_offset 3, -56
	vmovdqa	.LC0(%rip), %ymm3
	vmovdqa	.LC3(%rip), %ymm2
	vmovdqa	.LC1(%rip), %ymm6
	vmovdqa	.LC2(%rip), %ymm5
	vpunpcklbw	%ymm3, %ymm3, %ymm8
	vpunpcklbw	%ymm2, %ymm2, %ymm7
	vpunpckhbw	%ymm3, %ymm3, %ymm3
	vpunpckhbw	%ymm2, %ymm2, %ymm2
.L2:
	vmovd	%eax, %xmm0
	vmovd	%ecx, %xmm4
	addl	$1, %eax
	addl	$5, %ecx
	vpbroadcastb	%xmm0, %ymm0
	vpbroadcastb	%xmm4, %ymm4
	addq	$64, %rdx
	vpunpcklbw	%ymm0, %ymm0, %ymm9
	vpunpckhbw	%ymm0, %ymm0, %ymm0
	vpmullw	%ymm0, %ymm3, %ymm1
	vpmullw	%ymm9, %ymm8, %ymm10
	vpmullw	%ymm0, %ymm2, %ymm0
	vpshufb	%ymm6, %ymm10, %ymm10
	vpshufb	%ymm5, %ymm1, %ymm1
	vpblendd	$51, %ymm10, %ymm1, %ymm1
	vpshufb	%ymm5, %ymm0, %ymm0
	vpaddb	%ymm1, %ymm4, %ymm1
	vmovdqa	%ymm1, -64(%rdx)
	vpmullw	%ymm9, %ymm7, %ymm1
	vpshufb	%ymm6, %ymm1, %ymm1
	vpblendd	$51, %ymm1, %ymm0, %ymm0
	vpaddb	%ymm0, %ymm4, %ymm4
	vmovdqa	%ymm4, -32(%rdx)
	cmpb	$67, %al
	jne	.L2
	movl	$16711935, %eax
	leaq	113+out(%rip), %r10
	vmovdqa	.LC6(%rip), %xmm0
	movq	$0, 48(%rsp)
	vmovd	%eax, %xmm2
	vpxor	%xmm9, %xmm9, %xmm9
	vpxor	%xmm7, %xmm7, %xmm7
	movq	%r10, %r8
	vpmovzxbw	%xmm0, %ymm8
	vpmovzxbw	%xmm0, %xmm10
	vpbroadcastd	%xmm2, %ymm5
	movq	$-64, 40(%rsp)
	vpsrldq	$8, %xmm0, %xmm0
	leaq	128+img(%rip), %rbx
	vpbroadcastd	%xmm2, %xmm2
	movl	$128, %r11d
	movl	$3, %r14d
	vpmovzxbw	%xmm0, %xmm4
.L5:
	vmovdqu	-127(%rbx), %ymm1
	vmovdqa	(%rbx), %ymm3
	leaq	-14(%rbx), %rsi
	movq	%r8, %rdi
	vmovdqa	-128(%rbx), %ymm13
	vmovdqu	-126(%rbx), %ymm12
	vpmovzxbw	%xmm1, %ymm0
	vmovdqu	1(%rbx), %ymm6
	vextracti128	$0x1, %ymm1, %xmm1
	vmovdqu	2(%rbx), %ymm11
	vpsubw	%ymm0, %ymm9, %ymm0
	vpmovzxbw	%xmm13, %ymm14
	vpmovzxbw	%xmm3, %ymm15
	vpsllw	$1, %ymm0, %ymm0
	vpmovzxbw	%xmm1, %ymm1
	vextracti128	$0x1, %ymm3, %xmm3
	vpsubw	%ymm14, %ymm0, %ymm0
	vpmovzxbw	%xmm12, %ymm14
	vpsubw	%ymm1, %ymm9, %ymm1
	vpsubw	%ymm14, %ymm0, %ymm0
	vpmovzxbw	%xmm6, %ymm14
	vextracti128	$0x1, %ymm6, %xmm6
	vpmovzxbw	%xmm6, %ymm6
	vpmullw	%ymm8, %ymm14, %ymm14
	vextracti128	$0x1, %ymm13, %xmm13
	vpmullw	%ymm8, %ymm6, %ymm6
	vpmovzxbw	%xmm3, %ymm3
	vpmovzxbw	%xmm13, %ymm13
	vpsllw	$1, %ymm1, %ymm1
	vextracti128	$0x1, %ymm12, %xmm12
	vpsubw	%ymm13, %ymm1, %ymm1
	vpmovzxbw	%xmm12, %ymm12
	vpsubw	%ymm12, %ymm1, %ymm1
	vpaddw	%ymm14, %ymm15, %ymm14
	vpmovzxbw	%xmm11, %ymm15
	vpaddw	%ymm6, %ymm3, %ymm3
	vextracti128	$0x1, %ymm11, %xmm6
	vpaddw	%ymm15, %ymm14, %ymm14
	vpmovzxbw	%xmm6, %ymm6
	vpaddw	%ymm14, %ymm0, %ymm0
	vpaddw	%ymm6, %ymm3, %ymm3
	vpabsw	%ymm0, %ymm0
	vpaddw	%ymm3, %ymm1, %ymm1
	vpminuw	%ymm5, %ymm0, %ymm0
	vpabsw	%ymm1, %ymm1
	vpand	%ymm5, %ymm0, %ymm0
	vpminuw	%ymm5, %ymm1, %ymm1
	vpand	%ymm5, %ymm1, %ymm1
	vpackuswb	%ymm1, %ymm0, %ymm0
	vpermq	$216, %ymm0, %ymm0
	vmovdqu	%ymm0, -48(%r8)
	vmovdqu	-63(%rbx), %ymm1
	vmovdqa	-64(%rbx), %ymm13
	vmovdqu	-62(%rbx), %ymm12
	vmovdqu	65(%rbx), %ymm6
	vpmovzxbw	%xmm1, %ymm0
	vpmovzxbw	%xmm13, %ymm14
	vmovdqa	64(%rbx), %ymm3
	vextracti128	$0x1, %ymm1, %xmm1
	vpsubw	%ymm0, %ymm9, %ymm0
	vmovdqu	66(%rbx), %ymm11
	vpmovzxbw	%xmm1, %ymm1
	vextracti128	$0x1, %ymm13, %xmm13
	vpsllw	$1, %ymm0, %ymm0
	vpmovzxbw	%xmm3, %ymm15
	vextracti128	$0x1, %ymm3, %xmm3
	vpsubw	%ymm14, %ymm0, %ymm0
	vpmovzxbw	%xmm12, %ymm14
	vpsubw	%ymm1, %ymm9, %ymm1
	vpsubw	%ymm14, %ymm0, %ymm0
	vpmovzxbw	%xmm6, %ymm14
	vextracti128	$0x1, %ymm6, %xmm6
	vpmovzxbw	%xmm6, %ymm6
	vpmullw	%ymm8, %ymm14, %ymm14
	vpmovzxbw	%xmm3, %ymm3
	vpmullw	%ymm8, %ymm6, %ymm6
	vpsllw	$1, %ymm1, %ymm1
	vpmovzxbw	%xmm13, %ymm13
	vextracti128	$0x1, %ymm12, %xmm12
	vpsubw	%ymm13, %ymm1, %ymm1
	vpmovzxbw	%xmm12, %ymm12
	vpsubw	%ymm12, %ymm1, %ymm1
	vpaddw	%ymm14, %ymm15, %ymm14
	vpmovzxbw	%xmm11, %ymm15
	vpaddw	%ymm6, %ymm3, %ymm3
	vextracti128	$0x1, %ymm11, %xmm6
	vpaddw	%ymm15, %ymm14, %ymm14
	vpmovzxbw	%xmm6, %ymm6
	vpaddw	%ymm14, %ymm0, %ymm0
	vpaddw	%ymm6, %ymm3, %ymm3
	vpabsw	%ymm0, %ymm0
	vpaddw	%ymm3, %ymm1, %ymm1
	vpminuw	%ymm5, %ymm0, %ymm0
	vpabsw	%ymm1, %ymm1
	vpand	%ymm5, %ymm0, %ymm0
	vpminuw	%ymm5, %ymm1, %ymm1
	vpand	%ymm5, %ymm1, %ymm1
	vpackuswb	%ymm1, %ymm0, %ymm0
	vpermq	$216, %ymm0, %ymm0
	vmovdqu	%ymm0, 16(%r8)
	vmovdqu	33(%rbx), %xmm6
	vmovdqu	-95(%rbx), %xmm0
	vmovdqa	-96(%rbx), %xmm13
	vmovdqu	-94(%rbx), %xmm12
	vpmovzxbw	%xmm6, %xmm15
	vpsrldq	$8, %xmm6, %xmm6
	vpmovzxbw	%xmm0, %xmm1
	vmovdqa	32(%rbx), %xmm3
	vpmovzxbw	%xmm6, %xmm6
	vpsubw	%xmm1, %xmm7, %xmm1
	vpmovzxbw	%xmm13, %xmm14
	vmovdqu	34(%rbx), %xmm11
	vpmullw	%xmm4, %xmm6, %xmm6
	vpmullw	%xmm10, %xmm15, %xmm15
	vpsllw	$1, %xmm1, %xmm1
	vpsrldq	$8, %xmm0, %xmm0
	vpsubw	%xmm14, %xmm1, %xmm1
	vpmovzxbw	%xmm12, %xmm14
	vpmovzxbw	%xmm0, %xmm0
	vpsubw	%xmm14, %xmm1, %xmm1
	vpmovzxbw	%xmm3, %xmm14
	vpsubw	%xmm0, %xmm7, %xmm0
	vpsrldq	$8, %xmm3, %xmm3
	vpsrldq	$8, %xmm13, %xmm13
	vpmovzxbw	%xmm3, %xmm3
	vpmovzxbw	%xmm13, %xmm13
	vpaddw	%xmm15, %xmm14, %xmm14
	vpaddw	%xmm6, %xmm3, %xmm3
	vpsllw	$1, %xmm0, %xmm0
	vpmovzxbw	%xmm11, %xmm15
	vpsrldq	$8, %xmm11, %xmm6
	vpsrldq	$8, %xmm12, %xmm12
	vpsubw	%xmm13, %xmm0, %xmm0
	vpmovzxbw	%xmm12, %xmm12
	vpmovzxbw	%xmm6, %xmm6
	vpaddw	%xmm15, %xmm14, %xmm14
	vpaddw	%xmm6, %xmm3, %xmm3
	vpsubw	%xmm12, %xmm0, %xmm0
	vpaddw	%xmm14, %xmm1, %xmm1
	vpaddw	%xmm3, %xmm0, %xmm0
	vpabsw	%xmm1, %xmm1
	vpabsw	%xmm0, %xmm0
	vpminuw	%xmm2, %xmm1, %xmm1
	vpminuw	%xmm2, %xmm0, %xmm0
	vpand	%xmm2, %xmm1, %xmm1
	vpand	%xmm2, %xmm0, %xmm0
	vpackuswb	%xmm0, %xmm1, %xmm0
	vmovdqu	%xmm0, -16(%r8)
	vmovdqu	97(%rbx), %xmm6
	vmovdqu	-31(%rbx), %xmm0
	vmovdqa	-32(%rbx), %xmm13
	vmovdqu	-30(%rbx), %xmm12
	vpmovzxbw	%xmm6, %xmm15
	vpsrldq	$8, %xmm6, %xmm6
	vpmovzxbw	%xmm0, %xmm1
	vmovdqa	96(%rbx), %xmm3
	vpmovzxbw	%xmm6, %xmm6
	vpsubw	%xmm1, %xmm7, %xmm1
	vpmovzxbw	%xmm13, %xmm14
	vmovdqu	98(%rbx), %xmm11
	vpmullw	%xmm4, %xmm6, %xmm6
	vpmullw	%xmm10, %xmm15, %xmm15
	vpsllw	$1, %xmm1, %xmm1
	vpsrldq	$8, %xmm0, %xmm0
	vpsubw	%xmm14, %xmm1, %xmm1
	vpmovzxbw	%xmm12, %xmm14
	vpmovzxbw	%xmm0, %xmm0
	vpsubw	%xmm14, %xmm1, %xmm1
	vpmovzxbw	%xmm3, %xmm14
	vpsubw	%xmm0, %xmm7, %xmm0
	vpsrldq	$8, %xmm3, %xmm3
	vpsrldq	$8, %xmm13, %xmm13
	vpmovzxbw	%xmm3, %xmm3
	vpsllw	$1, %xmm0, %xmm0
	vpmovzxbw	%xmm13, %xmm13
	vpaddw	%xmm6, %xmm3, %xmm3
	vpsrldq	$8, %xmm12, %xmm12
	vpaddw	%xmm15, %xmm14, %xmm14
	vpsrldq	$8, %xmm11, %xmm6
	vpmovzxbw	%xmm11, %xmm15
	vpsubw	%xmm13, %xmm0, %xmm0
	vpmovzxbw	%xmm12, %xmm12
	vpmovzxbw	%xmm6, %xmm6
	vpaddw	%xmm15, %xmm14, %xmm14
	vpsubw	%xmm12, %xmm0, %xmm0
	vpaddw	%xmm6, %xmm3, %xmm3
	vpaddw	%xmm14, %xmm1, %xmm1
	vpaddw	%xmm3, %xmm0, %xmm0
	vpabsw	%xmm1, %xmm1
	vpabsw	%xmm0, %xmm0
	vpminuw	%xmm2, %xmm1, %xmm1
	vpminuw	%xmm2, %xmm0, %xmm0
	vpand	%xmm2, %xmm1, %xmm1
	vpand	%xmm2, %xmm0, %xmm0
	vpackuswb	%xmm0, %xmm1, %xmm0
	vmovdqu	%xmm0, 48(%r8)
	movzbl	-79(%rbx), %r13d
	movzbl	-80(%rbx), %r12d
	movzbl	48(%rbx), %edx
	movzbl	49(%rbx), %eax
	movzbl	-15(%rbx), %r10d
	movzbl	113(%rbx), %r15d
	movq	%r8, 24(%rsp)
	movzbl	112(%rbx), %ecx
	movzbl	-16(%rbx), %r9d
	movl	%r14d, 56(%rsp)
	movl	%r15d, 60(%rsp)
	movq	%rbx, 32(%rsp)
	movl	%ecx, %ebx
.L4:
	movq	40(%rsp), %r15
	movl	%r13d, %ecx
	movq	48(%rsp), %r14
	negl	%ecx
	leaq	(%r15,%rsi), %r8
	addl	%ecx, %ecx
	subl	%r12d, %ecx
	movzbl	(%r8,%r14), %r12d
	subl	%r12d, %ecx
	addl	%edx, %ecx
	movzbl	(%r8,%r11), %edx
	leal	(%rcx,%rax,2), %ecx
	addl	%edx, %ecx
	movl	%ecx, %r14d
	negl	%r14d
	cmovns	%r14d, %ecx
	movl	$255, %r14d
	cmpl	%r14d, %ecx
	cmovg	%r14d, %ecx
	leaq	(%r15,%rdi), %r14
	movb	%cl, (%rdi)
	movl	%r10d, %ecx
	negl	%ecx
	addl	%ecx, %ecx
	subl	%r9d, %ecx
	movzbl	(%rsi), %r9d
	subl	%r9d, %ecx
	addl	%ebx, %ecx
	movl	60(%rsp), %ebx
	leal	(%rcx,%rbx,2), %ecx
	movzbl	128(%rsi), %ebx
	addl	%ebx, %ecx
	movl	%ecx, %r15d
	negl	%r15d
	cmovns	%r15d, %ecx
	movl	$255, %r15d
	cmpl	%r15d, %ecx
	cmovg	%r15d, %ecx
	movq	48(%rsp), %r15
	movb	%cl, (%r14,%r11)
	movl	%r12d, %ecx
	negl	%ecx
	addl	%ecx, %ecx
	subl	%r13d, %ecx
	movzbl	1(%r8,%r15), %r13d
	movl	60(%rsp), %r15d
	subl	%r13d, %ecx
	addl	%eax, %ecx
	movzbl	1(%r8,%r11), %eax
	leal	(%rcx,%rdx,2), %ecx
	addl	%eax, %ecx
	movl	%ecx, %r8d
	negl	%r8d
	cmovns	%r8d, %ecx
	movl	$255, %r8d
	cmpl	%r8d, %ecx
	cmovg	%r8d, %ecx
	movb	%cl, 1(%rdi)
	movl	%r9d, %ecx
	negl	%ecx
	addl	%ecx, %ecx
	subl	%r10d, %ecx
	movzbl	1(%rsi), %r10d
	subl	%r10d, %ecx
	addl	%r15d, %ecx
	movzbl	129(%rsi), %r15d
	leal	(%rcx,%rbx,2), %ecx
	addl	%r15d, %ecx
	movl	%r15d, 60(%rsp)
	movl	%ecx, %r8d
	negl	%r8d
	cmovns	%r8d, %ecx
	movl	$255, %r8d
	cmpl	%r8d, %ecx
	cmovg	%r8d, %ecx
	addq	$2, %rsi
	addq	$2, %rdi
	movb	%cl, 1(%r14,%r11)
	cmpq	%rsi, 32(%rsp)
	jne	.L4
	movl	56(%rsp), %r14d
	movq	32(%rsp), %rbx
	subq	$-128, %r11
	movq	24(%rsp), %r8
	addq	$-128, 40(%rsp)
	addl	$2, %r14d
	subq	$-128, 48(%rsp)
	subq	$-128, %rbx
	subq	$-128, %r8
	cmpl	$65, %r14d
	jne	.L5
	leaq	64+out(%rip), %rcx
	xorl	%esi, %esi
	leaq	4096(%rcx), %rdi
.L6:
	leaq	-64(%rcx), %rax
	.p2align 5
	.p2align 4
	.p2align 3
.L7:
	movq	%rsi, %rdx
	addq	$1, %rax
	salq	$5, %rdx
	subq	%rsi, %rdx
	movzbl	-1(%rax), %esi
	addq	%rdx, %rsi
	cmpq	%rax, %rcx
	jne	.L7
	addq	$64, %rcx
	cmpq	%rcx, %rdi
	jne	.L6
	xorl	%eax, %eax
	leaq	.LC8(%rip), %rdi
	vzeroupper
	call	printf@PLT
	leaq	-40(%rbp), %rsp
	xorl	%eax, %eax
	popq	%rbx
	popq	%r12
	popq	%r13
	popq	%r14
	popq	%r15
	popq	%rbp
	.cfi_def_cfa 7, 8
	ret
	.cfi_endproc
.LFE12:
	.size	main, .-main
	.local	out
	.comm	out,4096,32
	.local	img
	.comm	img,4096,32
	.section	.rodata.cst32,"aM",@progbits,32
	.align 32
.LC0:
	.byte	0
	.byte	1
	.byte	2
	.byte	3
	.byte	4
	.byte	5
	.byte	6
	.byte	7
	.byte	8
	.byte	9
	.byte	10
	.byte	11
	.byte	12
	.byte	13
	.byte	14
	.byte	15
	.byte	16
	.byte	17
	.byte	18
	.byte	19
	.byte	20
	.byte	21
	.byte	22
	.byte	23
	.byte	24
	.byte	25
	.byte	26
	.byte	27
	.byte	28
	.byte	29
	.byte	30
	.byte	31
	.align 32
.LC1:
	.byte	0
	.byte	2
	.byte	4
	.byte	6
	.byte	8
	.byte	10
	.byte	12
	.byte	14
	.byte	8
	.byte	9
	.byte	10
	.byte	11
	.byte	12
	.byte	13
	.byte	14
	.byte	15
	.byte	0
	.byte	2
	.byte	4
	.byte	6
	.byte	8
	.byte	10
	.byte	12
	.byte	14
	.byte	8
	.byte	9
	.byte	10
	.byte	11
	.byte	12
	.byte	13
	.byte	14
	.byte	15
	.align 32
.LC2:
	.byte	0
	.byte	1
	.byte	2
	.byte	3
	.byte	4
	.byte	5
	.byte	6
	.byte	7
	.byte	0
	.byte	2
	.byte	4
	.byte	6
	.byte	8
	.byte	10
	.byte	12
	.byte	14
	.byte	0
	.byte	1
	.byte	2
	.byte	3
	.byte	4
	.byte	5
	.byte	6
	.byte	7
	.byte	0
	.byte	2
	.byte	4
	.byte	6
	.byte	8
	.byte	10
	.byte	12
	.byte	14
	.align 32
.LC3:
	.byte	32
	.byte	33
	.byte	34
	.byte	35
	.byte	36
	.byte	37
	.byte	38
	.byte	39
	.byte	40
	.byte	41
	.byte	42
	.byte	43
	.byte	44
	.byte	45
	.byte	46
	.byte	47
	.byte	48
	.byte	49
	.byte	50
	.byte	51
	.byte	52
	.byte	53
	.byte	54
	.byte	55
	.byte	56
	.byte	57
	.byte	58
	.byte	59
	.byte	60
	.byte	61
	.byte	62
	.byte	63
	.section	.rodata.cst16,"aM",@progbits,16
	.align 16
.LC6:
	.byte	2
	.byte	2
	.byte	2
	.byte	2
	.byte	2
	.byte	2
	.byte	2
	.byte	2
	.byte	2
	.byte	2
	.byte	2
	.byte	2
	.byte	2
	.byte	2
	.byte	2
	.byte	2
	.ident	"GCC: (Debian 14.2.0-19) 14.2.0"
	.section	.note.GNU-stack,"",@progbits
