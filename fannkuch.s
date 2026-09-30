	.file	"fannkuch.c"
	.text
	.section	.rodata.str1.1,"aMS",@progbits,1
.LC3:
	.string	"%d\nPfannkuchen(%d) = %d\n"
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
	xorl	%edi, %edi
	xorl	%esi, %esi
	movl	$11, %edx
	leaq	perm1(%rip), %r8
	movq	%rsp, %rbp
	.cfi_def_cfa_register 6
	pushq	%r15
	pushq	%r14
	.cfi_offset 15, -24
	.cfi_offset 14, -32
	xorl	%r14d, %r14d
	pushq	%r13
	.cfi_offset 13, -40
	leaq	count(%rip), %r13
	pushq	%r12
	pushq	%rbx
	.cfi_offset 12, -48
	.cfi_offset 3, -56
	leaq	perm(%rip), %rbx
	andq	$-32, %rsp
	subq	$32, %rsp
	movq	.LC1(%rip), %rax
	vmovdqa	.LC0(%rip), %ymm0
	movl	$0, maxflips(%rip)
	movb	$0, 19(%rsp)
	vmovdqa	.LC2(%rip), %ymm1
	movl	$0, checksum(%rip)
	movq	%rax, 32+perm1(%rip)
	movl	$10, 40+perm1(%rip)
	vmovdqa	%ymm0, perm1(%rip)
.L2:
	cmpl	$1, %edx
	jle	.L84
.L3:
	leal	-1(%rdx), %eax
	movslq	%eax, %rcx
	movl	%edx, 0(%r13,%rcx,4)
	movl	%eax, %edx
	cmpl	$1, %edx
	jg	.L3
.L84:
	movq	32+perm1(%rip), %rax
	vmovdqa	(%r8), %ymm0
	movq	%rax, 32+perm(%rip)
	movl	40+perm1(%rip), %eax
	vmovd	%xmm0, %r12d
	vmovdqa	%ymm0, (%rbx)
	movl	%eax, 40+perm(%rip)
	testl	%r12d, %r12d
	je	.L26
	movl	%r14d, 28(%rsp)
	movl	%r12d, %ecx
	xorl	%r11d, %r11d
	movl	%edx, 24(%rsp)
	movl	%esi, 20(%rsp)
	.p2align 4
	.p2align 3
.L15:
	leal	1(%rcx), %eax
	sarl	%eax
	testl	%eax, %eax
	jle	.L6
	leal	-1(%rax), %r14d
	movl	%eax, %r10d
	cmpl	$2, %r14d
	jbe	.L7
	movslq	%ecx, %r15
	movslq	%eax, %rsi
	leaq	0(,%r15,4), %rdx
	salq	$2, %rsi
	leaq	4(%rdx), %r9
	subq	%rsi, %r9
	cmpq	%rsi, %r9
	jl	.L8
	cmpl	$6, %r14d
	jbe	.L27
	vmovdqa	(%rbx), %ymm0
	leaq	-28(%rbx,%rdx), %rsi
	movl	%eax, %r9d
	vpermd	(%rsi), %ymm1, %ymm2
	shrl	$3, %r9d
	vpermd	%ymm0, %ymm1, %ymm0
	vmovdqa	%ymm2, (%rbx)
	vmovdqu	%ymm0, (%rsi)
	cmpl	$1, %r9d
	je	.L10
	vmovdqa	32+perm(%rip), %ymm0
	vpermd	-32(%rsi), %ymm1, %ymm2
	vpermd	%ymm0, %ymm1, %ymm0
	vmovdqa	%ymm2, 32+perm(%rip)
	vmovdqu	%ymm0, -32(%rsi)
.L10:
	movl	%eax, %esi
	andl	$-8, %esi
	movl	%esi, %r9d
	cmpl	%esi, %eax
	je	.L14
	movl	%eax, %r10d
	subl	%esi, %r10d
	leal	-1(%r10), %r14d
	cmpl	$2, %r14d
	jbe	.L12
.L9:
	salq	$2, %r9
	subq	$12, %rdx
	leaq	(%rbx,%r9), %r14
	subq	%r9, %rdx
	vmovdqa	(%r14), %xmm0
	vpshufd	$27, (%rbx,%rdx), %xmm2
	vmovdqa	%xmm2, (%r14)
	vpshufd	$27, %xmm0, %xmm0
	vmovdqu	%xmm0, (%rbx,%rdx)
	movl	%r10d, %edx
	andl	$-4, %edx
	addl	%edx, %esi
	andl	$3, %r10d
	je	.L14
