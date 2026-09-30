	.file	"lz4_compress.c"
	.text
	.section	.rodata.str1.1,"aMS",@progbits,1
.LC11:
	.string	"%016llx\n"
	.section	.text.startup,"ax",@progbits
	.p2align 4
	.globl	main
	.type	main, @function
main:
.LFB15:
	.cfi_startproc
	pushq	%rbp
	.cfi_def_cfa_offset 16
	.cfi_offset 6, -16
	xorl	%edx, %edx
	movl	$1511734397, %eax
	movq	%rsp, %rbp
	.cfi_def_cfa_register 6
	pushq	%r15
	pushq	%r14
	.cfi_offset 15, -24
	.cfi_offset 14, -32
	leaq	src_data(%rip), %r14
	pushq	%r13
	pushq	%r12
	pushq	%rbx
	andq	$-32, %rsp
	subq	$64, %rsp
	.cfi_offset 13, -40
	.cfi_offset 12, -48
	.cfi_offset 3, -56
	.p2align 4
	.p2align 3
.L5:
	imull	$1664525, %eax, %eax
	addl	$1013904223, %eax
	movl	%eax, %ecx
	andl	$15, %ecx
	cmpl	$5, %ecx
	movl	%eax, %ecx
	ja	.L2
	cmpq	$127, %rdx
	jbe	.L2
	andl	$63, %ecx
	leal	-128(%rcx,%rdx), %ecx
	movzbl	(%r14,%rcx), %ecx
	movb	%cl, (%r14,%rdx)
	addq	$1, %rdx
	cmpq	$524288, %rdx
	jne	.L5
.L4:
	movq	$0, 24(%rsp)
	leaq	src_data(%rip), %r12
	leaq	hash_table(%rip), %r11
	leaq	524288(%r12), %r13
	leaq	524276(%r12), %r15
	.p2align 4
	.p2align 3
.L6:
	movq	%r11, %rdi
	movl	$65536, %edx
	xorl	%esi, %esi
	call	memset@PLT
	leaq	src_data(%rip), %r9
	movq	%r12, 56(%rsp)
	leaq	dst_data(%rip), %rbx
	leaq	1(%r9), %r10
	movq	%rbx, %rcx
	movq	%rax, %r11
	movq	%r10, %rbx
	.p2align 4
	.p2align 3
.L45:
	movl	(%rbx), %esi
	movq	%rbx, %rdi
	subq	%r14, %rdi
	imull	$-1640531535, %esi, %eax
	shrl	$18, %eax
	movl	(%r11,%rax,4), %edx
	movl	%edi, (%r11,%rax,4)
	movq	%rbx, %rax
	subq	%r9, %rax
	addq	%r14, %rdx
	cmpq	%rbx, %rdx
	jnb	.L7
	cmpq	%r14, %rdx
	jb	.L7
	cmpl	(%rdx), %esi
	je	.L89
.L7:
	sarq	$6, %rax
	leaq	1(%rbx,%rax), %rbx
.L44:
	cmpq	%r15, %rbx
	jb	.L45
	movq	%r13, %r8
	movq	%rcx, %rbx
	movq	56(%rsp), %r12
	leaq	1(%rcx), %rcx
	subq	%r9, %r8
	cmpl	$14, %r8d
	jbe	.L46
	movb	$-16, (%rbx)
.L47:
	movq	%r8, 56(%rsp)
	movl	%r8d, %edx
	movq	%rcx, %rdi
	movq	%r9, %rsi
	vzeroupper
	call	memcpy@PLT
	movq	56(%rsp), %r8
	leaq	hash_table(%rip), %r11
	movq	%rax, %rcx
	leal	-1(%r8), %eax
	leaq	1(%rcx,%rax), %rcx
.L48:
	leaq	dst_data(%rip), %rsi
	xorb	$85, (%r12)
	addq	$4099, %r12
	subq	%rsi, %rcx
	movzbl	(%rsi), %eax
	movq	%rcx, %rdx
	shrl	%ecx
	salq	$32, %rdx
	xorq	%rdx, %rax
	movzbl	(%rsi,%rcx), %edx
	salq	$16, %rdx
	xorq	%rdx, %rax
	addq	%rax, 24(%rsp)
	leaq	98376+src_data(%rip), %rax
	cmpq	%r12, %rax
	jne	.L6
	movq	24(%rsp), %rsi
	leaq	.LC11(%rip), %rdi
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
	.cfi_remember_state
	.cfi_def_cfa 7, 8
	ret
	.p2align 4,,10
	.p2align 3
