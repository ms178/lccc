	.file	"moving_stats.c"
	.text
	.section	.rodata.str1.1,"aMS",@progbits,1
.LC9:
	.string	"%llx\n"
	.section	.text.startup,"ax",@progbits
	.p2align 4
	.globl	main
	.type	main, @function
main:
.LFB12:
	.cfi_startproc
	movl	$8, %esi
	pushq	%rbp
	.cfi_def_cfa_offset 16
	.cfi_offset 6, -16
	leaq	a.3(%rip), %rax
	vmovdqa	.LC0(%rip), %ymm4
	vmovd	%esi, %xmm11
	movl	$16, %esi
	leaq	2048(%rax), %rcx
	movq	%rax, %rdx
	vmovd	%esi, %xmm10
	movl	$717, %esi
	vpbroadcastd	%xmm11, %ymm11
	vmovd	%esi, %xmm8
	movl	$41, %esi
	movq	%rsp, %rbp
	.cfi_def_cfa_register 6
	andq	$-32, %rsp
	vmovd	%esi, %xmm7
	movl	$1098962147, %esi
	vpbroadcastd	%xmm10, %ymm10
	vmovd	%esi, %xmm3
	movl	$2001, %esi
	vpbroadcastd	%xmm8, %ymm8
	vmovd	%esi, %xmm6
	movl	$65535, %esi
	vpbroadcastd	%xmm7, %ymm7
	vmovd	%esi, %xmm5
	movl	$-65471464, %esi
	vpbroadcastd	%xmm3, %ymm3
	vmovd	%esi, %xmm9
	vpbroadcastd	%xmm6, %ymm6
	vpbroadcastd	%xmm5, %ymm5
	vpbroadcastd	%xmm9, %ymm9
	.p2align 4
	.p2align 3
.L2:
	vmovdqa	%ymm4, %ymm0
	vpaddd	%ymm11, %ymm4, %ymm1
	vpaddd	%ymm10, %ymm4, %ymm4
	addq	$32, %rdx
	vpmulld	%ymm8, %ymm0, %ymm0
	vpmulld	%ymm8, %ymm1, %ymm1
	vpaddd	%ymm7, %ymm0, %ymm0
	vpaddd	%ymm7, %ymm1, %ymm1
	vpmuldq	%ymm3, %ymm0, %ymm12
	vpsrlq	$32, %ymm0, %ymm2
	vpmuldq	%ymm3, %ymm2, %ymm2
	vpshufd	$245, %ymm12, %ymm12
	vpblendd	$85, %ymm12, %ymm2, %ymm2
	vpmuldq	%ymm3, %ymm1, %ymm12
	vpsrad	$9, %ymm2, %ymm2
	vpmulld	%ymm6, %ymm2, %ymm2
	vpshufd	$245, %ymm12, %ymm12
	vpsubd	%ymm2, %ymm0, %ymm0
	vpsrlq	$32, %ymm1, %ymm2
	vpmuldq	%ymm3, %ymm2, %ymm2
	vpand	%ymm0, %ymm5, %ymm0
	vpblendd	$85, %ymm12, %ymm2, %ymm2
	vpsrad	$9, %ymm2, %ymm2
	vpmulld	%ymm6, %ymm2, %ymm2
	vpsubd	%ymm2, %ymm1, %ymm1
	vpand	%ymm1, %ymm5, %ymm1
	vpackusdw	%ymm1, %ymm0, %ymm0
	vpermq	$216, %ymm0, %ymm0
	vpaddw	%ymm0, %ymm9, %ymm0
	vmovdqa	%ymm0, -32(%rdx)
	cmpq	%rdx, %rcx
	jne	.L2
	leaq	sums.2(%rip), %rdi
	leaq	mns.1(%rip), %r9
	leaq	mxs.0(%rip), %r8
	movq	%rdi, %rdx
	movq	%r9, %rsi
	movq	%r8, %rcx
	leaq	2016+a.3(%rip), %r10
	.p2align 4
	.p2align 3
