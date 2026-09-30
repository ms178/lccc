	.file	"sieve.c"
	.text
	.p2align 4
	.globl	count_primes
	.type	count_primes, @function
count_primes:
.LFB11:
	.cfi_startproc
	pushq	%rbp
	.cfi_def_cfa_offset 16
	.cfi_offset 6, -16
	movl	$10000001, %edx
	movl	$1, %esi
	leaq	sieve(%rip), %rcx
	movq	%rcx, %rdi
	movq	%rsp, %rbp
	.cfi_def_cfa_register 6
	andq	$-32, %rsp
	call	memset@PLT
	movl	$2, %edx
	movq	%rax, %rcx
	xorl	%eax, %eax
	movw	%ax, sieve(%rip)
	movl	$4, %eax
	jmp	.L4
	.p2align 5
	.p2align 4,,10
	.p2align 3
.L2:
	leal	1(%rsi), %eax
	addq	$1, %rdx
	imull	%eax, %eax
	cmpl	$10000000, %eax
	jg	.L27
.L4:
	cmpb	$0, (%rcx,%rdx)
	movl	%edx, %esi
	je	.L2
	cltq
	.p2align 4
	.p2align 4
	.p2align 3
.L3:
	movb	$0, (%rcx,%rax)
	addq	%rdx, %rax
	cmpl	$10000000, %eax
	jle	.L3
	jmp	.L2
.L27:
	leaq	2+sieve(%rip), %rax
	vpxor	%xmm4, %xmm4, %xmm4
	vpxor	%xmm3, %xmm3, %xmm3
	leaq	9999968(%rax), %rdx
	.p2align 4
	.p2align 3
.L5:
	vpcmpeqb	(%rax), %ymm3, %ymm0
	addq	$32, %rax
	vpcmpeqb	%ymm3, %ymm0, %ymm0
	vpmovsxbw	%xmm0, %ymm2
	vextracti128	$0x1, %ymm0, %xmm0
	vpmovsxwd	%xmm2, %ymm1
	vextracti128	$0x1, %ymm2, %xmm2
	vpmovsxbw	%xmm0, %ymm0
	vpsubd	%ymm1, %ymm4, %ymm1
	vpmovsxwd	%xmm2, %ymm2
	vpsubd	%ymm2, %ymm1, %ymm1
	vpmovsxwd	%xmm0, %ymm2
	vextracti128	$0x1, %ymm0, %xmm0
	vpsubd	%ymm2, %ymm1, %ymm1
	vpmovsxwd	%xmm0, %ymm0
	vpsubd	%ymm0, %ymm1, %ymm4
	cmpq	%rdx, %rax
	jne	.L5
	vpxor	%xmm1, %xmm1, %xmm1
	cmpb	$1, 9999986+sieve(%rip)
	vpcmpeqb	9999970+sieve(%rip), %xmm1, %xmm0
	vpcmpeqb	%xmm1, %xmm0, %xmm0
	vpmovsxbw	%xmm0, %xmm2
	vpsrldq	$8, %xmm0, %xmm0
	vpmovsxbw	%xmm0, %xmm1
	vmovdqa	%xmm4, %xmm0
	vextracti128	$0x1, %ymm4, %xmm4
	vpmovsxwd	%xmm2, %xmm3
	vpsrldq	$8, %xmm2, %xmm2
	vpaddd	%xmm4, %xmm0, %xmm0
	vpsubd	%xmm3, %xmm0, %xmm0
	vpmovsxwd	%xmm2, %xmm2
	vpsubd	%xmm2, %xmm0, %xmm0
	vpmovsxwd	%xmm1, %xmm2
	vpsrldq	$8, %xmm1, %xmm1
	vpsubd	%xmm2, %xmm0, %xmm0
	vpmovsxwd	%xmm1, %xmm1
	vpsubd	%xmm1, %xmm0, %xmm0
	vpsrldq	$8, %xmm0, %xmm1
	vpaddd	%xmm1, %xmm0, %xmm0
	vpsrldq	$4, %xmm0, %xmm1
	vpaddd	%xmm1, %xmm0, %xmm0
	vmovd	%xmm0, %eax
	sbbl	$-1, %eax
	cmpb	$1, 9999987+sieve(%rip)
	sbbl	$-1, %eax
	cmpb	$1, 9999988+sieve(%rip)
	sbbl	$-1, %eax
	cmpb	$1, 9999989+sieve(%rip)
	sbbl	$-1, %eax
	cmpb	$1, 9999990+sieve(%rip)
	sbbl	$-1, %eax
	cmpb	$1, 9999991+sieve(%rip)
	sbbl	$-1, %eax
	cmpb	$1, 9999992+sieve(%rip)
	sbbl	$-1, %eax
	cmpb	$1, 9999993+sieve(%rip)
	sbbl	$-1, %eax
	cmpb	$1, 9999994+sieve(%rip)
	sbbl	$-1, %eax
	cmpb	$1, 9999995+sieve(%rip)
	sbbl	$-1, %eax
	cmpb	$1, 9999996+sieve(%rip)
	sbbl	$-1, %eax
	cmpb	$1, 9999997+sieve(%rip)
	sbbl	$-1, %eax
	cmpb	$1, 9999998+sieve(%rip)
	sbbl	$-1, %eax
	cmpb	$1, 9999999+sieve(%rip)
	sbbl	$-1, %eax
	cmpb	$1, 10000000+sieve(%rip)
	sbbl	$-1, %eax
	vzeroupper
	leave
	.cfi_def_cfa 7, 8
	ret
	.cfi_endproc
.LFE11:
	.size	count_primes, .-count_primes
	.section	.rodata.str1.1,"aMS",@progbits,1
.LC0:
	.string	"primes up to %d: %d\n"
	.section	.text.startup,"ax",@progbits
	.p2align 4
	.globl	main
	.type	main, @function
main:
.LFB12:
	.cfi_startproc
	subq	$24, %rsp
	.cfi_def_cfa_offset 32
	call	count_primes
	movl	$10000000, %esi
	leaq	.LC0(%rip), %rdi
	movl	%eax, 12(%rsp)
	movl	12(%rsp), %edx
	xorl	%eax, %eax
	call	printf@PLT
	xorl	%eax, %eax
	addq	$24, %rsp
	.cfi_def_cfa_offset 8
	ret
	.cfi_endproc
.LFE12:
	.size	main, .-main
	.local	sieve
	.comm	sieve,10000001,32
	.ident	"GCC: (Debian 14.2.0-19) 14.2.0"
	.section	.note.GNU-stack,"",@progbits
