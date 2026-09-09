	.file	"sha256_transform.c"
	.text
	.p2align 4
	.type	sha256_transform, @function
sha256_transform:
.LFB11:
	.cfi_startproc
	pushq	%rbp
	.cfi_def_cfa_offset 16
	.cfi_offset 6, -16
	movq	%rsp, %rbp
	.cfi_def_cfa_register 6
	pushq	%r15
	pushq	%r14
	pushq	%r13
	pushq	%r12
	pushq	%rbx
	andq	$-32, %rsp
	subq	$168, %rsp
	.cfi_offset 15, -24
	.cfi_offset 14, -32
	.cfi_offset 13, -40
	.cfi_offset 12, -48
	.cfi_offset 3, -56
	movq	%rdi, -96(%rsp)
	leaq	-88(%rsp), %r13
	leaq	104(%rsp), %rdx
	movq	%r13, %rax
	vmovdqu	(%rsi), %ymm0
	vmovdqa	%ymm0, -88(%rsp)
	vmovdqu	32(%rsi), %ymm0
	vmovdqa	%ymm0, -56(%rsp)
	vmovq	-32(%rsp), %xmm0
	.p2align 4
	.p2align 3
.L2:
	vmovq	4(%rax), %xmm2
	addq	$8, %rax
	vpsrld	$18, %xmm2, %xmm3
	vpslld	$14, %xmm2, %xmm1
	vpsrld	$7, %xmm2, %xmm4
	vpor	%xmm3, %xmm1, %xmm1
	vpslld	$25, %xmm2, %xmm3
	vpsrld	$3, %xmm2, %xmm2
	vpor	%xmm4, %xmm3, %xmm3
	vpsrld	$17, %xmm0, %xmm4
	vpxor	%xmm3, %xmm1, %xmm1
	vpsrld	$19, %xmm0, %xmm3
	vpxor	%xmm2, %xmm1, %xmm1
	vpslld	$13, %xmm0, %xmm2
	vpor	%xmm3, %xmm2, %xmm2
	vpslld	$15, %xmm0, %xmm3
	vpor	%xmm4, %xmm3, %xmm3
	vpsrld	$10, %xmm0, %xmm0
	vpxor	%xmm3, %xmm2, %xmm2
	vpxor	%xmm0, %xmm2, %xmm0
	vmovq	-8(%rax), %xmm2
	vpaddd	%xmm0, %xmm1, %xmm1
	vmovq	28(%rax), %xmm0
	vpaddd	%xmm2, %xmm0, %xmm0
	vpaddd	%xmm0, %xmm1, %xmm0
	vmovq	%xmm0, 56(%rax)
	cmpq	%rax, %rdx
	jne	.L2
	movq	-96(%rsp), %rax
	xorl	%r8d, %r8d
	leaq	K(%rip), %r14
	vmovdqu	(%rax), %ymm3
	movl	(%rax), %esi
	movl	4(%rax), %r9d
	movl	8(%rax), %r10d
	vextracti128	$0x1, %ymm3, %xmm0
	movl	16(%rax), %ecx
	movl	20(%rax), %r11d
	vpextrd	$3, %xmm3, %r12d
	movl	24(%rax), %ebx
	vpextrd	$3, %xmm0, %edi
	jmp	.L3
	.p2align 4,,10
	.p2align 3
.L4:
	movl	%r11d, %ebx
	movl	%r9d, %r10d
	movl	%ecx, %r11d
	movl	%esi, %r9d
	movl	%r15d, %ecx
	movl	%edx, %esi