.L3:
	vmovdqa	(%rax), %ymm2
	vmovdqu	2(%rax), %ymm6
	addq	$32, %rax
	addq	$64, %rdx
	vmovdqu	-28(%rax), %ymm1
	vmovdqu	-26(%rax), %ymm14
	addq	$32, %rsi
	addq	$32, %rcx
	vpmovsxwd	%xmm6, %ymm15
	vpmovsxwd	%xmm2, %ymm0
	vmovdqu	-24(%rax), %ymm3
	vmovdqu	-22(%rax), %ymm10
	vpaddd	%ymm15, %ymm0, %ymm0
	vpmovsxwd	%xmm1, %ymm15
	vmovdqu	-20(%rax), %ymm5
	vmovdqu	-18(%rax), %ymm13
	vpaddd	%ymm15, %ymm0, %ymm0
	vpmovsxwd	%xmm14, %ymm15
	vmovdqu	-16(%rax), %ymm9
	vmovdqu	-14(%rax), %ymm4
	vpaddd	%ymm15, %ymm0, %ymm0
	vpmovsxwd	%xmm3, %ymm15
	vmovdqu	-12(%rax), %ymm12
	vmovdqu	-10(%rax), %ymm8
	vpaddd	%ymm15, %ymm0, %ymm0
	vpmovsxwd	%xmm10, %ymm15
	vmovdqu	-6(%rax), %ymm11
	vmovdqu	-4(%rax), %ymm7
	vpaddd	%ymm15, %ymm0, %ymm0
	vpmovsxwd	%xmm5, %ymm15
	vpaddd	%ymm15, %ymm0, %ymm0
	vpmovsxwd	%xmm13, %ymm15
	vpaddd	%ymm15, %ymm0, %ymm0
	vpmovsxwd	%xmm9, %ymm15
	vpaddd	%ymm15, %ymm0, %ymm0
	vpmovsxwd	%xmm4, %ymm15
	vpaddd	%ymm15, %ymm0, %ymm0
	vpmovsxwd	%xmm12, %ymm15
	vpaddd	%ymm15, %ymm0, %ymm0
	vpmovsxwd	%xmm8, %ymm15
	vpaddd	%ymm15, %ymm0, %ymm0
	vpmovsxwd	-8(%rax), %ymm15
	vpaddd	%ymm15, %ymm0, %ymm0
	vpmovsxwd	%xmm11, %ymm15
	vpaddd	%ymm15, %ymm0, %ymm0
	vpmovsxwd	%xmm7, %ymm15
	vpaddd	%ymm15, %ymm0, %ymm0
	vpmovsxwd	-2(%rax), %ymm15
	vpaddd	%ymm15, %ymm0, %ymm0
	vextracti128	$0x1, %ymm6, %xmm15
	vmovdqa	%ymm0, -64(%rdx)
	vextracti128	$0x1, %ymm2, %xmm0
	vpmovsxwd	%xmm15, %ymm15
	vpmovsxwd	%xmm0, %ymm0
	vpaddd	%ymm15, %ymm0, %ymm0
	vextracti128	$0x1, %ymm1, %xmm15
	vpmovsxwd	%xmm15, %ymm15
	vpaddd	%ymm15, %ymm0, %ymm0
	vextracti128	$0x1, %ymm14, %xmm15
	vpmovsxwd	%xmm15, %ymm15
	vpaddd	%ymm15, %ymm0, %ymm0
	vextracti128	$0x1, %ymm3, %xmm15
	vpmovsxwd	%xmm15, %ymm15
	vpaddd	%ymm15, %ymm0, %ymm0
	vextracti128	$0x1, %ymm10, %xmm15
	vpmovsxwd	%xmm15, %ymm15
	vpaddd	%ymm15, %ymm0, %ymm0
	vextracti128	$0x1, %ymm5, %xmm15
	vpmovsxwd	%xmm15, %ymm15
	vpaddd	%ymm15, %ymm0, %ymm0
	vextracti128	$0x1, %ymm13, %xmm15
	vpmovsxwd	%xmm15, %ymm15
	vpaddd	%ymm15, %ymm0, %ymm0
	vextracti128	$0x1, %ymm9, %xmm15
	vpmovsxwd	%xmm15, %ymm15
	vpaddd	%ymm15, %ymm0, %ymm0
	vextracti128	$0x1, %ymm4, %xmm15
	vpmovsxwd	%xmm15, %ymm15
	vpaddd	%ymm15, %ymm0, %ymm0
	vextracti128	$0x1, %ymm12, %xmm15
	vpmovsxwd	%xmm15, %ymm15
	vpaddd	%ymm15, %ymm0, %ymm0
	vextracti128	$0x1, %ymm8, %xmm15
	vpmovsxwd	%xmm15, %ymm15
	vpaddd	%ymm15, %ymm0, %ymm0
	vmovdqu	-8(%rax), %ymm15
	vextracti128	$0x1, %ymm15, %xmm15
	vpmovsxwd	%xmm15, %ymm15
	vpaddd	%ymm15, %ymm0, %ymm0
	vextracti128	$0x1, %ymm11, %xmm15
	vpmovsxwd	%xmm15, %ymm15
	vpaddd	%ymm15, %ymm0, %ymm0
	vextracti128	$0x1, %ymm7, %xmm15
	vpmovsxwd	%xmm15, %ymm15
	vpaddd	%ymm15, %ymm0, %ymm0
	vmovdqu	-2(%rax), %ymm15
	vextracti128	$0x1, %ymm15, %xmm15
	vpmovsxwd	%xmm15, %ymm15
	vpaddd	%ymm15, %ymm0, %ymm0
	vpminsw	%ymm10, %ymm3, %ymm15
	vmovdqa	%ymm0, -32(%rdx)
	vpminsw	%ymm14, %ymm1, %ymm0
	vpminsw	%ymm9, %ymm15, %ymm15
	vpminsw	%ymm13, %ymm0, %ymm0
	vpminsw	%ymm8, %ymm15, %ymm15
	vpminsw	%ymm12, %ymm0, %ymm0
	vpminsw	%ymm7, %ymm15, %ymm15
	vpminsw	%ymm11, %ymm0, %ymm0
	vpminsw	%ymm15, %ymm0, %ymm0
	vpminsw	%ymm6, %ymm2, %ymm15
	vpminsw	%ymm5, %ymm15, %ymm15
	vpminsw	%ymm4, %ymm15, %ymm15
	vpminsw	-8(%rax), %ymm15, %ymm15
	vpminsw	-2(%rax), %ymm15, %ymm15
	vpminsw	%ymm15, %ymm0, %ymm0
	vmovdqa	%ymm0, -32(%rsi)
	vpmaxsw	%ymm14, %ymm1, %ymm0
	vpmaxsw	%ymm10, %ymm3, %ymm1
	vpmaxsw	%ymm13, %ymm0, %ymm0
	vpmaxsw	%ymm9, %ymm1, %ymm1
	vpmaxsw	%ymm12, %ymm0, %ymm0
	vpmaxsw	%ymm8, %ymm1, %ymm1
	vpmaxsw	%ymm7, %ymm1, %ymm1
	vpmaxsw	%ymm11, %ymm0, %ymm0
	vpmaxsw	%ymm1, %ymm0, %ymm0
	vpmaxsw	%ymm6, %ymm2, %ymm1
	vpmaxsw	%ymm5, %ymm1, %ymm1
	vpmaxsw	%ymm4, %ymm1, %ymm1
	vpmaxsw	-8(%rax), %ymm1, %ymm1
	vpmaxsw	-2(%rax), %ymm1, %ymm1
	vpmaxsw	%ymm1, %ymm0, %ymm0
	vmovdqa	%ymm0, -32(%rcx)
	cmpq	%rax, %r10
	jne	.L3
	vmovdqa	2016+a.3(%rip), %ymm3
	xorl	%eax, %eax
	xorl	%esi, %esi
	vextracti128	$0x1, %ymm3, %xmm0
	vpmovsxwd	%xmm3, %ymm1
	vpmovsxwd	%xmm0, %ymm2
	vpaddd	%ymm1, %ymm2, %ymm2
	vextracti128	$0x1, %ymm2, %xmm1
	vpaddd	%xmm2, %xmm1, %xmm1
	vpsrldq	$8, %xmm1, %xmm2
	vpaddd	%xmm2, %xmm1, %xmm1
	vpsrldq	$4, %xmm1, %xmm2
	vpaddd	%xmm2, %xmm1, %xmm2
	vpminsw	%xmm3, %xmm0, %xmm1
	vpmaxsw	%xmm3, %xmm0, %xmm0
	vpsrldq	$8, %xmm1, %xmm4
	vmovd	%xmm2, 4032+sums.2(%rip)
	vpsrldq	$8, %xmm0, %xmm3
	vpminsw	%xmm4, %xmm1, %xmm1
	vpmaxsw	%xmm3, %xmm0, %xmm0
	vpsrldq	$4, %xmm1, %xmm4
	vpsrldq	$4, %xmm0, %xmm3
	vpminsw	%xmm4, %xmm1, %xmm1
	vpmaxsw	%xmm3, %xmm0, %xmm0
	vpsrldq	$2, %xmm1, %xmm4
	vpsrldq	$2, %xmm0, %xmm3
	vpminsw	%xmm4, %xmm1, %xmm1
	vpextrw	$0, %xmm1, 2016+mns.1(%rip)
	vpmaxsw	%xmm3, %xmm0, %xmm0
	vpextrw	$0, %xmm0, 2016+mxs.0(%rip)
	.p2align 4
	.p2align 3
.L4:
	movq	%rsi, %rcx
	movl	(%rdi,%rax,2), %edx
	salq	$5, %rcx
	subq	%rsi, %rcx
	addq	%rcx, %rdx
	movq	%rdx, %rcx
	salq	$5, %rcx
	subq	%rdx, %rcx
	movswl	(%r9,%rax), %edx
	addl	$1000, %edx
	addq	%rcx, %rdx
	movq	%rdx, %rcx
	salq	$5, %rcx
	subq	%rdx, %rcx
	movswl	(%r8,%rax), %edx
	addq	$2, %rax
	leal	1000(%rdx), %esi
	addq	%rcx, %rsi
	cmpq	$2018, %rax
	jne	.L4
	xorl	%eax, %eax
	leaq	.LC9(%rip), %rdi
	vzeroupper
	call	printf@PLT
	xorl	%eax, %eax
	leave
	.cfi_def_cfa 7, 8
	ret
	.cfi_endproc
.LFE12:
	.size	main, .-main
	.local	mxs.0
	.comm	mxs.0,2048,32
	.local	mns.1
	.comm	mns.1,2048,32
	.local	sums.2
	.comm	sums.2,4096,32
	.local	a.3
	.comm	a.3,2048,32
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
