	.file	"chacha20_block.c"
	.text
	.p2align 4
	.type	chacha20_core, @function
chacha20_core:
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
	.cfi_offset 15, -24
	.cfi_offset 14, -32
	.cfi_offset 13, -40
	.cfi_offset 12, -48
	.cfi_offset 3, -56
	movq	%rsi, %rbx
	andq	$-32, %rsp
	subq	$8, %rsp
	movq	%rdi, -88(%rsp)
	movq	%rsi, -96(%rsp)
	movq	(%rsi), %rax
	movq	8(%rsi), %rdi
	movq	24(%rbx), %rcx
	movq	16(%rsi), %rsi
	vmovdqu	32(%rbx), %ymm0
	movq	%rax, -56(%rsp)
	movl	%eax, %r14d
	movq	%rsi, -40(%rsp)
	movl	-40(%rsp), %r10d
	vmovdqa	%ymm0, -24(%rsp)
	movq	%rdi, -48(%rsp)
	movl	-24(%rsp), %edi
	movq	%rcx, -32(%rsp)
	movl	-8(%rsp), %ecx
	movl	-52(%rsp), %r9d
	movl	(%rsp), %eax
	movl	$10, -80(%rsp)
	movl	-36(%rsp), %r11d
	movl	-4(%rsp), %edx
	movl	-20(%rsp), %esi
	movl	-48(%rsp), %r8d
	movl	%eax, -60(%rsp)
	movl	4(%rsp), %eax
	movl	-32(%rsp), %r12d
	movl	-16(%rsp), %r15d
	movl	-44(%rsp), %ebx
	movl	%eax, -68(%rsp)
	movl	-12(%rsp), %eax
	movl	-28(%rsp), %r13d
	movl	%eax, -72(%rsp)
	movl	-60(%rsp), %eax
	.p2align 4
	.p2align 3
