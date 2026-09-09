	.file	"sha256_transform.c"
	.text
	.p2align 4
	.type	sha256_transform, @function
sha256_transform:
.LFB11:
	.cfi_startproc
	pushq	%r15
	.cfi_def_cfa_offset 16
	.cfi_offset 15, -16
	pushq	%r14
	.cfi_def_cfa_offset 24
	.cfi_offset 14, -24
	pushq	%r13
	.cfi_def_cfa_offset 32
	.cfi_offset 13, -32
	movq	%rdi, %r13
	pushq	%r12
	.cfi_def_cfa_offset 40
	.cfi_offset 12, -40
	pushq	%rbp
	.cfi_def_cfa_offset 48
	.cfi_offset 6, -48
	pushq	%rbx
	.cfi_def_cfa_offset 56
	.cfi_offset 3, -56
	subq	$144, %rsp
	.cfi_def_cfa_offset 200
	movdqu	(%rsi), %xmm0
	leaq	-56(%rsp), %rax
	leaq	136(%rsp), %rdx
	movaps	%xmm0, -120(%rsp)
	movdqu	16(%rsi), %xmm0
	movaps	%xmm0, -104(%rsp)
	movdqu	32(%rsi), %xmm0
	movaps	%xmm0, -88(%rsp)
	movdqu	48(%rsi), %xmm0
	movaps	%xmm0, -72(%rsp)
	movq	-64(%rsp), %xmm0
	.p2align 4
	.p2align 3
.L2:
	movq	-60(%rax), %xmm2
	addq	$8, %rax
	movdqa	%xmm2, %xmm1
	movdqa	%xmm2, %xmm3
	movdqa	%xmm2, %xmm4
	psrld	$18, %xmm3
	pslld	$14, %xmm1
	por	%xmm3, %xmm1
	psrld	$7, %xmm4
	movdqa	%xmm2, %xmm3
	pslld	$25, %xmm3
	psrld	$3, %xmm2
	por	%xmm4, %xmm3
	movdqa	%xmm0, %xmm4
	pxor	%xmm3, %xmm1
	psrld	$17, %xmm4
	movdqa	%xmm0, %xmm3
	pxor	%xmm2, %xmm1
	psrld	$19, %xmm3
	movdqa	%xmm0, %xmm2
	pslld	$13, %xmm2
	por	%xmm3, %xmm2
	movdqa	%xmm0, %xmm3
	pslld	$15, %xmm3
	psrld	$10, %xmm0
	por	%xmm4, %xmm3
	pxor	%xmm3, %xmm2
	pxor	%xmm2, %xmm0
	movq	-72(%rax), %xmm2
	paddd	%xmm0, %xmm1
	movq	-36(%rax), %xmm0
	paddd	%xmm2, %xmm0
	paddd	%xmm1, %xmm0
	movq	%xmm0, -8(%rax)
	cmpq	%rax, %rdx
	jne	.L2
	movl	12(%r13), %ebp
	movl	28(%r13), %edi
	xorl	%r8d, %r8d
	leaq	-120(%rsp), %r12
	movl	0(%r13), %esi
	movl	4(%r13), %r9d
	leaq	K(%rip), %r14
	movdqu	0(%r13), %xmm2
	movdqu	16(%r13), %xmm1
	movl	8(%r13), %r10d
	movl	16(%r13), %ecx
	movl	20(%r13), %r11d
	movl	24(%r13), %ebx
	jmp	.L3
	.p2align 4,,10
	.p2align 3
.L4:
	movl	%r11d, %ebx
	movl	%r9d, %r10d
	movl	%ecx, %r11d
	movl	%esi, %r9d
	movl	%r15d, %ecx
	movl	%eax, %esi
