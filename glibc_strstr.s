	.file	"glibc_strstr.c"
	.text
	.p2align 4
	.type	two_way_short_needle.constprop.0, @function
two_way_short_needle.constprop.0:
.LFB14:
	.cfi_startproc
	pushq	%rbp
	.cfi_def_cfa_offset 16
	.cfi_offset 6, -16
	movq	%rdi, %rcx
	vmovd	%esi, %xmm0
	movl	%esi, %edx
	vpbroadcastb	%xmm0, %ymm0
	leal	-1(%rsi), %edi
	subl	$1, %esi
	movq	%rsp, %rbp
	.cfi_def_cfa_register 6
	pushq	%r15
	pushq	%r14
	pushq	%r13
	pushq	%r12
	pushq	%rbx
	subq	$144, %rsp
	.cfi_offset 15, -24
	.cfi_offset 14, -32
	.cfi_offset 13, -40
	.cfi_offset 12, -48
	.cfi_offset 3, -56
	vmovdqu	%ymm0, -120(%rsp)
	vmovdqu	%ymm0, -88(%rsp)
	movzbl	(%rcx), %eax
	vmovdqu	%ymm0, -56(%rsp)
	vmovdqu	%ymm0, -24(%rsp)
	vmovdqu	%ymm0, 8(%rsp)
	movq	%rax, %r15
	vmovdqu	%ymm0, 40(%rsp)
	vmovdqu	%ymm0, 72(%rsp)
	vmovdqu	%ymm0, 104(%rsp)
	movb	%sil, -120(%rsp,%rax)
	movzbl	1(%rcx), %eax
	leal	-2(%rdx), %esi
	movb	%sil, -120(%rsp,%rax)
	movzbl	2(%rcx), %eax
	leal	-3(%rdx), %esi
	movb	%sil, -120(%rsp,%rax)
	cmpl	$4, %edx
	je	.L2
	movzbl	3(%rcx), %eax
	leal	-4(%rdx), %esi
	movb	%sil, -120(%rsp,%rax)
	cmpl	$4, %edi
	je	.L2
	movzbl	4(%rcx), %eax
	leal	-5(%rdx), %esi
	movb	%sil, -120(%rsp,%rax)
	cmpl	$5, %edi
	je	.L2
	movzbl	5(%rcx), %eax
	leal	-6(%rdx), %esi
	movb	%sil, -120(%rsp,%rax)
	cmpl	$6, %edi
	je	.L2
	movzbl	6(%rcx), %eax
	leal	-7(%rdx), %esi
	movb	%sil, -120(%rsp,%rax)
	cmpl	$7, %edi
	je	.L2
	movzbl	7(%rcx), %eax
	leal	-8(%rdx), %esi
	movb	%sil, -120(%rsp,%rax)
	cmpl	$8, %edi
	je	.L2
	movzbl	8(%rcx), %eax
	leal	-9(%rdx), %esi
	movb	%sil, -120(%rsp,%rax)
	cmpl	$10, %edi
	jne	.L2
	movzbl	9(%rcx), %eax
	movb	$1, -120(%rsp,%rax)
.L2:
	movslq	%edi, %r8
	leaq	haystack(%rip), %rsi
	leal	-2(%rdx), %r10d
	xorl	%eax, %eax
	movzbl	(%rcx,%r8), %r14d
	leal	-3(%rdx), %ebx
	leal	-4(%rdx), %r12d
	leal	-5(%rdx), %r9d
	leal	-6(%rdx), %r11d
	cmpb	(%rsi,%r8), %r14b
	jne	.L4
.L3:
	movl	%r10d, %r13d
	leal	(%rax,%r10), %r8d
	movzbl	(%rcx,%r13), %r13d
	cmpb	%r13b, (%rsi,%r8)
	jne	.L4
	movl	%ebx, %r13d
	leal	(%rbx,%rax), %r8d
	movzbl	(%rcx,%r13), %r13d
	cmpb	%r13b, (%rsi,%r8)
	jne	.L4
	movl	%r12d, %r13d
	leal	(%r12,%rax), %r8d
	movzbl	(%rcx,%r13), %r13d
	cmpb	%r13b, (%rsi,%r8)
	jne	.L4
	cmpl	$-1, %r9d
	je	.L1
	movl	%r9d, %r13d
	leal	(%r9,%rax), %r8d
	movzbl	(%rcx,%r13), %r13d
	cmpb	%r13b, (%rsi,%r8)
	jne	.L4
	cmpl	$-1, %r11d
	je	.L1
	leal	(%r11,%rax), %r13d
	movl	%r11d, %r8d
	movzbl	(%rsi,%r13), %r13d
	cmpb	%r13b, (%rcx,%r8)
	jne	.L4
	leal	-7(%rdx), %r8d
	cmpl	$6, %edx
	je	.L1
	movslq	%r8d, %r13
	addl	%eax, %r8d
	movl	%r8d, %r8d
	movzbl	(%rsi,%r8), %r8d
	cmpb	%r8b, (%rcx,%r13)
	jne	.L4
	leal	-8(%rdx), %r8d
	cmpl	$7, %edx
	je	.L1
	movslq	%r8d, %r13
	addl	%eax, %r8d
	movl	%r8d, %r8d
	movzbl	(%rsi,%r8), %r8d
	cmpb	%r8b, (%rcx,%r13)
	jne	.L4
	leal	-9(%rdx), %r8d
	cmpl	$8, %edx
	je	.L1
	movslq	%r8d, %r13
	addl	%eax, %r8d
	movl	%r8d, %r8d
	movzbl	(%rsi,%r8), %r8d
	cmpb	%r8b, (%rcx,%r13)
	jne	.L4
	leal	-10(%rdx), %r8d
	cmpl	$9, %edx
	je	.L1
	movslq	%r8d, %r13
	addl	%eax, %r8d
	movl	%r8d, %r8d
	movzbl	(%rsi,%r8), %r8d
	cmpb	%r8b, (%rcx,%r13)
	jne	.L4
	cmpl	$10, %edx
	je	.L1
	movl	%eax, %r8d
	cmpb	%r15b, (%rsi,%r8)
	jne	.L4
