	.file	"loop_patterns.c"
	.text
	.section	.rodata.str1.8,"aMS",@progbits,1
	.align 8
.LC1:
	.string	"sum=%ld pos=%ld max=%d scaled=%ld dot=%ld prefix=%d\n"
	.section	.text.startup,"ax",@progbits
	.p2align 4
	.globl	main
	.type	main, @function
main:
.LFB17:
	.cfi_startproc
	leaq	8(%rsp), %r10
	.cfi_def_cfa 10, 0
	andq	$-32, %rsp
	movl	$42, %eax
	pushq	-8(%r10)
	leaq	array(%rip), %rcx
	pushq	%rbp
	leaq	40000000(%rcx), %rdi
	movq	%rcx, %r9
	movq	%rcx, %rdx
	movq	%rsp, %rbp
	.cfi_escape 0x10,0x6,0x2,0x76,0
	pushq	%r12
	pushq	%r10
	.cfi_escape 0xf,0x3,0x76,0x70,0x6
	.cfi_escape 0x10,0xc,0x2,0x76,0x78
	pushq	%rbx
	subq	$24, %rsp
	.cfi_escape 0x10,0x3,0x2,0x76,0x68
	.p2align 6
	.p2align 4
	.p2align 3
.L2:
	imull	$1664525, %eax, %eax
	addq	$4, %rdx
	addl	$1013904223, %eax
	movl	%eax, %esi
	shrl	%esi
	subl	$1000000000, %esi
	movl	%esi, -4(%rdx)
	cmpq	%rdi, %rdx
	jne	.L2
	leaq	array(%rip), %rax
	vpxor	%xmm1, %xmm1, %xmm1
	.p2align 6
	.p2align 4
	.p2align 3
.L3:
	vmovdqa	(%rax), %ymm0
	addq	$32, %rax
	vpmovsxdq	%xmm0, %ymm2
	vextracti128	$0x1, %ymm0, %xmm0
	vpaddq	%ymm1, %ymm2, %ymm1
	vpmovsxdq	%xmm0, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm1
	cmpq	%rax, %rdi
	jne	.L3
	vextracti128	$0x1, %ymm1, %xmm0
	leaq	array(%rip), %rax
	vpxor	%xmm3, %xmm3, %xmm3
	vpaddq	%xmm1, %xmm0, %xmm0
	vpxor	%xmm5, %xmm5, %xmm5
	vpsrldq	$8, %xmm0, %xmm1
	vpaddq	%xmm1, %xmm0, %xmm0
	vmovq	%xmm0, %r10
	.p2align 6
	.p2align 4
	.p2align 3
.L4:
	vmovdqa	(%rax), %ymm1
	addq	$32, %rax
	vpcmpgtd	%ymm5, %ymm1, %ymm0
	vpmovsxdq	%xmm1, %ymm4
	vextracti128	$0x1, %ymm1, %xmm1
	vpmovsxdq	%xmm1, %ymm1
	vpmovsxdq	%xmm0, %ymm2
	vextracti128	$0x1, %ymm0, %xmm0
	vpand	%ymm4, %ymm2, %ymm2
	vpmovsxdq	%xmm0, %ymm0
	vpand	%ymm1, %ymm0, %ymm0
	vpaddq	%ymm2, %ymm3, %ymm3
	vpaddq	%ymm3, %ymm0, %ymm3
	cmpq	%rax, %rdi
	jne	.L4
	vextracti128	$0x1, %ymm3, %xmm0
	leaq	4+array(%rip), %rdi
	leaq	39999972(%rcx), %rdx
	vpaddq	%xmm3, %xmm0, %xmm0
	movq	%rdi, %rax
	vpsrldq	$8, %xmm0, %xmm1
	vpaddq	%xmm1, %xmm0, %xmm0
	vmovq	%xmm0, %r11
	vpbroadcastd	array(%rip), %ymm0
	.p2align 4
	.p2align 4
	.p2align 3
.L5:
	vpmaxsd	(%rax), %ymm0, %ymm0
	addq	$32, %rax
	cmpq	%rax, %rdx
	jne	.L5
	vmovdqa	%xmm0, %xmm1
	vextracti128	$0x1, %ymm0, %xmm0
	movl	$7, %esi
	xorl	%eax, %eax
	vpmaxsd	%xmm0, %xmm1, %xmm0
	leaq	buf.0(%rip), %rdx
	vmovd	39999988(%r9), %xmm2
	vpmaxsd	39999972(%r9), %xmm0, %xmm0
	vpsrldq	$8, %xmm0, %xmm1
	vpmaxsd	%xmm1, %xmm0, %xmm0
	vpsrldq	$4, %xmm0, %xmm1
	vpmaxsd	%xmm1, %xmm0, %xmm0
	vmovd	39999992(%r9), %xmm1
	vpmaxsd	%xmm2, %xmm1, %xmm1
	vmovd	39999996(%r9), %xmm2
	vpmaxsd	%xmm2, %xmm1, %xmm1
	vmovd	%esi, %xmm2
	vpmaxsd	%xmm0, %xmm1, %xmm0
	vpbroadcastd	%xmm2, %ymm2
	vmovd	%xmm0, %ebx
	.p2align 6
	.p2align 4
	.p2align 3
