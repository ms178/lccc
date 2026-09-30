	.file	"k_strcmp_signed.c"
	.text
	.p2align 4
	.type	signed_strcmp_loop, @function
signed_strcmp_loop:
.LFB0:
	.cfi_startproc
	movzbl	(%rdi), %eax
	testb	%al, %al
	jne	.L2
	jmp	.L8
	.p2align 5
	.p2align 4,,10
	.p2align 3
.L4:
	movzbl	1(%rdi), %eax
	addq	$1, %rdi
	leaq	1(%rsi), %rdx
	testb	%al, %al
	je	.L9
	movq	%rdx, %rsi
.L2:
	movzbl	(%rsi), %edx
	cmpb	%al, %dl
	je	.L4
.L3:
	subl	%edx, %eax
	ret
	.p2align 4,,10
	.p2align 3
.L9:
	movzbl	1(%rsi), %edx
	xorl	%eax, %eax
	subl	%edx, %eax
	ret
.L8:
	movzbl	(%rsi), %edx
	xorl	%eax, %eax
	jmp	.L3
	.cfi_endproc
.LFE0:
	.size	signed_strcmp_loop, .-signed_strcmp_loop
	.p2align 4
	.globl	bench_setup
	.type	bench_setup, @function
bench_setup:
.LFB1:
	.cfi_startproc
	movl	$8, %esi
	xorl	%eax, %eax
	vmovdqa	.LC0(%rip), %ymm4
	leaq	left(%rip), %rcx
	vmovd	%esi, %xmm10
	movl	$32, %esi
	leaq	right(%rip), %rdx
	vmovd	%esi, %xmm9
	movl	$16, %esi
	vpbroadcastd	%xmm10, %ymm10
	vmovd	%esi, %xmm8
	movl	$24, %esi
	vpbroadcastd	%xmm9, %ymm9
	vmovd	%esi, %xmm7
	movl	$-1307163959, %esi
	vpbroadcastd	%xmm8, %ymm8
	vmovd	%esi, %xmm2
	movl	$65535, %esi
	vpbroadcastd	%xmm7, %ymm7
	vmovd	%esi, %xmm3
	movl	$16711935, %esi
	vpbroadcastd	%xmm2, %ymm2
	vmovd	%esi, %xmm5
	movl	$1633771873, %esi
	vpbroadcastd	%xmm3, %ymm3
	vmovd	%esi, %xmm6
	vpbroadcastd	%xmm5, %ymm5
	vpbroadcastd	%xmm6, %ymm6
	.p2align 4
	.p2align 3