.L1:
	vzeroupper
	addq	$144, %rsp
	popq	%rbx
	popq	%r12
	popq	%r13
	popq	%r14
	popq	%r15
	popq	%rbp
	.cfi_remember_state
	.cfi_def_cfa 7, 8
	ret
.L4:
	.cfi_restore_state
	movl	$524288, %r8d
	subl	%edx, %r8d
.L5:
	leal	(%rdi,%rax), %r13d
	movzbl	(%rsi,%r13), %r13d
	movzbl	-120(%rsp,%r13), %r13d
	addl	%r13d, %eax
	cmpl	%eax, %r8d
	jb	.L52
	leal	(%rax,%rdi), %r13d
	cmpb	(%rsi,%r13), %r14b
	je	.L3
	jmp	.L5
.L52:
	movl	$-1, %eax
	jmp	.L1
	.cfi_endproc
.LFE14:
	.size	two_way_short_needle.constprop.0, .-two_way_short_needle.constprop.0
	.section	.rodata.str1.1,"aMS",@progbits,1
.LC0:
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
	leaq	haystack(%rip), %rax
	movl	$2084213038, %edx
	leaq	524288(%rax), %rdi
	movq	%rax, %rcx
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
	movq	%rax, 24(%rsp)
	.p2align 6
	.p2align 4
	.p2align 3
.L54:
	imull	$1664525, %edx, %edx
	addq	$1, %rcx
	leal	1013904223(%rdx), %eax
	movq	%rax, %rdx
	imulq	$1321528399, %rax, %rax
	movl	%edx, %esi
	shrq	$35, %rax
	imull	$26, %eax, %eax
	subl	%eax, %esi
	leal	97(%rsi), %eax
	movb	%al, -1(%rcx)
	cmpq	%rcx, %rdi
	jne	.L54
	movb	$0, 524288+haystack(%rip)
	movl	$523124044, %r15d
	xorl	%ebx, %ebx
	leaq	32(%rsp), %r12
	movb	$97, 23(%rsp)
.L55:
	xorl	%r13d, %r13d
	xorl	%r14d, %r14d
	jmp	.L68
	.p2align 4,,10
	.p2align 3
.L102:
	imull	$31337, %r15d, %edi
	movl	%edi, %r9d
	shrl	$5, %r9d
	movq	%r9, %rax
	salq	$14, %rax
	addq	%r9, %rax
	salq	$13, %rax
	addq	%r9, %rax
	movq	%r12, %r9
	shrq	$41, %rax
	imull	$524256, %eax, %eax
	subl	%eax, %edi
	leaq	haystack(%rip), %rax
	addq	%rax, %rdi
	movq	%rdi, %rax
	cmpl	$8, %esi
	jnb	.L99
.L57:
	xorl	%edi, %edi
	testb	$4, %sil
	je	.L60
	movl	(%rax), %edi
	movl	%edi, (%r9)
	movl	$4, %edi
.L60:
	testb	$2, %sil
	je	.L61
	movzwl	(%rax,%rdi), %r10d
	movw	%r10w, (%r9,%rdi)
	addq	$2, %rdi
.L61:
	testb	$1, %sil
	je	.L66
	movzbl	(%rax,%rdi), %eax
	movb	%al, (%r9,%rdi)
.L66:
	movq	%r12, %rdi
	call	two_way_short_needle.constprop.0
	cmpl	$-1, %eax
	je	.L100
.L63:
	movq	%r14, %rsi
	cltq
	salq	$24, %rsi
	xorq	%rsi, %rax
	addq	%rax, %rbx
.L67:
	addq	$104729, %r13
	addq	$1, %r14
	cmpq	$214484992, %r13
	je	.L101