.L2:
	addl	%r10d, %r14d
	addl	%r12d, %r8d
	addl	%r11d, %r9d
	addl	%r13d, %ebx
	xorl	%r14d, %ecx
	xorl	%r8d, %eax
	xorl	%r9d, %edx
	rorx	$16, %ecx, %ecx
	addl	%ecx, %edi
	rorx	$16, %eax, %eax
	rorx	$16, %edx, %edx
	xorl	%edi, %r10d
	addl	%edx, %esi
	rorx	$20, %r10d, %r10d
	addl	%r10d, %r14d
	xorl	%esi, %r11d
	xorl	%r14d, %ecx
	rorx	$20, %r11d, %r11d
	addl	%r11d, %r9d
	rorx	$24, %ecx, %ecx
	addl	%ecx, %edi
	xorl	%r9d, %edx
	xorl	%edi, %r10d
	movl	%edi, -60(%rsp)
	leal	(%rax,%r15), %edi
	movl	-68(%rsp), %r15d
	rorx	$25, %r10d, %r10d
	movl	%r10d, -76(%rsp)
	xorl	%edi, %r12d
	rorx	$24, %edx, %edx
	xorl	%ebx, %r15d
	rorx	$20, %r12d, %r12d
	addl	%r12d, %r8d
	addl	%edx, %esi
	rorx	$16, %r15d, %r10d
	movl	-72(%rsp), %r15d
	xorl	%r8d, %eax
	xorl	%esi, %r11d
	movl	%esi, -64(%rsp)
	rorx	$24, %eax, %eax
	addl	%eax, %edi
	rorx	$25, %r11d, %r11d
	addl	%r10d, %r15d
	xorl	%edi, %r12d
	addl	%r11d, %r14d
	xorl	%r15d, %r13d
	movl	%r15d, %esi
	rorx	$25, %r12d, %r12d
	rorx	$20, %r13d, %r13d
	addl	%r13d, %ebx
	xorl	%ebx, %r10d
	rorx	$24, %r10d, %r10d
	addl	%r10d, %esi
	xorl	%esi, %r13d
	xorl	%r14d, %r10d
	addl	%r12d, %r9d
	xorl	%r9d, %ecx
	rorx	$16, %r10d, %r10d
	addl	%r10d, %edi
	rorx	$25, %r13d, %r13d
	rorx	$16, %ecx, %ecx
	addl	%ecx, %esi
	xorl	%edi, %r11d
	addl	%r13d, %r8d
	xorl	%esi, %r12d
	rorx	$20, %r11d, %r11d
	addl	%r11d, %r14d
	xorl	%r8d, %edx
	rorx	$20, %r12d, %r12d
	addl	%r12d, %r9d
	xorl	%r14d, %r10d
	rorx	$16, %edx, %edx
	xorl	%r9d, %ecx
	rorx	$24, %r10d, %r15d
	movl	%r15d, -68(%rsp)
	addl	%edi, %r15d
	rorx	$24, %ecx, %ecx
	leal	(%rsi,%rcx), %r10d
	movl	-60(%rsp), %edi
	movl	-64(%rsp), %esi
	xorl	%r10d, %r12d
	movl	%r10d, -72(%rsp)
	movl	-76(%rsp), %r10d
	xorl	%r15d, %r11d
	addl	%edx, %edi
	rorx	$25, %r11d, %r11d
	rorx	$25, %r12d, %r12d
	addl	%r10d, %ebx
	xorl	%edi, %r13d
	xorl	%ebx, %eax
	rorx	$20, %r13d, %r13d
	addl	%r13d, %r8d
	rorx	$16, %eax, %eax
	addl	%eax, %esi
	xorl	%r8d, %edx
	xorl	%esi, %r10d
	rorx	$24, %edx, %edx
	addl	%edx, %edi
	rorx	$20, %r10d, %r10d
	addl	%r10d, %ebx
	xorl	%edi, %r13d
	xorl	%ebx, %eax
	rorx	$25, %r13d, %r13d
	rorx	$24, %eax, %eax
	addl	%eax, %esi
	xorl	%esi, %r10d
	subl	$1, -80(%rsp)
	rorx	$25, %r10d, %r10d
	jne	.L2
	vmovd	%edi, %xmm2
	vmovd	%r15d, %xmm3
	vmovd	%ecx, %xmm4
	movl	%edx, -64(%rsp)
	vpinsrd	$1, -72(%rsp), %xmm3, %xmm1
	vpinsrd	$1, %esi, %xmm2, %xmm0
	vmovd	%eax, %xmm5
	movl	%eax, -60(%rsp)
	movl	%r14d, -56(%rsp)
	vpunpcklqdq	%xmm1, %xmm0, %xmm0
	vpinsrd	$1, -68(%rsp), %xmm5, %xmm1
	movl	%r11d, -36(%rsp)
	vmovdqa	%xmm0, -24(%rsp)
	vpinsrd	$1, %edx, %xmm4, %xmm0
	movq	-96(%rsp), %rdx
	vpunpcklqdq	%xmm1, %xmm0, %xmm0
	movl	%r9d, -52(%rsp)
	leaq	4(%rdx), %rax
	movq	-88(%rsp), %rdx
	movl	%r12d, -32(%rsp)
	movl	%r8d, -48(%rsp)
	subq	%rax, %rdx
	movl	%r13d, -28(%rsp)
	movl	%ebx, -44(%rsp)
	movl	%r10d, -40(%rsp)
	vmovdqa	%xmm0, -8(%rsp)
	cmpq	$24, %rdx
	jbe	.L3
	movq	-96(%rsp), %rbx
	movq	-88(%rsp), %rax
	vmovdqu	(%rbx), %ymm0
	vpaddd	-56(%rsp), %ymm0, %ymm0
	vmovdqu	%ymm0, (%rax)
	vmovdqu	32(%rbx), %ymm0
	vpaddd	-24(%rsp), %ymm0, %ymm0
	vmovdqu	%ymm0, 32(%rax)
.L6:
	vzeroupper
	leaq	-40(%rbp), %rsp
	popq	%rbx
	popq	%r12
	popq	%r13
	popq	%r14
	popq	%r15
	popq	%rbp
	.cfi_remember_state
	.cfi_def_cfa 7, 8
	ret
	.p2align 4,,10
	.p2align 3
.L3:
	.cfi_restore_state
	movq	-96(%rsp), %rdx
	movq	-88(%rsp), %rax
	addl	(%rdx), %r14d
	movl	%r14d, (%rax)
	addl	4(%rdx), %r9d
	movq	%rax, %r14
	movl	%r9d, 4(%rax)
	addl	8(%rdx), %r8d
	movl	%r8d, 8(%rax)
	addl	12(%rdx), %ebx
	movl	%ebx, 12(%rax)
	addl	16(%rdx), %r10d
	movq	%rdx, %rbx
	movl	%r10d, 16(%rax)
	addl	20(%rdx), %r11d
	movl	%r11d, 20(%rax)
	addl	24(%rdx), %r12d
	movl	%r12d, 24(%rax)
	movl	28(%rdx), %r8d
	addl	%r13d, %r8d
	movl	%r8d, 28(%rax)
	addl	32(%rdx), %edi
	movl	%edi, 32(%rax)
	addl	36(%rdx), %esi
	movl	%esi, 36(%rax)
	addl	40(%rdx), %r15d
	movl	%r15d, 40(%rax)
	movl	-72(%rsp), %esi
	addl	44(%rdx), %esi
	movl	%esi, 44(%rax)
	addl	48(%rdx), %ecx
	movl	%ecx, 48(%rax)
	movl	-64(%rsp), %edx
	addl	52(%rbx), %edx
	movl	%edx, 52(%rax)
	movl	-60(%rsp), %eax
	addl	56(%rbx), %eax
	movl	%eax, 56(%r14)
	movl	-68(%rsp), %eax
	addl	60(%rbx), %eax
	movl	%eax, 60(%r14)
	jmp	.L6
	.cfi_endproc
