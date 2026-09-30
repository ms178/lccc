	.file	"ascii_case_fold.c"
	.text
	.section	.text.startup,"ax",@progbits
	.p2align 4
	.globl	main
	.type	main, @function
main:
.LFB2:
	.cfi_startproc
	pushq	%rbp
	.cfi_def_cfa_offset 16
	.cfi_offset 6, -16
	movl	$8, %ecx
	leaq	data(%rip), %r8
	vmovd	%ecx, %xmm15
	movl	$16, %ecx
	movq	%r8, %rax
	vmovd	%ecx, %xmm6
	movl	$24, %ecx
	leaq	65536(%r8), %rdx
	movq	%rsp, %rbp
	.cfi_def_cfa_register 6
	andq	$-32, %rsp
	vpbroadcastd	%xmm6, %ymm6
	subq	$8, %rsp
	vpbroadcastd	%xmm15, %ymm15
	vmovdqa	%ymm6, -24(%rsp)
	vmovd	%ecx, %xmm6
	movl	$32, %ecx
	vpbroadcastd	%xmm6, %ymm6
	vmovdqa	%ymm6, -56(%rsp)
	vmovd	%ecx, %xmm6
	movl	$1103515245, %ecx
	vmovdqa	.LC0(%rip), %ymm10
	vmovd	%ecx, %xmm13
	movl	$12345, %ecx
	vpbroadcastd	%xmm6, %ymm6
	vmovd	%ecx, %xmm12
	movl	$1491936009, %ecx
	vmovdqa	%ymm6, -88(%rsp)
	vpbroadcastd	%xmm13, %ymm13
	vmovd	%ecx, %xmm6
	movl	$65535, %ecx
	vpbroadcastd	%xmm12, %ymm12
	vmovd	%ecx, %xmm11
	movl	$16711935, %ecx
	vpbroadcastd	%xmm6, %ymm6
	vmovd	%ecx, %xmm14
	movl	$538976288, %ecx
	vpbroadcastd	%xmm11, %ymm11
	vmovd	%ecx, %xmm7
	vpbroadcastd	%xmm14, %ymm14
	vpbroadcastd	%xmm7, %ymm7
	vmovdqa	%ymm7, -120(%rsp)
	.p2align 4
	.p2align 3