.L12:
	movl	%ecx, %edx
	movslq	%esi, %r9
	subl	%esi, %edx
	movl	(%rbx,%r9,4), %r10d
	movslq	%edx, %rdx
	movl	(%rbx,%rdx,4), %r14d
	movl	%r14d, (%rbx,%r9,4)
	movl	%r10d, (%rbx,%rdx,4)
	leal	1(%rsi), %edx
	cmpl	%edx, %eax
	jle	.L14
	movl	%ecx, %r9d
	movslq	%edx, %r10
	addl	$2, %esi
	subl	%edx, %r9d
	movl	(%rbx,%r10,4), %r14d
	movslq	%r9d, %r9
	movl	(%rbx,%r9,4), %edx
	movl	%edx, (%rbx,%r10,4)
	movl	%r14d, (%rbx,%r9,4)
	cmpl	%esi, %eax
	jle	.L14
	subl	%esi, %ecx
	movslq	%esi, %rax
	movslq	%ecx, %rcx
	movl	(%rbx,%rax,4), %edx
	movl	(%rbx,%rcx,4), %esi
	movl	%esi, (%rbx,%rax,4)
	movl	%edx, (%rbx,%rcx,4)
.L14:
	movl	(%rbx), %ecx
	addl	$1, %r11d
	testl	%ecx, %ecx
	jne	.L15
	movl	28(%rsp), %r14d
	movl	24(%rsp), %edx
	movl	20(%rsp), %esi
.L4:
	cmpl	%r11d, %edi
	jge	.L16
	movb	$1, 19(%rsp)
	movl	%r11d, %edi
.L16:
	movl	%r11d, %eax
	movslq	%edx, %r15
	movl	%esi, 24(%rsp)
	negl	%eax
	testb	$1, %sil
	cmovne	%eax, %r11d
	addl	%r11d, %r14d
	movl	%r14d, 28(%rsp)
	movq	%r15, %r14
	movl	%edi, %r15d
	jmp	.L23
	.p2align 4,,10
	.p2align 3
.L86:
	movl	(%r8), %r12d
.L23:
	testl	%r14d, %r14d
	jle	.L21
	movl	%r14d, %edx
	movq	%r8, %rdi
	leaq	4+perm1(%rip), %rsi
	salq	$2, %rdx
	vzeroupper
	call	memmove@PLT
	vmovdqa	.LC2(%rip), %ymm1
	movq	%rax, %r8
.L21:
	movl	0(%r13,%r14,4), %eax
	movl	%r12d, (%r8,%r14,4)
	subl	$1, %eax
	movl	%eax, 0(%r13,%r14,4)
	testl	%eax, %eax
	jg	.L85
	addq	$1, %r14
	cmpl	$11, %r14d
	jne	.L86
	movl	28(%rsp), %r14d
	cmpb	$0, 19(%rsp)
	movl	%r15d, %edi
	movl	%r14d, checksum(%rip)
	je	.L29
	movl	%r15d, maxflips(%rip)
.L24:
	movl	%edi, %ecx
	movl	%r14d, %esi
	xorl	%eax, %eax
	movl	$11, %edx
	leaq	.LC3(%rip), %rdi
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
	.cfi_remember_state
	.cfi_def_cfa 7, 8
	ret
.L6:
	.cfi_restore_state
	jmp	.L6
	.p2align 4,,10
	.p2align 3
.L7:
	movslq	%ecx, %rdx
	movl	(%rbx), %esi
	movl	(%rbx,%rdx,4), %r9d
	movl	%r9d, (%rbx)
	movl	%esi, (%rbx,%rdx,4)
	cmpl	$1, %eax
	je	.L14
	leal	-1(%rcx), %edx
	movl	4+perm(%rip), %esi
	movslq	%edx, %rdx
	movl	(%rbx,%rdx,4), %r9d
	movl	%esi, (%rbx,%rdx,4)
	movl	%r9d, 4+perm(%rip)
	cmpl	$3, %eax
	jne	.L14