.L11:
	vpaddd	%ymm10, %ymm4, %ymm12
	vmovdqa	%ymm4, %ymm11
	vpaddd	%ymm9, %ymm4, %ymm4
	vpslld	$4, %ymm12, %ymm1
	vpslld	$4, %ymm11, %ymm0
	vpaddd	%ymm12, %ymm1, %ymm1
	vpaddd	%ymm8, %ymm11, %ymm12
	vpaddd	%ymm11, %ymm0, %ymm0
	vpslld	$4, %ymm12, %ymm13
	vpaddd	%ymm7, %ymm11, %ymm11
	vpaddd	%ymm12, %ymm13, %ymm13
	vpslld	$4, %ymm11, %ymm12
	vpaddd	%ymm11, %ymm12, %ymm11
	vpmuludq	%ymm2, %ymm0, %ymm14
	vpsrlq	$32, %ymm0, %ymm12
	vpmuludq	%ymm2, %ymm12, %ymm12
	vpshufd	$245, %ymm14, %ymm14
	vpblendd	$85, %ymm14, %ymm12, %ymm12
	vpsrld	$4, %ymm12, %ymm12
	vpslld	$1, %ymm12, %ymm14
	vpaddd	%ymm12, %ymm14, %ymm14
	vpslld	$3, %ymm14, %ymm14
	vpsubd	%ymm12, %ymm14, %ymm12
	vpmuludq	%ymm2, %ymm1, %ymm14
	vpsubd	%ymm12, %ymm0, %ymm0
	vpsrlq	$32, %ymm1, %ymm12
	vpmuludq	%ymm2, %ymm12, %ymm12
	vpand	%ymm0, %ymm3, %ymm0
	vpshufd	$245, %ymm14, %ymm14
	vpblendd	$85, %ymm14, %ymm12, %ymm12
	vpsrld	$4, %ymm12, %ymm12
	vpslld	$1, %ymm12, %ymm14
	vpaddd	%ymm12, %ymm14, %ymm14
	vpslld	$3, %ymm14, %ymm14
	vpsubd	%ymm12, %ymm14, %ymm12
	vpsubd	%ymm12, %ymm1, %ymm1
	vpsrlq	$32, %ymm13, %ymm12
	vpand	%ymm1, %ymm3, %ymm1
	vpmuludq	%ymm2, %ymm12, %ymm12
	vpackusdw	%ymm1, %ymm0, %ymm0
	vpmuludq	%ymm2, %ymm13, %ymm1
	vpermq	$216, %ymm0, %ymm0
	vpand	%ymm0, %ymm5, %ymm0
	vpshufd	$245, %ymm1, %ymm1
	vpblendd	$85, %ymm1, %ymm12, %ymm12
	vpsrld	$4, %ymm12, %ymm12
	vpslld	$1, %ymm12, %ymm1
	vpaddd	%ymm12, %ymm1, %ymm1
	vpslld	$3, %ymm1, %ymm1
	vpsubd	%ymm12, %ymm1, %ymm1
	vpsrlq	$32, %ymm11, %ymm12
	vpsubd	%ymm1, %ymm13, %ymm1
	vpmuludq	%ymm2, %ymm11, %ymm13
	vpmuludq	%ymm2, %ymm12, %ymm12
	vpand	%ymm1, %ymm3, %ymm1
	vpshufd	$245, %ymm13, %ymm13
	vpblendd	$85, %ymm13, %ymm12, %ymm12
	vpsrld	$4, %ymm12, %ymm12
	vpslld	$1, %ymm12, %ymm13
	vpaddd	%ymm12, %ymm13, %ymm13
	vpslld	$3, %ymm13, %ymm13
	vpsubd	%ymm12, %ymm13, %ymm12
	vpsubd	%ymm12, %ymm11, %ymm11
	vpand	%ymm11, %ymm3, %ymm11
	vpackusdw	%ymm11, %ymm1, %ymm1
	vpermq	$216, %ymm1, %ymm1
	vpand	%ymm1, %ymm5, %ymm1
	vpackuswb	%ymm1, %ymm0, %ymm0
	vpermq	$216, %ymm0, %ymm0
	vpaddb	%ymm0, %ymm6, %ymm0
	vmovdqa	%ymm0, (%rcx,%rax)
	vmovdqa	%ymm0, (%rdx,%rax)
	addq	$32, %rax
	cmpq	$736, %rax
	jne	.L11
	vmovdqa	.LC9(%rip), %xmm0
	vmovdqa	%xmm0, 736+left(%rip)
	vmovdqa	%xmm0, 736+right(%rip)
	vmovdqa	.LC10(%rip), %xmm0
	vmovdqa	%xmm0, 752+left(%rip)
	vmovdqa	.LC11(%rip), %xmm0
	vmovdqa	%xmm0, 752+right(%rip)
	vzeroupper
	ret
	.cfi_endproc
.LFE1:
	.size	bench_setup, .-bench_setup
	.p2align 4
	.globl	bench_run
	.type	bench_run, @function
bench_run:
.LFB2:
	.cfi_startproc
	xorl	%ecx, %ecx
	xorl	%r8d, %r8d
	leaq	right(%rip), %r10
	leaq	left(%rip), %r9
	.p2align 4
	.p2align 3
.L14:
	leaq	(%r10,%rcx), %rsi
	leaq	(%r9,%rcx), %rdi
	addq	$1, %rcx
	call	signed_strcmp_loop
	movl	%eax, %eax
	addq	%rax, %r8
	cmpq	$64, %rcx
	jne	.L14
	movq	%r8, %rax
	ret
	.cfi_endproc
.LFE2:
	.size	bench_run, .-bench_run
	.local	right
	.comm	right,768,32
	.local	left
	.comm	left,768,32
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
	.section	.rodata.cst16,"aM",@progbits,16
	.align 16
.LC9:
	.byte	97
	.byte	114
	.byte	108
	.byte	102
	.byte	119
	.byte	113
	.byte	107
	.byte	101
	.byte	118
	.byte	112
	.byte	106
	.byte	100
	.byte	117
	.byte	111
	.byte	105
	.byte	99
	.align 16
.LC10:
	.byte	116
	.byte	110
	.byte	104
	.byte	98
	.byte	115
	.byte	109
	.byte	103
	.byte	97
	.byte	114
	.byte	108
	.byte	102
	.byte	119
	.byte	113
	.byte	107
	.byte	101
	.byte	0
	.align 16
.LC11:
	.byte	116
	.byte	110
	.byte	104
	.byte	98
	.byte	115
	.byte	109
	.byte	103
	.byte	98
	.byte	114
	.byte	108
	.byte	102
	.byte	119
	.byte	113
	.byte	107
	.byte	101
	.byte	0
	.ident	"GCC: (Debian 14.2.0-19) 14.2.0"
	.section	.note.GNU-stack,"",@progbits