.L68:
	imull	$1664525, %r15d, %edx
	movl	%r14d, %esi
	andl	$7, %esi
	addl	$4, %esi
	leal	1013904223(%rdx), %r15d
	testb	$1, %r14b
	jne	.L102
	movl	%r15d, %eax
	movl	%r15d, %edi
	imulq	$1321528399, %rax, %rax
	shrq	$35, %rax
	imull	$26, %eax, %eax
	subl	%eax, %edi
	leal	97(%rdi), %eax
	movl	%r15d, %edi
	shrl	$3, %edi
	movb	%al, 32(%rsp)
	movl	%edi, %eax
	imulq	$1321528399, %rax, %rax
	shrq	$35, %rax
	imull	$26, %eax, %eax
	subl	%eax, %edi
	addl	$97, %edi
	movb	%dil, 33(%rsp)
	movl	%r15d, %edi
	shrl	$6, %edi
	movl	%edi, %eax
	imulq	$1321528399, %rax, %rax
	shrq	$35, %rax
	imull	$26, %eax, %eax
	subl	%eax, %edi
	addl	$97, %edi
	movb	%dil, 34(%rsp)
	movl	%r15d, %edi
	shrl	$9, %edi
	movl	%edi, %eax
	imulq	$1321528399, %rax, %rax
	shrq	$35, %rax
	imull	$26, %eax, %eax
	subl	%eax, %edi
	addl	$97, %edi
	movb	%dil, 35(%rsp)
	cmpl	$4, %esi
	je	.L66
	movl	%r15d, %edi
	shrl	$12, %edi
	movl	%edi, %eax
	imulq	$1321528399, %rax, %rax
	shrq	$35, %rax
	imull	$26, %eax, %eax
	subl	%eax, %edi
	addl	$97, %edi
	movb	%dil, 36(%rsp)
	movl	%r15d, %edi
	shrl	$15, %edi
	movl	%edi, %eax
	imulq	$1321528399, %rax, %rax
	shrq	$35, %rax
	imull	$26, %eax, %eax
	subl	%eax, %edi
	addl	$97, %edi
	movb	%dil, 37(%rsp)
	cmpl	$6, %esi
	je	.L66
	movl	%r15d, %edi
	shrl	$18, %edi
	movl	%edi, %eax
	imulq	$1321528399, %rax, %rax
	shrq	$35, %rax
	imull	$26, %eax, %eax
	subl	%eax, %edi
	addl	$97, %edi
	movb	%dil, 38(%rsp)
	cmpl	$7, %esi
	je	.L66
	movl	%r15d, %edi
	shrl	$21, %edi
	movl	%edi, %eax
	imulq	$1321528399, %rax, %rax
	shrq	$35, %rax
	imull	$26, %eax, %eax
	subl	%eax, %edi
	addl	$97, %edi
	movb	%dil, 39(%rsp)
	cmpl	$8, %esi
	je	.L66
	movl	%r15d, %edi
	shrl	$24, %edi
	movl	%edi, %eax
	imulq	$1321528399, %rax, %rax
	shrq	$35, %rax
	imull	$26, %eax, %eax
	subl	%eax, %edi
	addl	$97, %edi
	movb	%dil, 40(%rsp)
	cmpl	$9, %esi
	je	.L66
	movl	%r15d, %edi
	shrl	$27, %edi
	movl	%edi, %eax
	imulq	$1321528399, %rax, %rax
	shrq	$35, %rax
	imull	$26, %eax, %eax
	subl	%eax, %edi
	addl	$97, %edi
	movb	%dil, 41(%rsp)
	cmpl	$11, %esi
	jne	.L66
	movl	%r15d, %eax
	movq	%r12, %rdi
	shrl	$30, %eax
	addl	$97, %eax
	movb	%al, 42(%rsp)
	call	two_way_short_needle.constprop.0
	cmpl	$-1, %eax
	jne	.L63
	.p2align 4
	.p2align 3
.L100:
	addq	%r13, %rbx
	jmp	.L67
	.p2align 4,,10
	.p2align 3
.L99:
	movl	%esi, %r10d
	xorl	%eax, %eax
	andl	$-8, %r10d
.L58:
	movl	%eax, %r9d
	addl	$8, %eax
	movq	(%rdi,%r9), %r11
	movq	%r11, (%r12,%r9)
	cmpl	%r10d, %eax
	jb	.L58
	leaq	(%r12,%rax), %r9
	addq	%rdi, %rax
	jmp	.L57
.L101:
	movq	24(%rsp), %rax
	movzbl	23(%rsp), %ecx
	movb	%cl, (%rax)
	addl	$1, %ecx
	addq	$16381, %rax
	movb	%cl, 23(%rsp)
	leaq	131048+haystack(%rip), %rcx
	movq	%rax, 24(%rsp)
	cmpq	%rax, %rcx
	jne	.L55
	movq	%rbx, %rsi
	leaq	.LC0(%rip), %rdi
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
	.local	haystack
	.comm	haystack,524416,32
	.ident	"GCC: (Debian 14.2.0-19) 14.2.0"
	.section	.note.GNU-stack,"",@progbits