.L25:
	leal	-2(%rcx), %edx
	movl	8+perm(%rip), %esi
	movslq	%edx, %rdx
	movl	(%rbx,%rdx,4), %r9d
	movl	%esi, (%rbx,%rdx,4)
	movl	%r9d, 8+perm(%rip)
	cmpl	$3, %eax
	je	.L14
	leal	-3(%rcx), %edx
	movl	12+perm(%rip), %esi
	movslq	%edx, %rdx
	movl	(%rbx,%rdx,4), %r9d
	movl	%esi, (%rbx,%rdx,4)
	movl	%r9d, 12+perm(%rip)
	cmpl	$4, %eax
	je	.L14
	leal	-4(%rcx), %edx
	movl	16+perm(%rip), %esi
	movslq	%edx, %rdx
	movl	(%rbx,%rdx,4), %r9d
	movl	%esi, (%rbx,%rdx,4)
	movl	%r9d, 16+perm(%rip)
	cmpl	$5, %eax
	je	.L14
	leal	-5(%rcx), %edx
	movl	20+perm(%rip), %esi
	movslq	%edx, %rdx
	movl	(%rbx,%rdx,4), %r9d
	movl	%esi, (%rbx,%rdx,4)
	movl	%r9d, 20+perm(%rip)
	cmpl	$6, %eax
	je	.L14
	leal	-6(%rcx), %edx
	movl	24+perm(%rip), %esi
	movslq	%edx, %rdx
	movl	(%rbx,%rdx,4), %r9d
	movl	%esi, (%rbx,%rdx,4)
	movl	%r9d, 24+perm(%rip)
	cmpl	$7, %eax
	je	.L14
	leal	-7(%rcx), %edx
	movl	28+perm(%rip), %esi
	movslq	%edx, %rdx
	movl	(%rbx,%rdx,4), %r9d
	movl	%esi, (%rbx,%rdx,4)
	movl	%r9d, 28+perm(%rip)
	cmpl	$8, %eax
	je	.L14
	leal	-8(%rcx), %edx
	movl	32+perm(%rip), %esi
	movslq	%edx, %rdx
	movl	(%rbx,%rdx,4), %r9d
	movl	%esi, (%rbx,%rdx,4)
	movl	%r9d, 32+perm(%rip)
	cmpl	$9, %eax
	je	.L14
	leal	-9(%rcx), %edx
	movl	36+perm(%rip), %esi
	movslq	%edx, %rdx
	movl	(%rbx,%rdx,4), %r9d
	movl	%esi, (%rbx,%rdx,4)
	movl	%r9d, 36+perm(%rip)
	cmpl	$10, %eax
	je	.L14
	leal	-10(%rcx), %edx
	movl	40+perm(%rip), %esi
	movslq	%edx, %rdx
	movl	(%rbx,%rdx,4), %r9d
	movl	%esi, (%rbx,%rdx,4)
	movl	%r9d, 40+perm(%rip)
	cmpl	$11, %eax
	je	.L14
	leal	-11(%rcx), %edx
	movl	44+perm(%rip), %esi
	movslq	%edx, %rdx
	movl	(%rbx,%rdx,4), %r9d
	movl	%esi, (%rbx,%rdx,4)
	movl	%r9d, 44+perm(%rip)
	cmpl	$12, %eax
	je	.L14
	leal	-12(%rcx), %edx
	movl	48+perm(%rip), %esi
	movslq	%edx, %rdx
	movl	(%rbx,%rdx,4), %r9d
	movl	%esi, (%rbx,%rdx,4)
	movl	%r9d, 48+perm(%rip)
	cmpl	$13, %eax
	je	.L14
	leal	-13(%rcx), %edx
	movl	52+perm(%rip), %esi
	movslq	%edx, %rdx
	movl	(%rbx,%rdx,4), %r9d
	movl	%esi, (%rbx,%rdx,4)
	movl	%r9d, 52+perm(%rip)
	cmpl	$14, %eax
	je	.L14
	leal	-14(%rcx), %esi
	movl	56+perm(%rip), %edx
	movslq	%esi, %rsi
	movl	(%rbx,%rsi,4), %r9d
	movl	%edx, (%rbx,%rsi,4)
	movl	%r9d, 56+perm(%rip)
	cmpl	$15, %eax
	je	.L14
	leal	-15(%rcx), %eax
	cltq
	movl	(%rbx,%rax,4), %ecx
	movl	%ecx, 60+perm(%rip)
	movl	%edx, (%rbx,%rax,4)
	jmp	.L14
.L8:
	movl	(%rbx,%r15,4), %esi
	movl	(%rbx), %edx
	movl	%esi, (%rbx)
	movl	%edx, (%rbx,%r15,4)
	leal	-1(%rcx), %edx
	movl	4+perm(%rip), %esi
	movslq	%edx, %rdx
	movl	(%rbx,%rdx,4), %r9d
	movl	%esi, (%rbx,%rdx,4)
	movl	%r9d, 4+perm(%rip)
	jmp	.L25
.L27:
	xorl	%r9d, %r9d
	xorl	%esi, %esi
	jmp	.L9
.L85:
	movl	24(%rsp), %esi
	movl	%r15d, %edi
	movq	%r14, %r15
	movl	28(%rsp), %r14d
	movl	%r15d, %edx
	addl	$1, %esi
	jmp	.L2
.L29:
	xorl	%edi, %edi
	jmp	.L24
.L26:
	xorl	%r11d, %r11d
	jmp	.L4
	.cfi_endproc
.LFE12:
	.size	main, .-main
	.local	checksum
	.comm	checksum,4,4
	.local	maxflips
	.comm	maxflips,4,4
	.local	count
	.comm	count,64,32
	.local	perm1
	.comm	perm1,64,32
	.local	perm
	.comm	perm,64,32
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
.LC1:
	.long	8
	.long	9
	.section	.rodata.cst32
	.align 32
.LC2:
	.long	7
	.long	6
	.long	5
	.long	4
	.long	3
	.long	2
	.long	1
	.long	0
	.ident	"GCC: (Debian 14.2.0-19) 14.2.0"
	.section	.note.GNU-stack,"",@progbits