.L3:
	rorx	$11, %ecx, %edx
	movl	%ecx, %r15d
	rorx	$6, %ecx, %eax
	xorl	%edx, %eax
	andl	%r11d, %r15d
	rorx	$25, %ecx, %edx
	xorl	%edx, %eax
	movl	0(%r13,%r8), %edx
	addl	(%r14,%r8), %edx
	addq	$4, %r8
	addl	%edx, %eax
	andn	%ebx, %ecx, %edx
	xorl	%r15d, %edx
	movl	%r9d, %r15d
	addl	%edx, %eax
	rorx	$2, %esi, %edx
	andl	%r10d, %r15d
	addl	%edi, %eax
	rorx	$13, %esi, %edi
	xorl	%edi, %edx
	rorx	$22, %esi, %edi
	xorl	%edi, %edx
	movl	%r9d, %edi
	xorl	%r10d, %edi
	andl	%esi, %edi
	xorl	%r15d, %edi
	leal	(%rax,%r12), %r15d
	movl	%r10d, %r12d
	addl	%edi, %edx
	movl	%ebx, %edi
	addl	%eax, %edx
	cmpq	$256, %r8
	jne	.L4
	vmovd	%r15d, %xmm6
	vmovd	%r11d, %xmm5
	vmovd	%r9d, %xmm7
	movq	-96(%rsp), %rax
	vpinsrd	$1, %ebx, %xmm5, %xmm0
	vpinsrd	$1, %ecx, %xmm6, %xmm2
	vmovd	%edx, %xmm5
	vpunpcklqdq	%xmm0, %xmm2, %xmm2
	vpinsrd	$1, %esi, %xmm5, %xmm1
	vpinsrd	$1, %r10d, %xmm7, %xmm0
	vpunpcklqdq	%xmm0, %xmm1, %xmm0
	vinserti128	$0x1, %xmm2, %ymm0, %ymm0
	vpaddd	%ymm3, %ymm0, %ymm0
	vmovdqu	%ymm0, (%rax)
	vzeroupper
	leaq	-40(%rbp), %rsp
	popq	%rbx
	popq	%r12
	popq	%r13
	popq	%r14
	popq	%r15
	popq	%rbp
	.cfi_def_cfa 7, 8
	ret
	.cfi_endproc
.LFE11:
	.size	sha256_transform, .-sha256_transform
	.section	.rodata.str1.1,"aMS",@progbits,1
.LC1:
	.string	"%016llx\n"
	.section	.text.startup,"ax",@progbits
	.p2align 4
	.globl	main
	.type	main, @function
main:
.LFB13:
	.cfi_startproc
	pushq	%rbp
	.cfi_def_cfa_offset 16
	.cfi_offset 6, -16
	vpxor	%xmm0, %xmm0, %xmm0
	movq	%rsp, %rbp
	.cfi_def_cfa_register 6
	pushq	%r15
	pushq	%r14
	pushq	%r13
	pushq	%r12
	pushq	%rbx
	andq	$-32, %rsp
	addq	$-128, %rsp
	.cfi_offset 15, -24
	.cfi_offset 14, -32
	.cfi_offset 13, -40
	.cfi_offset 12, -48
	.cfi_offset 3, -56
	vmovdqu	%ymm0, 68(%rsp)
	leaq	64(%rsp), %r13
	leaq	32(%rsp), %r12
	vmovdqu	%ymm0, 92(%rsp)
	vmovdqa	.LC0(%rip), %ymm0
	movq	%r13, %rsi
	movq	%r12, %rdi
	movl	$1633837952, 64(%rsp)
	movl	$24, 124(%rsp)
	vmovdqa	%ymm0, 32(%rsp)
	vzeroupper
	call	sha256_transform
	cmpl	$-1166534977, 32(%rsp)
	jne	.L9
	cmpl	$-1895706646, 36(%rsp)
	je	.L18
.L9:
	leaq	-40(%rbp), %rsp
	movl	$2, %eax
	popq	%rbx
	popq	%r12
	popq	%r13
	popq	%r14
	popq	%r15
	popq	%rbp
	.cfi_remember_state
	.cfi_def_cfa 7, 8
	ret
.L18:
	.cfi_restore_state
	cmpl	$-234875475, 60(%rsp)
	jne	.L9
	xorl	%eax, %eax
	xorl	%ebx, %ebx
	leaq	68(%rsp), %r14
	movl	%eax, 28(%rsp)