.L3:
	movl	%ecx, %eax
	movl	%ecx, %edx
	movl	%ecx, %r15d
	rorl	$11, %edx
	rorl	$6, %eax
	andl	%r11d, %r15d
	xorl	%edx, %eax
	movl	%ecx, %edx
	roll	$7, %edx
	xorl	%edx, %eax
	movl	(%r12,%r8), %edx
	addl	(%r14,%r8), %edx
	addq	$4, %r8
	addl	%edx, %eax
	movl	%ecx, %edx
	notl	%edx
	andl	%ebx, %edx
	xorl	%r15d, %edx
	movl	%r9d, %r15d
	addl	%edx, %eax
	movl	%esi, %edx
	andl	%r10d, %r15d
	addl	%edi, %eax
	movl	%esi, %edi
	rorl	$2, %edx
	rorl	$13, %edi
	xorl	%edi, %edx
	movl	%esi, %edi
	roll	$10, %edi
	xorl	%edi, %edx
	movl	%r9d, %edi
	xorl	%r10d, %edi
	andl	%esi, %edi
	xorl	%r15d, %edi
	leal	(%rax,%rbp), %r15d
	movl	%r10d, %ebp
	addl	%edi, %edx
	movl	%ebx, %edi
	addl	%edx, %eax
	cmpq	$256, %r8
	jne	.L4
	movd	%r10d, %xmm5
	movd	%r9d, %xmm3
	movd	%eax, %xmm0
	movd	%esi, %xmm6
	punpckldq	%xmm5, %xmm3
	movd	%ebx, %xmm7
	punpckldq	%xmm6, %xmm0
	movd	%ecx, %xmm5
	punpcklqdq	%xmm3, %xmm0
	paddd	%xmm2, %xmm0
	movd	%r11d, %xmm2
	movups	%xmm0, 0(%r13)
	movd	%r15d, %xmm0
	punpckldq	%xmm7, %xmm2
	punpckldq	%xmm5, %xmm0
	punpcklqdq	%xmm2, %xmm0
	paddd	%xmm1, %xmm0
	movups	%xmm0, 16(%r13)
	addq	$144, %rsp
	.cfi_def_cfa_offset 56
	popq	%rbx
	.cfi_def_cfa_offset 48
	popq	%rbp
	.cfi_def_cfa_offset 40
	popq	%r12
	.cfi_def_cfa_offset 32
	popq	%r13
	.cfi_def_cfa_offset 24
	popq	%r14
	.cfi_def_cfa_offset 16
	popq	%r15
	.cfi_def_cfa_offset 8
	ret
	.cfi_endproc
.LFE11:
	.size	sha256_transform, .-sha256_transform
	.section	.rodata.str1.1,"aMS",@progbits,1
.LC2:
	.string	"%016llx\n"
	.section	.text.startup,"ax",@progbits
	.p2align 4
	.globl	main
	.type	main, @function
main:
.LFB13:
	.cfi_startproc
	pushq	%r15
	.cfi_def_cfa_offset 16
	.cfi_offset 15, -16
	pxor	%xmm0, %xmm0
	pushq	%r14
	.cfi_def_cfa_offset 24
	.cfi_offset 14, -24
	pushq	%r13
	.cfi_def_cfa_offset 32
	.cfi_offset 13, -32
	pushq	%r12
	.cfi_def_cfa_offset 40
	.cfi_offset 12, -40
	pushq	%rbp
	.cfi_def_cfa_offset 48
	.cfi_offset 6, -48
	pushq	%rbx
	.cfi_def_cfa_offset 56
	.cfi_offset 3, -56
	subq	$104, %rsp
	.cfi_def_cfa_offset 160
	movups	%xmm0, 36(%rsp)
	leaq	32(%rsp), %r12
	movq	%rsp, %rdi
	movups	%xmm0, 52(%rsp)
	movq	%r12, %rsi
	movups	%xmm0, 68(%rsp)
	movdqa	.LC0(%rip), %xmm0
	movq	$0, 84(%rsp)
	movaps	%xmm0, (%rsp)
	movdqa	.LC1(%rip), %xmm0
	movl	$1633837952, 32(%rsp)
	movl	$24, 92(%rsp)
	movaps	%xmm0, 16(%rsp)
	call	sha256_transform
	cmpl	$-1166534977, (%rsp)
	jne	.L9
	cmpl	$-1895706646, 4(%rsp)
	je	.L18
