	.file	"vecreg_new_ops.c"
	.text
	.p2align 4
	.type	sat_kernel, @function
sat_kernel:
.LFB6577:
	.cfi_startproc
	xorl	%eax, %eax
	.p2align 6
	.p2align 4
	.p2align 3
.L2:
	vmovdqu	(%rsi), %xmm1
	vpaddusb	(%rdi), %xmm1, %xmm1
	addl	$1, %eax
	vpaddusb	(%rdx), %xmm1, %xmm2
	vpavgb	%xmm2, %xmm1, %xmm0
	vpsubq	%xmm1, %xmm0, %xmm0
	vpxor	%xmm2, %xmm0, %xmm0
	vmovdqu	%xmm0, (%rcx)
	cmpl	%eax, %r8d
	jne	.L2
	vpsrldq	$8, %xmm0, %xmm1
	vpxor	%xmm1, %xmm0, %xmm0
	vmovq	%xmm0, %rax
	ret
	.cfi_endproc
.LFE6577:
	.size	sat_kernel, .-sat_kernel
	.section	.rodata.str1.1,"aMS",@progbits,1
.LC3:
	.string	"%llu\n"
	.section	.text.startup,"ax",@progbits
	.p2align 4
	.globl	main
	.type	main, @function
main:
.LFB6578:
	.cfi_startproc
	pushq	%rbp
	.cfi_def_cfa_offset 16
	.cfi_offset 6, -16
	movl	$1000, %r8d
	movq	%rsp, %rbp
	.cfi_def_cfa_register 6
	andq	$-32, %rsp
	subq	$96, %rsp
	cmpl	$1, %edi
	jg	.L12
.L6:
	vmovdqa	.LC0(%rip), %xmm0
	leaq	80(%rsp), %rcx
	leaq	64(%rsp), %rdx
	movq	%rsp, %rdi
	leaq	32(%rsp), %rsi
	vmovdqa	%xmm0, (%rsp)
	vmovdqa	.LC1(%rip), %xmm0
	vmovdqa	%xmm0, 32(%rsp)
	vmovdqa	.LC2(%rip), %xmm0
	vmovdqa	%xmm0, 64(%rsp)
	call	sat_kernel
	leaq	.LC3(%rip), %rdi
	movq	%rax, %rsi
	xorl	%eax, %eax
	call	printf@PLT
	xorl	%eax, %eax
	leave
	.cfi_remember_state
	.cfi_def_cfa 7, 8
	ret
.L12:
	.cfi_restore_state
	movq	8(%rsi), %rdi
	xorl	%edx, %edx
	xorl	%esi, %esi
	call	strtoul@PLT
	movl	$4294967294, %edx
	movq	%rax, %r8
	leaq	-1(%rax), %rax
	cmpq	%rax, %rdx
	jnb	.L6
	leave
	.cfi_def_cfa 7, 8
	movl	$2, %eax
	ret
	.cfi_endproc
.LFE6578:
	.size	main, .-main
	.section	.rodata.cst16,"aM",@progbits,16
	.align 16
.LC0:
	.byte	3
	.byte	20
	.byte	37
	.byte	54
	.byte	71
	.byte	88
	.byte	105
	.byte	122
	.byte	-117
	.byte	-100
	.byte	-83
	.byte	-66
	.byte	-49
	.byte	-32
	.byte	-15
	.byte	2
	.align 16
.LC1:
	.byte	5
	.byte	16
	.byte	27
	.byte	38
	.byte	49
	.byte	60
	.byte	71
	.byte	82
	.byte	93
	.byte	104
	.byte	115
	.byte	126
	.byte	-119
	.byte	-108
	.byte	-97
	.byte	-86
	.align 16
.LC2:
	.byte	9
	.byte	16
	.byte	23
	.byte	30
	.byte	37
	.byte	44
	.byte	51
	.byte	58
	.byte	65
	.byte	72
	.byte	79
	.byte	86
	.byte	93
	.byte	100
	.byte	107
	.byte	114
	.ident	"GCC: (Debian 14.2.0-19) 14.2.0"
	.section	.note.GNU-stack,"",@progbits