.L2:
	.cfi_restore_state
	shrl	$24, %ecx
	movb	%cl, (%r14,%rdx)
	addq	$1, %rdx
	cmpq	$524288, %rdx
	jne	.L5
	jmp	.L4
	.p2align 4,,10
	.p2align 3
.L89:
	addq	$4, %rbx
	leaq	4(%rdx), %r12
	cmpq	%r13, %rbx
	jb	.L8
	jmp	.L90
	.p2align 5
	.p2align 4,,10
	.p2align 3
.L10:
	addq	$1, %rbx
	addq	$1, %r12
	cmpq	%r13, %rbx
	je	.L86
.L8:
	movzbl	(%r12), %esi
	cmpb	%sil, (%rbx)
	je	.L10
.L86:
	movq	%r12, %rsi
	subq	%rdx, %rsi
	leal	-4(%rsi), %edi
	movl	%esi, 48(%rsp)
	movl	%edi, 52(%rsp)
.L9:
	leaq	1(%rcx), %rdi
	cmpl	$14, %eax
	ja	.L91
	movl	%eax, %edx
	sall	$4, %edx
	movb	%dl, (%rcx)
	testl	%eax, %eax
	je	.L20
.L19:
	movl	%eax, %edx
	movq	%rcx, 32(%rsp)
	movq	%r9, %rsi
	movq	%rdx, 40(%rsp)
	vzeroupper
	call	memcpy@PLT
	movq	40(%rsp), %rdx
	movq	32(%rsp), %rcx
	movq	%rax, %rdi
	leaq	hash_table(%rip), %r11
	addq	%rdx, %rdi
.L20:
	movq	%rbx, %rax
	leaq	2(%rdi), %rsi
	subq	%r12, %rax
	cmpl	$14, 52(%rsp)
	movw	%ax, (%rdi)
	movzbl	(%rcx), %eax
	ja	.L92
	orb	52(%rsp), %al
	movq	%rbx, %r9
	movb	%al, (%rcx)
	movq	%rsi, %rcx
	jmp	.L44
	.p2align 4,,10
	.p2align 3
.L46:
	movl	%r8d, %eax
	sall	$4, %eax
	movb	%al, (%rbx)
	testl	%r8d, %r8d
	jne	.L47
	vzeroupper
	jmp	.L48
	.p2align 4,,10
	.p2align 3
.L91:
	leal	-15(%rax), %r8d
	movb	$-16, (%rcx)
	cmpl	$254, %r8d
	jbe	.L12
	leal	-270(%rax), %esi
	movl	$2155905153, %r10d
	movq	%rsi, %rdx
	imulq	%r10, %rsi
	shrq	$39, %rsi
	movl	%esi, %r11d
	leal	1(%rsi), %r10d
	cmpl	$7904, %edx
	jbe	.L50
	movl	%r10d, %esi
	vmovq	%rdi, %xmm7
	vmovd	%eax, %xmm0
	movq	%rcx, %rdx
	shrl	$5, %esi
	vpbroadcastq	%xmm7, %ymm1
	vpbroadcastd	%xmm0, %ymm0
	vpbroadcastd	.LC13(%rip), %ymm3
	salq	$5, %rsi
	vpcmpeqd	%ymm4, %ymm4, %ymm4
	vpaddq	.LC0(%rip), %ymm1, %ymm1
	vpbroadcastq	.LC14(%rip), %ymm2
	vpaddd	.LC1(%rip), %ymm0, %ymm0
	addq	%rcx, %rsi
	.p2align 5
	.p2align 4
	.p2align 3
.L14:
	vmovdqu	%ymm4, 1(%rdx)
	addq	$32, %rdx
	vmovdqa	%ymm0, %ymm6
	vmovdqa	%ymm1, %ymm5
	vpaddd	%ymm3, %ymm0, %ymm0
	vpaddq	%ymm2, %ymm1, %ymm1
	cmpq	%rdx, %rsi
	jne	.L14
	movl	%r10d, %edx
	andl	$-32, %edx
	imull	$-255, %edx, %esi
	addl	%esi, %r8d
	movl	%edx, %esi
	addq	%rsi, %rdi
	andl	$31, %r10d
	je	.L93