.L2:
	vmovdqa	%ymm10, %ymm0
	vpaddd	%ymm15, %ymm10, %ymm1
	vpaddd	-24(%rsp), %ymm10, %ymm3
	addq	$32, %rax
	vpmulld	%ymm13, %ymm0, %ymm0
	vpmulld	%ymm13, %ymm1, %ymm1
	vpaddd	-56(%rsp), %ymm10, %ymm2
	vpmulld	%ymm13, %ymm3, %ymm3
	vpaddd	-88(%rsp), %ymm10, %ymm10
	vpmulld	%ymm13, %ymm2, %ymm2
	vpaddd	%ymm12, %ymm0, %ymm0
	vpaddd	%ymm12, %ymm1, %ymm1
	vpsrld	$16, %ymm0, %ymm0
	vpsrld	$16, %ymm1, %ymm1
	vpaddd	%ymm12, %ymm3, %ymm3
	vpmuludq	%ymm6, %ymm0, %ymm4
	vpsrlq	$32, %ymm0, %ymm5
	vpaddd	%ymm12, %ymm2, %ymm2
	vpmuludq	%ymm6, %ymm5, %ymm5
	vpsrlq	$32, %ymm1, %ymm9
	vpmuludq	%ymm6, %ymm9, %ymm9
	vpsrld	$16, %ymm3, %ymm3
	vpsrlq	$32, %ymm3, %ymm7
	vpsrld	$16, %ymm2, %ymm2
	vpmuludq	%ymm6, %ymm7, %ymm7
	vpmuludq	%ymm6, %ymm2, %ymm8
	vpshufd	$245, %ymm4, %ymm4
	vpblendd	$85, %ymm4, %ymm5, %ymm5
	vpmuludq	%ymm6, %ymm1, %ymm4
	vpshufd	$245, %ymm8, %ymm8
	vpshufd	$245, %ymm4, %ymm4
	vpblendd	$85, %ymm4, %ymm9, %ymm9
	vpmuludq	%ymm6, %ymm3, %ymm4
	vpshufd	$245, %ymm4, %ymm4
	vpblendd	$85, %ymm4, %ymm7, %ymm7
	vpsrlq	$32, %ymm2, %ymm4
	vpmuludq	%ymm6, %ymm4, %ymm4
	vpblendd	$85, %ymm8, %ymm4, %ymm4
	vpsubd	%ymm5, %ymm0, %ymm8
	vpsrld	$1, %ymm8, %ymm8
	vpaddd	%ymm5, %ymm8, %ymm8
	vpsrld	$6, %ymm8, %ymm8
	vpslld	$1, %ymm8, %ymm5
	vpaddd	%ymm8, %ymm5, %ymm5
	vpslld	$5, %ymm5, %ymm5
	vpsubd	%ymm8, %ymm5, %ymm5
	vpsubd	%ymm5, %ymm0, %ymm0
	vpsubd	%ymm9, %ymm1, %ymm5
	vpsrld	$1, %ymm5, %ymm5
	vpand	%ymm0, %ymm11, %ymm0
	vpaddd	%ymm9, %ymm5, %ymm5
	vpsrld	$6, %ymm5, %ymm5
	vpslld	$1, %ymm5, %ymm8
	vpaddd	%ymm5, %ymm8, %ymm8
	vpslld	$5, %ymm8, %ymm8
	vpsubd	%ymm5, %ymm8, %ymm5
	vpsubd	%ymm5, %ymm1, %ymm1
	vpsubd	%ymm7, %ymm3, %ymm5
	vpsrld	$1, %ymm5, %ymm5
	vpand	%ymm1, %ymm11, %ymm1
	vpaddd	%ymm7, %ymm5, %ymm5
	vpackusdw	%ymm1, %ymm0, %ymm0
	vpsrld	$6, %ymm5, %ymm5
	vpermq	$216, %ymm0, %ymm0
	vpslld	$1, %ymm5, %ymm1
	vpand	%ymm0, %ymm14, %ymm0
	vpaddd	%ymm5, %ymm1, %ymm1
	vpslld	$5, %ymm1, %ymm1
	vpsubd	%ymm5, %ymm1, %ymm1
	vpsubd	%ymm1, %ymm3, %ymm1
	vpsubd	%ymm4, %ymm2, %ymm3
	vpsrld	$1, %ymm3, %ymm3
	vpand	%ymm1, %ymm11, %ymm1
	vpaddd	%ymm4, %ymm3, %ymm3
	vpsrld	$6, %ymm3, %ymm3
	vpslld	$1, %ymm3, %ymm4
	vpaddd	%ymm3, %ymm4, %ymm4
	vpslld	$5, %ymm4, %ymm4
	vpsubd	%ymm3, %ymm4, %ymm3
	vpsubd	%ymm3, %ymm2, %ymm2
	vpand	%ymm2, %ymm11, %ymm2
	vpackusdw	%ymm2, %ymm1, %ymm1
	vpermq	$216, %ymm1, %ymm1
	vpand	%ymm1, %ymm14, %ymm1
	vpackuswb	%ymm1, %ymm0, %ymm0
	vpermq	$216, %ymm0, %ymm0
	vpaddb	-120(%rsp), %ymm0, %ymm0
	vmovdqa	%ymm0, -32(%rax)
	cmpq	%rax, %rdx
	jne	.L2
	xorl	%eax, %eax
	xorl	%ecx, %ecx
	leaq	folded(%rip), %r9
	.p2align 6
	.p2align 4
	.p2align 3
.L6:
	movzbl	(%r8,%rcx), %esi
	leal	-65(%rsi), %edi
	movl	%esi, %edx
	cmpl	$25, %edi
	ja	.L3
	addl	$32, %edx
	movb	%dl, (%r9,%rcx)
	movl	%eax, %edx
	addq	$1, %rcx
	sall	$5, %edx
	addl	%edx, %eax
	leal	32(%rsi,%rax), %eax
	cmpq	$65536, %rcx
	jne	.L6
.L5:
	cmpb	$32, folded(%rip)
	movl	$1, %edx
	jne	.L1
	cmpb	$55, 1+folded(%rip)
	je	.L17
.L1:
	vzeroupper
	movl	%edx, %eax
	leave
	.cfi_remember_state
	.cfi_def_cfa 7, 8
	ret
	.p2align 4,,10
	.p2align 3
.L3:
	.cfi_restore_state
	movl	%eax, %edx
	movb	%sil, (%r9,%rcx)
	addq	$1, %rcx
	sall	$5, %edx
	addl	%edx, %eax
	addl	%esi, %eax
	cmpq	$65536, %rcx
	jne	.L6
	jmp	.L5
.L17:
	cmpb	$110, 2+folded(%rip)
	jne	.L1
	xorl	%edx, %edx
	cmpl	$-1723667183, %eax
	setne	%dl
	addl	%edx, %edx
	jmp	.L1
	.cfi_endproc
.LFE2:
	.size	main, .-main
	.local	folded
	.comm	folded,65536,32
	.local	data
	.comm	data,65536,32
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