.L10:
	movl	28(%rsp), %edx
	movl	$-1150833019, 36(%rsp)
	xorl	%r15d, %r15d
	movabsq	$-6534734903820487822, %rax
	movq	%rax, 40(%rsp)
	movabsq	$-7276294671082564993, %rax
	movq	%rax, 48(%rsp)
	xorl	$1779033703, %edx
	movabsq	$6620516960021240235, %rax
	movl	%edx, 32(%rsp)
	movq	%rax, 56(%rsp)
	.p2align 4
	.p2align 3
.L12:
	xorl	%r15d, %edx
	leal	1013904223(%r15), %ecx
	movl	$1, %eax
	movl	%edx, 64(%rsp)
	movq	%r14, %rdx
	.p2align 5
	.p2align 4
	.p2align 3
.L13:
	movl	%eax, %esi
	addl	$1, %eax
	addq	$4, %rdx
	andl	$7, %esi
	movl	32(%rsp,%rsi,4), %edi
	xorl	%ecx, %edi
	addl	$1013904223, %ecx
	movl	%edi, -4(%rdx)
	cmpl	$16, %eax
	jne	.L13
	movq	%r13, %rsi
	movq	%r12, %rdi
	addl	$1664525, %r15d
	call	sha256_transform
	movl	32(%rsp), %edx
	movl	44(%rsp), %eax
	movq	%rdx, %rcx
	salq	$16, %rax
	salq	$32, %rcx
	xorq	%rcx, %rax
	movl	60(%rsp), %ecx
	xorq	%rcx, %rax
	addq	%rax, %rbx
	cmpl	$-870711296, %r15d
	jne	.L12
	addl	$1, 28(%rsp)
	movl	28(%rsp), %eax
	cmpl	$8, %eax
	jne	.L10
	movq	%rbx, %rsi
	leaq	.LC1(%rip), %rdi
	xorl	%eax, %eax
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
.LFE13:
	.size	main, .-main
	.section	.rodata
	.align 32
	.type	K, @object
	.size	K, 256
K:
	.long	1116352408
	.long	1899447441
	.long	-1245643825
	.long	-373957723
	.long	961987163
	.long	1508970993
	.long	-1841331548
	.long	-1424204075
	.long	-670586216
	.long	310598401
	.long	607225278
	.long	1426881987
	.long	1925078388
	.long	-2132889090
	.long	-1680079193
	.long	-1046744716
	.long	-459576895
	.long	-272742522
	.long	264347078
	.long	604807628
	.long	770255983
	.long	1249150122
	.long	1555081692
	.long	1996064986
	.long	-1740746414
	.long	-1473132947
	.long	-1341970488
	.long	-1084653625
	.long	-958395405
	.long	-710438585
	.long	113926993
	.long	338241895
	.long	666307205
	.long	773529912
	.long	1294757372
	.long	1396182291
	.long	1695183700
	.long	1986661051
	.long	-2117940946
	.long	-1838011259
	.long	-1564481375
	.long	-1474664885
	.long	-1035236496
	.long	-949202525
	.long	-778901479
	.long	-694614492
	.long	-200395387
	.long	275423344
	.long	430227734
	.long	506948616
	.long	659060556
	.long	883997877
	.long	958139571
	.long	1322822218
	.long	1537002063
	.long	1747873779
	.long	1955562222
	.long	2024104815
	.long	-2067236844
	.long	-1933114872
	.long	-1866530822
	.long	-1538233109
	.long	-1090935817
	.long	-965641998
	.section	.rodata.cst32,"aM",@progbits,32
	.align 32
.LC0:
	.long	1779033703
	.long	-1150833019
	.long	1013904242
	.long	-1521486534
	.long	1359893119
	.long	-1694144372
	.long	528734635
	.long	1541459225
	.ident	"GCC: (Debian 14.2.0-19) 14.2.0"
	.section	.note.GNU-stack,"",@progbits