.L13:
	movl	%r11d, %r10d
	subl	%edx, %r10d
	leal	1(%r10), %esi
	cmpl	$14, %r10d
	jbe	.L51
	addl	$1, %edx
	vpcmpeqd	%xmm0, %xmm0, %xmm0
	vmovdqu	%xmm0, (%rcx,%rdx)
	movl	%esi, %edx
	andl	$-16, %edx
	imull	$-255, %edx, %r10d
	addq	%rdi, %rdx
	addl	%r8d, %r10d
	andl	$15, %esi
	je	.L94
.L16:
	leal	-255(%r10), %esi
	movb	$-1, (%rdx)
	leaq	1(%rdx), %r8
	cmpl	$254, %esi
	jbe	.L87
	leal	-510(%r10), %esi
	movb	$-1, 1(%rdx)
	leaq	2(%rdx), %rdi
	cmpl	$254, %esi
	jbe	.L59
	leal	-765(%r10), %esi
	movb	$-1, 2(%rdx)
	leaq	3(%rdx), %r8
	cmpl	$254, %esi
	jbe	.L15
	leal	-1020(%r10), %esi
	movb	$-1, 3(%rdx)
	leaq	4(%rdx), %rdi
	cmpl	$254, %esi
	jbe	.L59
	leal	-1275(%r10), %esi
	movb	$-1, 4(%rdx)
	leaq	5(%rdx), %r8
	cmpl	$254, %esi
	jbe	.L15
	leal	-1530(%r10), %esi
	movb	$-1, 5(%rdx)
	leaq	6(%rdx), %rdi
	cmpl	$254, %esi
	jbe	.L59
	leal	-1785(%r10), %esi
	movb	$-1, 6(%rdx)
	leaq	7(%rdx), %r8
	cmpl	$254, %esi
	jbe	.L15
	leal	-2040(%r10), %esi
	movb	$-1, 7(%rdx)
	leaq	8(%rdx), %rdi
	cmpl	$254, %esi
	jbe	.L59
	leal	-2295(%r10), %esi
	movb	$-1, 8(%rdx)
	leaq	9(%rdx), %r8
	cmpl	$254, %esi
	jbe	.L15
	leal	-2550(%r10), %esi
	movb	$-1, 9(%rdx)
	leaq	10(%rdx), %rdi
	cmpl	$254, %esi
	jbe	.L59
	leal	-2805(%r10), %esi
	movb	$-1, 10(%rdx)
	leaq	11(%rdx), %r8
	cmpl	$254, %esi
	jbe	.L15
	leal	-3060(%r10), %esi
	movb	$-1, 11(%rdx)
	leaq	12(%rdx), %rdi
	cmpl	$254, %esi
	jbe	.L59
	leal	-3315(%r10), %esi
	movb	$-1, 12(%rdx)
	leaq	13(%rdx), %r8
	cmpl	$254, %esi
	jbe	.L15
	leal	-3570(%r10), %esi
	movb	$-1, 13(%rdx)
	leaq	14(%rdx), %rdi
	cmpl	$254, %esi
	jbe	.L59
	movb	$-1, 14(%rdx)
	leaq	15(%rdx), %r8
	leal	-3825(%r10), %esi
	.p2align 4
	.p2align 3
.L15:
	movb	%sil, (%r8)
	addq	$2, %rdi
	jmp	.L19
	.p2align 4,,10
	.p2align 3
.L92:
	orl	$15, %eax
	movb	%al, (%rcx)
	movl	48(%rsp), %eax
	leal	-19(%rax), %edx
	cmpl	$254, %edx
	jbe	.L22
	leal	-274(%rax), %r9d
	movl	$2155905153, %ecx
	movq	%r9, %rax
	imulq	%rcx, %r9
	shrq	$39, %r9
	leal	1(%r9), %r10d
	cmpl	$7904, %eax
	jbe	.L60
	movl	%r10d, %ecx
	vmovq	%rsi, %xmm7
	vpcmpeqd	%ymm4, %ymm4, %ymm4
	movq	%rdi, %rax
	shrl	$5, %ecx
	movl	$-8160, %r12d
	vpbroadcastq	%xmm7, %ymm1
	vpbroadcastd	48(%rsp), %ymm0
	salq	$5, %rcx
	vmovd	%r12d, %xmm3
	vpaddq	.LC0(%rip), %ymm1, %ymm1
	vpbroadcastq	.LC14(%rip), %ymm2
	vpaddd	.LC10(%rip), %ymm0, %ymm0
	addq	%rdi, %rcx
	vpbroadcastd	%xmm3, %ymm3
	.p2align 5
	.p2align 4
	.p2align 3