.L6:
	vmovdqa	(%r9,%rax), %ymm1
	vpslld	$1, %ymm1, %ymm0
	vpaddd	%ymm1, %ymm0, %ymm0
	vpaddd	%ymm2, %ymm0, %ymm0
	vmovdqa	%ymm0, (%rdx,%rax)
	addq	$32, %rax
	cmpq	$40000000, %rax
	jne	.L6
	leaq	buf.0(%rip), %rax
	leaq	40000000(%rdx), %rsi
	vpxor	%xmm1, %xmm1, %xmm1
	.p2align 6
	.p2align 4
	.p2align 3
.L7:
	vmovdqa	(%rax), %ymm0
	addq	$32, %rax
	vpmovsxdq	%xmm0, %ymm2
	vextracti128	$0x1, %ymm0, %xmm0
	vpaddq	%ymm1, %ymm2, %ymm1
	vpmovsxdq	%xmm0, %ymm0
	vpaddq	%ymm1, %ymm0, %ymm1
	cmpq	%rax, %rsi
	jne	.L7
	vextracti128	$0x1, %ymm1, %xmm0
	xorl	%eax, %eax
	vpxor	%xmm2, %xmm2, %xmm2
	vpaddq	%xmm1, %xmm0, %xmm0
	vpsrldq	$8, %xmm0, %xmm1
	vpaddq	%xmm1, %xmm0, %xmm0
	vmovq	%xmm0, %r8
	.p2align 6
	.p2align 4
	.p2align 3
.L8:
	vpermq	$216, (%rdx,%rax), %ymm0
	vpermq	$216, (%r9,%rax), %ymm1
	addq	$32, %rax
	vpshufd	$80, %ymm0, %ymm3
	vpshufd	$80, %ymm1, %ymm4
	vpshufd	$250, %ymm0, %ymm0
	vpmuldq	%ymm4, %ymm3, %ymm3
	vpshufd	$250, %ymm1, %ymm1
	vpmuldq	%ymm1, %ymm0, %ymm0
	vpaddq	%ymm2, %ymm3, %ymm2
	vpaddq	%ymm2, %ymm0, %ymm2
	cmpq	$4000000, %rax
	jne	.L8
	vextracti128	$0x1, %ymm2, %xmm0
	leaq	40000+array(%rip), %rsi
	movl	$42, %edx
	vpaddq	%xmm2, %xmm0, %xmm0
	vpsrldq	$8, %xmm0, %xmm1
	vpaddq	%xmm1, %xmm0, %xmm0
	vmovq	%xmm0, %r9
	.p2align 6
	.p2align 4
	.p2align 3
.L9:
	imull	$1664525, %edx, %edx
	addq	$4, %rcx
	leal	1013904223(%rdx), %eax
	movq	%rax, %rdx
	imulq	$1374389535, %rax, %rax
	movl	%edx, %r12d
	shrq	$37, %rax
	imull	$100, %eax, %eax
	subl	%eax, %r12d
	movl	%r12d, -4(%rcx)
	cmpq	%rcx, %rsi
	jne	.L9
	movl	array(%rip), %eax
	.p2align 4
	.p2align 4
	.p2align 3
.L10:
	addl	(%rdi), %eax
	addq	$4, %rdi
	movl	%eax, -4(%rdi)
	cmpq	%rsi, %rdi
	jne	.L10
	movl	39996+array(%rip), %eax
	subq	$8, %rsp
	movl	%ebx, %ecx
	movq	%r11, %rdx
	movq	%r10, %rsi
	leaq	.LC1(%rip), %rdi
	pushq	%rax
	xorl	%eax, %eax
	vzeroupper
	call	printf@PLT
	popq	%rax
	popq	%rdx
	leaq	-24(%rbp), %rsp
	xorl	%eax, %eax
	popq	%rbx
	popq	%r10
	.cfi_def_cfa 10, 0
	popq	%r12
	popq	%rbp
	leaq	-8(%r10), %rsp
	.cfi_def_cfa 7, 8
	ret
	.cfi_endproc
.LFE17:
	.size	main, .-main
	.local	buf.0
	.comm	buf.0,40000000,32
	.local	array
	.comm	array,40000000,32
	.ident	"GCC: (Debian 14.2.0-19) 14.2.0"
	.section	.note.GNU-stack,"",@progbits