.LFE11:
	.size	chacha20_core, .-chacha20_core
	.section	.rodata.str1.1,"aMS",@progbits,1
.LC4:
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
	leaq	test_in.0(%rip), %rsi
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
	subq	$136, %rsp
	.cfi_def_cfa_offset 192
	leaq	64(%rsp), %r12
	movq	%r12, %rdi
	call	chacha20_core
	cmpl	$-454561520, 64(%rsp)
	jne	.L10
	cmpl	$358169553, 68(%rsp)
	je	.L17
.L10:
	movl	$2, %eax
.L9:
	addq	$136, %rsp
	.cfi_remember_state
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
.L17:
	.cfi_restore_state
	cmpl	$-394014517, 120(%rsp)
	jne	.L10
	cmpl	$1312575650, 124(%rsp)
	jne	.L10
	movl	$185273099, 44(%rsp)
	xorl	%ebp, %ebp
	xorl	%ebx, %ebx
	movq	%rsp, %r13
	movq	.LC2(%rip), %rax
	vmovdqa	.LC0(%rip), %xmm0
	movl	$-1985229329, 60(%rsp)
	movl	$67372036, %r15d
	movq	%rax, 36(%rsp)
	movq	.LC3(%rip), %rax
	vmovdqa	%xmm0, (%rsp)
	vmovdqa	.LC1(%rip), %xmm0
	movq	%rax, 52(%rsp)
	vmovdqu	%xmm0, 20(%rsp)
.L11:
	xorl	%r14d, %r14d
	.p2align 4
	.p2align 3
.L13:
	movl	%r14d, %eax
	movq	%r13, %rsi
	movq	%r12, %rdi
	movl	%r15d, 16(%rsp)
	xorl	%ebp, %eax
	addl	$1, %r14d
	movl	%eax, 48(%rsp)
	call	chacha20_core
	movl	64(%rsp), %edx
	movl	92(%rsp), %eax
	movq	%rdx, %rcx
	salq	$16, %rax
	movzbl	%dl, %edx
	salq	$32, %rcx
	xorl	%edx, %r15d
	xorq	%rcx, %rax
	movl	124(%rsp), %ecx
	xorq	%rcx, %rax
	addq	%rax, %rbx
	cmpl	$131072, %r14d
	jne	.L13
	addl	$1, %ebp
	cmpl	$16, %ebp
	jne	.L11
	movq	%rbx, %rsi
	leaq	.LC4(%rip), %rdi
	xorl	%eax, %eax
	call	printf@PLT
	xorl	%eax, %eax
	jmp	.L9
	.cfi_endproc
.LFE13:
	.size	main, .-main
	.section	.rodata
	.align 32
	.type	test_in.0, @object
	.size	test_in.0, 64
test_in.0:
	.long	1634760805
	.long	857760878
	.long	2036477234
	.long	1797285236
	.long	50462976
	.long	117835012
	.long	185207048
	.long	252579084
	.long	319951120
	.long	387323156
	.long	454695192
	.long	522067228
	.long	1
	.long	150994944
	.long	1241513984
	.long	0
	.section	.rodata.cst16,"aM",@progbits,16
	.align 16
.LC0:
	.long	1634760805
	.long	857760878
	.long	2036477234
	.long	1797285236
	.align 16
.LC1:
	.long	84215045
	.long	101058054
	.long	117901063
	.long	134744072
	.section	.rodata.cst8,"aM",@progbits,8
	.align 8
.LC2:
	.long	151587081
	.long	168430090
	.align 8
.LC3:
	.long	-559038737
	.long	19088743
	.ident	"GCC: (Debian 14.2.0-19) 14.2.0"
	.section	.note.GNU-stack,"",@progbits