.L24:
	vmovdqu	%ymm4, 2(%rax)
	addq	$32, %rax
	vmovdqa	%ymm0, %ymm6
	vmovdqa	%ymm1, %ymm5
	vpaddd	%ymm3, %ymm0, %ymm0
	vpaddq	%ymm2, %ymm1, %ymm1
	cmpq	%rcx, %rax
	jne	.L24
	movl	%r10d, %eax
	andl	$-32, %eax
	imull	$-255, %eax, %ecx
	addl	%ecx, %edx
	movl	%eax, %ecx
	addq	%rcx, %rsi
	andl	$31, %r10d
	je	.L95
.L23:
	subl	%eax, %r9d
	leal	1(%r9), %r10d
	cmpl	$14, %r9d
	jbe	.L61
	addl	$2, %eax
	vpcmpeqd	%xmm0, %xmm0, %xmm0
	vmovdqu	%xmm0, (%rdi,%rax)
	movl	%r10d, %eax
	andl	$-16, %eax
	imull	$-255, %eax, %ecx
	addq	%rsi, %rax
	addl	%edx, %ecx
	andl	$15, %r10d
	je	.L96
.L27:
	leal	-255(%rcx), %edx
	movb	$-1, (%rax)
	leaq	1(%rax), %rsi
	cmpl	$254, %edx
	jbe	.L22
	leal	-510(%rcx), %edx
	movb	$-1, 1(%rax)
	leaq	2(%rax), %rsi
	cmpl	$254, %edx
	jbe	.L22
	leal	-765(%rcx), %edx
	movb	$-1, 2(%rax)
	leaq	3(%rax), %rsi
	cmpl	$254, %edx
	jbe	.L22
	leal	-1020(%rcx), %edx
	movb	$-1, 3(%rax)
	leaq	4(%rax), %rsi
	cmpl	$254, %edx
	jbe	.L22
	leal	-1275(%rcx), %edx
	movb	$-1, 4(%rax)
	leaq	5(%rax), %rsi
	cmpl	$254, %edx
	jbe	.L22
	leal	-1530(%rcx), %edx
	movb	$-1, 5(%rax)
	leaq	6(%rax), %rsi
	cmpl	$254, %edx
	jbe	.L22
	leal	-1785(%rcx), %edx
	movb	$-1, 6(%rax)
	leaq	7(%rax), %rsi
	cmpl	$254, %edx
	jbe	.L22
	leal	-2040(%rcx), %edx
	movb	$-1, 7(%rax)
	leaq	8(%rax), %rsi
	cmpl	$254, %edx
	jbe	.L22
	leal	-2295(%rcx), %edx
	movb	$-1, 8(%rax)
	leaq	9(%rax), %rsi
	cmpl	$254, %edx
	jbe	.L22
	leal	-2550(%rcx), %edx
	movb	$-1, 9(%rax)
	leaq	10(%rax), %rsi
	cmpl	$254, %edx
	jbe	.L22
	leal	-2805(%rcx), %edx
	movb	$-1, 10(%rax)
	leaq	11(%rax), %rsi
	cmpl	$254, %edx
	jbe	.L22
	leal	-3060(%rcx), %edx
	movb	$-1, 11(%rax)
	leaq	12(%rax), %rsi
	cmpl	$254, %edx
	jbe	.L22
	leal	-3315(%rcx), %edx
	movb	$-1, 12(%rax)
	leaq	13(%rax), %rsi
	cmpl	$254, %edx
	jbe	.L22
	leal	-3570(%rcx), %edx
	movb	$-1, 13(%rax)
	leaq	14(%rax), %rsi
	cmpl	$254, %edx
	jbe	.L22
	movb	$-1, 14(%rax)
	leaq	15(%rax), %rsi
	leal	-3825(%rcx), %edx
	.p2align 4
	.p2align 3
.L22:
	movb	%dl, (%rsi)
	leaq	1(%rsi), %rcx
	movq	%rbx, %r9
	jmp	.L44
.L59:
	movq	%r8, %rdx
	movq	%rdi, %r8
.L87:
	movq	%rdx, %rdi
	jmp	.L15
.L90:
	movl	$0, 52(%rsp)
	movl	$4, 48(%rsp)
	jmp	.L9
.L12:
	movb	%r8b, 1(%rcx)
	leaq	2(%rcx), %rdi
	jmp	.L19
.L61:
	movl	%edx, %ecx
	movq	%rsi, %rax
	jmp	.L27
.L60:
	xorl	%eax, %eax
	jmp	.L23
.L51:
	movl	%r8d, %r10d
	movq	%rdi, %rdx
	jmp	.L16