.L9:
	addq	$104, %rsp
	.cfi_remember_state
	.cfi_def_cfa_offset 56
	movl	$2, %eax
	popq	%rbx
	.cfi_def_cfa_offset 48
	popq	%rbp
	.cfi_def_cfa_offset 40
	popq	%r12
	.cfi_def_cfa_offset 32
	popq	%r13
	.cfi_def_cfa_offset 24
	popq	%r14
	.cfi_def_cfa_offset 16
	popq	%r15
	.cfi_def_cfa_offset 8
	ret
.L18:
	.cfi_restore_state
	cmpl	$-234875475, 28(%rsp)
	jne	.L9
	movq	%rsp, %rbp
	xorl	%r14d, %r14d
	leaq	36(%rsp), %r13
	xorl	%ebx, %ebx
.L10:
	movabsq	$-6534734903820487822, %rax
	movl	%r14d, %edx
	movl	$-1150833019, 4(%rsp)
	xorl	%r15d, %r15d
	movq	%rax, 8(%rsp)
	xorl	$1779033703, %edx
	movabsq	$-7276294671082564993, %rax
	movq	%rax, 16(%rsp)
	movabsq	$6620516960021240235, %rax
	movl	%edx, (%rsp)
	movq	%rax, 24(%rsp)
	.p2align 4
	.p2align 3
.L12:
	xorl	%r15d, %edx
	leal	1013904223(%r15), %ecx
	movl	$1, %eax
	movl	%edx, 32(%rsp)
	movq	%r13, %rdx
	.p2align 5
	.p2align 4
	.p2align 3
.L13:
	movl	%eax, %esi
	addl	$1, %eax
	addq	$4, %rdx
	andl	$7, %esi
	movl	(%rsp,%rsi,4), %edi
	xorl	%ecx, %edi
	addl	$1013904223, %ecx
	movl	%edi, -4(%rdx)
	cmpl	$16, %eax
	jne	.L13
	movq	%r12, %rsi
	movq	%rbp, %rdi
	addl	$1664525, %r15d
	call	sha256_transform
	movl	(%rsp), %edx
	movl	12(%rsp), %eax
	movq	%rdx, %rcx
	salq	$16, %rax
	salq	$32, %rcx
	xorq	%rcx, %rax
	movl	28(%rsp), %ecx
	xorq	%rcx, %rax
	addq	%rax, %rbx
	cmpl	$-870711296, %r15d
	jne	.L12
	addl	$1, %r14d
	cmpl	$8, %r14d
	jne	.L10
	movq	%rbx, %rsi
	leaq	.LC2(%rip), %rdi
	xorl	%eax, %eax
	call	printf@PLT
	addq	$104, %rsp
	.cfi_def_cfa_offset 56
	xorl	%eax, %eax
	popq	%rbx
	.cfi_def_cfa_offset 48
	popq	%rbp
	.cfi_def_cfa_offset 40
	popq	%r12
	.cfi_def_cfa_offset 32
	popq	%r13
	.cfi_def_cfa_offset 24
	popq	%r14
	.cfi_def_cfa_offset 16
	popq	%r15
	.cfi_def_cfa_offset 8
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
	.section	.rodata.cst16,"aM",@progbits,16
	.align 16
.LC0:
	.long	1779033703
	.long	-1150833019
	.long	1013904242
	.long	-1521486534
	.align 16
.LC1:
	.long	1359893119
	.long	-1694144372
	.long	528734635
	.long	1541459225
	.ident	"GCC: (Debian 14.2.0-19) 14.2.0"
	.section	.note.GNU-stack,"",@progbits
