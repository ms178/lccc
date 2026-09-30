	.file	"matmul.c"
	.text
	.p2align 4
	.globl	matmul
	.type	matmul, @function
matmul:
.LFB11:
	.cfi_startproc
	xorl	%r9d, %r9d
	leaq	C(%rip), %r11
	leaq	A(%rip), %r10
	leaq	524288+B(%rip), %r8
.L2:
	leaq	(%r10,%r9), %rdi
	leaq	B(%rip), %rcx
	leaq	(%r11,%r9), %rdx
.L6:
	vbroadcastsd	(%rdi), %ymm2
	vbroadcastsd	8(%rdi), %ymm1
	leaq	2048(%rcx), %rsi
	xorl	%eax, %eax
	.p2align 6
	.p2align 4
	.p2align 3
.L3:
	vmovapd	(%rcx,%rax), %ymm0
	vfmadd213pd	(%rdx,%rax), %ymm2, %ymm0
	vmovapd	%ymm0, (%rdx,%rax)
	vfmadd231pd	(%rsi,%rax), %ymm1, %ymm0
	vmovapd	%ymm0, (%rdx,%rax)
	addq	$32, %rax
	cmpq	$2048, %rax
	jne	.L3
	addq	$4096, %rcx
	addq	$16, %rdi
	cmpq	%r8, %rcx
	jne	.L6
	addq	$2048, %r9
	cmpq	$524288, %r9
	jne	.L2
	vzeroupper
	ret
	.cfi_endproc
.LFE11:
	.size	matmul, .-matmul
	.section	.rodata.str1.1,"aMS",@progbits,1
.LC5:
	.string	"matmul C[128][128] = %.4f\n"
	.section	.text.startup,"ax",@progbits
	.p2align 4
	.globl	main
	.type	main, @function
main:
.LFB12:
	.cfi_startproc
	leaq	8(%rsp), %r10
	.cfi_def_cfa 10, 0
	andq	$-32, %rsp
	xorl	%esi, %esi
	movl	$8, %eax
	pushq	-8(%r10)
	vmovd	%eax, %xmm6
	movl	$1, %eax
	leaq	A(%rip), %rcx
	pushq	%rbp
	vmovd	%eax, %xmm5
	vpbroadcastd	%xmm6, %ymm6
	leaq	B(%rip), %rdx
	vpbroadcastd	%xmm5, %ymm5
	movq	%rsp, %rbp
	.cfi_escape 0x10,0x6,0x2,0x76,0
	pushq	%r10
	.cfi_escape 0xf,0x3,0x76,0x78,0x6
	subq	$40, %rsp
	vmovdqa	.LC0(%rip), %ymm7
	vbroadcastsd	.LC3(%rip), %ymm2
.L11:
	vmovd	%esi, %xmm4
	xorl	%eax, %eax
	vmovdqa	%ymm7, %ymm3
	vpbroadcastd	%xmm4, %ymm4
	.p2align 4
	.p2align 3
.L12:
	vmovdqa	%ymm3, %ymm0
	vpaddd	%ymm6, %ymm3, %ymm3
	vpaddd	%ymm0, %ymm4, %ymm1
	vpmulld	%ymm0, %ymm4, %ymm0
	vcvtdq2pd	%xmm1, %ymm8
	vmulpd	%ymm2, %ymm8, %ymm8
	vextracti128	$0x1, %ymm1, %xmm1
	vcvtdq2pd	%xmm1, %ymm1
	vmulpd	%ymm2, %ymm1, %ymm1
	vpaddd	%ymm5, %ymm0, %ymm0
	vmovapd	%ymm8, (%rcx,%rax)
	vmovapd	%ymm1, 32(%rcx,%rax)
	vcvtdq2pd	%xmm0, %ymm1
	vextracti128	$0x1, %ymm0, %xmm0
	vmulpd	%ymm2, %ymm1, %ymm1
	vcvtdq2pd	%xmm0, %ymm0
	vmulpd	%ymm2, %ymm0, %ymm0
	vmovapd	%ymm1, (%rdx,%rax)
	vmovapd	%ymm0, 32(%rdx,%rax)
	addq	$64, %rax
	cmpq	$2048, %rax
	jne	.L12
	addl	$1, %esi
	addq	$2048, %rcx
	addq	$2048, %rdx
	cmpl	$256, %esi
	jne	.L11
	vzeroupper
	call	matmul
	vmovsd	263168+C(%rip), %xmm0
	leaq	.LC5(%rip), %rdi
	movl	$1, %eax
	vmovsd	%xmm0, -24(%rbp)
	vmovsd	-24(%rbp), %xmm0
	call	printf@PLT
	addq	$40, %rsp
	xorl	%eax, %eax
	popq	%r10
	.cfi_def_cfa 10, 0
	popq	%rbp
	leaq	-8(%r10), %rsp
	.cfi_def_cfa 7, 8
	ret
	.cfi_endproc
.LFE12:
	.size	main, .-main
	.local	C
	.comm	C,524288,32
	.local	B
	.comm	B,524288,32
	.local	A
	.comm	A,524288,32
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
.LC3:
	.long	0
	.long	1064304640
	.ident	"GCC: (Debian 14.2.0-19) 14.2.0"
	.section	.note.GNU-stack,"",@progbits