.L50:
	xorl	%edx, %edx
	jmp	.L13
.L94:
	vmovq	%rdi, %xmm7
	leaq	1(%rdi), %rdx
	movl	$15, %edi
	vpinsrq	$1, %rdx, %xmm7, %xmm1
	vmovd	%r8d, %xmm7
	vpbroadcastd	%xmm7, %xmm0
	vpaddd	.LC7(%rip), %xmm0, %xmm0
	vpextrd	$3, %xmm0, %esi
	vmovq	%rdi, %xmm0
	movl	$14, %edi
	vpunpcklqdq	%xmm0, %xmm0, %xmm0
	vpaddq	%xmm0, %xmm1, %xmm0
	vpextrq	$1, %xmm0, %r8
	vmovq	%rdi, %xmm0
	vpunpcklqdq	%xmm0, %xmm0, %xmm0
	vpaddq	%xmm0, %xmm1, %xmm1
	vpextrq	$1, %xmm1, %rdi
	jmp	.L15
.L96:
	vmovd	%edx, %xmm7
	leaq	1(%rsi), %rax
	vpbroadcastd	%xmm7, %xmm0
	vpaddd	.LC7(%rip), %xmm0, %xmm0
	vmovq	%rsi, %xmm7
	vpextrd	$3, %xmm0, %edx
	vpinsrq	$1, %rax, %xmm7, %xmm0
	movl	$15, %eax
	vmovq	%rax, %xmm1
	vpunpcklqdq	%xmm1, %xmm1, %xmm1
	vpaddq	%xmm1, %xmm0, %xmm0
	vpextrq	$1, %xmm0, %rsi
	jmp	.L22
.L93:
	movl	$-6375, %edx
	vmovd	%edx, %xmm0
	vpbroadcastd	%xmm0, %ymm0
	vpaddd	%ymm0, %ymm6, %ymm6
	vextracti128	$0x1, %ymm6, %xmm0
	vpextrd	$3, %xmm0, %esi
	vpbroadcastq	.LC15(%rip), %ymm0
	vpaddq	%ymm0, %ymm5, %ymm0
	vextracti128	$0x1, %ymm0, %xmm0
	vpextrq	$1, %xmm0, %r8
	vpbroadcastq	.LC16(%rip), %ymm0
	vpaddq	%ymm0, %ymm5, %ymm5
	vextracti128	$0x1, %ymm5, %xmm0
	vpextrq	$1, %xmm0, %rdi
	jmp	.L15
.L95:
	movl	$-6375, %eax
	vmovd	%eax, %xmm0
	vpbroadcastd	%xmm0, %ymm0
	vpaddd	%ymm0, %ymm6, %ymm6
	vextracti128	$0x1, %ymm6, %xmm0
	vpextrd	$3, %xmm0, %edx
	vpbroadcastq	.LC15(%rip), %ymm0
	vpaddq	%ymm0, %ymm5, %ymm5
	vextracti128	$0x1, %ymm5, %xmm0
	vpextrq	$1, %xmm0, %rsi
	jmp	.L22
	.cfi_endproc
.LFE15:
	.size	main, .-main
	.local	hash_table
	.comm	hash_table,65536,32
	.local	dst_data
	.comm	dst_data,1048576,32
	.local	src_data
	.comm	src_data,524352,32
	.section	.rodata.cst32,"aM",@progbits,32
	.align 32
.LC0:
	.quad	0
	.quad	1
	.quad	2
	.quad	3
	.align 32
.LC1:
	.long	-15
	.long	-270
	.long	-525
	.long	-780
	.long	-1035
	.long	-1290
	.long	-1545
	.long	-1800
	.section	.rodata.cst16,"aM",@progbits,16
	.align 16
.LC7:
	.long	-3315
	.long	-3570
	.long	-3825
	.long	-4080
	.section	.rodata.cst32
	.align 32
.LC10:
	.long	-19
	.long	-274
	.long	-529
	.long	-784
	.long	-1039
	.long	-1294
	.long	-1549
	.long	-1804
	.section	.rodata.cst4,"aM",@progbits,4
	.align 4
.LC13:
	.long	-8160
	.section	.rodata.cst8,"aM",@progbits,8
	.align 8
.LC14:
	.quad	32
	.align 8
.LC15:
	.quad	29
	.align 8
.LC16:
	.quad	28
	.ident	"GCC: (Debian 14.2.0-19) 14.2.0"
	.section	.note.GNU-stack,"",@progbits
