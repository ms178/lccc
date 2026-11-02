	.att_syntax
	.file	"sign_test_cmov.c"
	.text
	.globl	nonnegative_difference          # -- Begin function nonnegative_difference
	.prefalign	4, .Lfunc_end0, nop
	.type	nonnegative_difference,@function
nonnegative_difference:                 #
	.cfi_startproc
# %bb.0:
	subl	%esi, %edi
	xorl	%eax, %eax
	testl	%edi, %edi
	cmovgl	%edi, %eax
	retq
.Lfunc_end0:
	.size	nonnegative_difference, .Lfunc_end0-nonnegative_difference
	.cfi_endproc
                                        # -- End function
	.globl	negative_difference             # -- Begin function negative_difference
	.prefalign	4, .Lfunc_end1, nop
	.type	negative_difference,@function
negative_difference:                    #
	.cfi_startproc
# %bb.0:
	subl	%esi, %edi
	movl	%edi, %eax
	sarl	$31, %eax
	andl	%edi, %eax
	retq
.Lfunc_end1:
	.size	negative_difference, .Lfunc_end1-negative_difference
	.cfi_endproc
                                        # -- End function
	.globl	positive_difference             # -- Begin function positive_difference
	.prefalign	4, .Lfunc_end2, nop
	.type	positive_difference,@function
positive_difference:                    #
	.cfi_startproc
# %bb.0:
	subl	%esi, %edi
	xorl	%eax, %eax
	testl	%edi, %edi
	cmovgl	%edi, %eax
	retq
.Lfunc_end2:
	.size	positive_difference, .Lfunc_end2-positive_difference
	.cfi_endproc
                                        # -- End function
	.globl	main                            # -- Begin function main
	.prefalign	4, .Lfunc_end3, nop
	.type	main,@function
main:                                   #
	.cfi_startproc
# %bb.0:
	pushq	%rbp
	.cfi_def_cfa_offset 16
	pushq	%r14
	.cfi_def_cfa_offset 24
	pushq	%rbx
	.cfi_def_cfa_offset 32
	subq	$16, %rsp
	.cfi_def_cfa_offset 48
	.cfi_offset %rbx, -32
	.cfi_offset %r14, -24
	.cfi_offset %rbp, -16
	stmxcsr	12(%rsp)
	orl	$32832, 12(%rsp)                # imm = 0x8040
	ldmxcsr	12(%rsp)
	movl	$-2147483648, %edi              # imm = 0x80000000
	movl	$-2147483648, %esi              # imm = 0x80000000
	callq	nonnegative_difference
	movl	%eax, %ebx
	movl	$-2147483648, %edi              # imm = 0x80000000
	movl	$-2147483648, %esi              # imm = 0x80000000
	callq	negative_difference
	movl	%eax, %ebp
	movl	$-2147483648, %edi              # imm = 0x80000000
	movl	$-2147483648, %esi              # imm = 0x80000000
	callq	positive_difference
	movl	%ebx, %ecx
	movl	%ebp, %edx
	addq	%rcx, %rdx
	shlq	$5, %rcx
	addq	%rcx, %rdx
	movl	%eax, %r14d
	addq	%rdx, %r14
	shlq	$5, %rdx
	addq	%rdx, %r14
	movl	$-2147483648, %edi              # imm = 0x80000000
	movl	$-2147483647, %esi              # imm = 0x80000001
	callq	nonnegative_difference
	movl	%eax, %ebx
	movl	$-2147483648, %edi              # imm = 0x80000000
	movl	$-2147483647, %esi              # imm = 0x80000001
	callq	negative_difference
	movl	%eax, %ebp
	movl	$-2147483648, %edi              # imm = 0x80000000
	movl	$-2147483647, %esi              # imm = 0x80000001
	callq	positive_difference
	imulq	$1089, %r14, %rcx               # imm = 0x441
	movl	%ebx, %edx
	movq	%rdx, %rsi
	shlq	$5, %rsi
	addq	%rdx, %rsi
	movl	%ebp, %edx
	addq	%rsi, %rdx
	addq	%rcx, %rdx
	movl	%eax, %r14d
	addq	%rdx, %r14
	shlq	$5, %rdx
	addq	%rdx, %r14
	movl	$-2147483648, %edi              # imm = 0x80000000
	movl	$-1, %esi
	callq	nonnegative_difference
	movl	%eax, %ebx
	movl	$-2147483648, %edi              # imm = 0x80000000
	movl	$-1, %esi
	callq	negative_difference
	movl	%eax, %ebp
	movl	$-2147483648, %edi              # imm = 0x80000000
	movl	$-1, %esi
	callq	positive_difference
	imulq	$1089, %r14, %rcx               # imm = 0x441
	movl	%ebx, %edx
	movq	%rdx, %rsi
	shlq	$5, %rsi
	addq	%rdx, %rsi
	movl	%ebp, %edx
	addq	%rsi, %rdx
	addq	%rcx, %rdx
	movl	%eax, %r14d
	addq	%rdx, %r14
	shlq	$5, %rdx
	addq	%rdx, %r14
	movl	$-2147483648, %edi              # imm = 0x80000000
	xorl	%esi, %esi
	callq	nonnegative_difference
	movl	%eax, %ebx
	movl	$-2147483648, %edi              # imm = 0x80000000
	xorl	%esi, %esi
	callq	negative_difference
	movl	%eax, %ebp
	movl	$-2147483648, %edi              # imm = 0x80000000
	xorl	%esi, %esi
	callq	positive_difference
	imulq	$1089, %r14, %rcx               # imm = 0x441
	movl	%ebx, %edx
	movq	%rdx, %rsi
	shlq	$5, %rsi
	addq	%rdx, %rsi
	movl	%ebp, %edx
	addq	%rsi, %rdx
	addq	%rcx, %rdx
	movl	%eax, %r14d
	addq	%rdx, %r14
	shlq	$5, %rdx
	addq	%rdx, %r14
	movl	$-2147483648, %edi              # imm = 0x80000000
	movl	$1, %esi
	callq	nonnegative_difference
	movl	%eax, %ebx
	movl	$-2147483648, %edi              # imm = 0x80000000
	movl	$1, %esi
	callq	negative_difference
	movl	%eax, %ebp
	movl	$-2147483648, %edi              # imm = 0x80000000
	movl	$1, %esi
	callq	positive_difference
	imulq	$1089, %r14, %rcx               # imm = 0x441
	movl	%ebx, %edx
	movq	%rdx, %rsi
	shlq	$5, %rsi
	addq	%rdx, %rsi
	movl	%ebp, %edx
	addq	%rsi, %rdx
	addq	%rcx, %rdx
	movl	%eax, %r14d
	addq	%rdx, %r14
	shlq	$5, %rdx
	addq	%rdx, %r14
	movl	$-2147483648, %edi              # imm = 0x80000000
	movl	$2147483646, %esi               # imm = 0x7FFFFFFE
	callq	nonnegative_difference
	movl	%eax, %ebx
	movl	$-2147483648, %edi              # imm = 0x80000000
	movl	$2147483646, %esi               # imm = 0x7FFFFFFE
	callq	negative_difference
	movl	%eax, %ebp
	movl	$-2147483648, %edi              # imm = 0x80000000
	movl	$2147483646, %esi               # imm = 0x7FFFFFFE
	callq	positive_difference
	imulq	$1089, %r14, %rcx               # imm = 0x441
	movl	%ebx, %edx
	movq	%rdx, %rsi
	shlq	$5, %rsi
	addq	%rdx, %rsi
	movl	%ebp, %edx
	addq	%rsi, %rdx
	addq	%rcx, %rdx
	movl	%eax, %r14d
	addq	%rdx, %r14
	shlq	$5, %rdx
	addq	%rdx, %r14
	movl	$-2147483648, %edi              # imm = 0x80000000
	movl	$2147483647, %esi               # imm = 0x7FFFFFFF
	callq	nonnegative_difference
	movl	%eax, %ebx
	movl	$-2147483648, %edi              # imm = 0x80000000
	movl	$2147483647, %esi               # imm = 0x7FFFFFFF
	callq	negative_difference
	movl	%eax, %ebp
	movl	$-2147483648, %edi              # imm = 0x80000000
	movl	$2147483647, %esi               # imm = 0x7FFFFFFF
	callq	positive_difference
	imulq	$1089, %r14, %rcx               # imm = 0x441
	movl	%ebx, %edx
	movq	%rdx, %rsi
	shlq	$5, %rsi
	addq	%rdx, %rsi
	movl	%ebp, %edx
	addq	%rsi, %rdx
	addq	%rcx, %rdx
	movl	%eax, %r14d
	addq	%rdx, %r14
	shlq	$5, %rdx
	addq	%rdx, %r14
	movl	$-2147483647, %edi              # imm = 0x80000001
	movl	$-2147483648, %esi              # imm = 0x80000000
	callq	nonnegative_difference
	movl	%eax, %ebx
	movl	$-2147483647, %edi              # imm = 0x80000001
	movl	$-2147483648, %esi              # imm = 0x80000000
	callq	negative_difference
	movl	%eax, %ebp
	movl	$-2147483647, %edi              # imm = 0x80000001
	movl	$-2147483648, %esi              # imm = 0x80000000
	callq	positive_difference
	imulq	$1089, %r14, %rcx               # imm = 0x441
	movl	%ebx, %edx
	movq	%rdx, %rsi
	shlq	$5, %rsi
	addq	%rdx, %rsi
	movl	%ebp, %edx
	addq	%rsi, %rdx
	addq	%rcx, %rdx
	movl	%eax, %r14d
	addq	%rdx, %r14
	shlq	$5, %rdx
	addq	%rdx, %r14
	movl	$-2147483647, %edi              # imm = 0x80000001
	movl	$-2147483647, %esi              # imm = 0x80000001
	callq	nonnegative_difference
	movl	%eax, %ebx
	movl	$-2147483647, %edi              # imm = 0x80000001
	movl	$-2147483647, %esi              # imm = 0x80000001
	callq	negative_difference
	movl	%eax, %ebp
	movl	$-2147483647, %edi              # imm = 0x80000001
	movl	$-2147483647, %esi              # imm = 0x80000001
	callq	positive_difference
	imulq	$1089, %r14, %rcx               # imm = 0x441
	movl	%ebx, %edx
	movq	%rdx, %rsi
	shlq	$5, %rsi
	addq	%rdx, %rsi
	movl	%ebp, %edx
	addq	%rsi, %rdx
	addq	%rcx, %rdx
	movl	%eax, %r14d
	addq	%rdx, %r14
	shlq	$5, %rdx
	addq	%rdx, %r14
	movl	$-2147483647, %edi              # imm = 0x80000001
	movl	$-1, %esi
	callq	nonnegative_difference
	movl	%eax, %ebx
	movl	$-2147483647, %edi              # imm = 0x80000001
	movl	$-1, %esi
	callq	negative_difference
	movl	%eax, %ebp
	movl	$-2147483647, %edi              # imm = 0x80000001
	movl	$-1, %esi
	callq	positive_difference
	imulq	$1089, %r14, %rcx               # imm = 0x441
	movl	%ebx, %edx
	movq	%rdx, %rsi
	shlq	$5, %rsi
	addq	%rdx, %rsi
	movl	%ebp, %edx
	addq	%rsi, %rdx
	addq	%rcx, %rdx
	movl	%eax, %r14d
	addq	%rdx, %r14
	shlq	$5, %rdx
	addq	%rdx, %r14
	movl	$-2147483647, %edi              # imm = 0x80000001
	xorl	%esi, %esi
	callq	nonnegative_difference
	movl	%eax, %ebx
	movl	$-2147483647, %edi              # imm = 0x80000001
	xorl	%esi, %esi
	callq	negative_difference
	movl	%eax, %ebp
	movl	$-2147483647, %edi              # imm = 0x80000001
	xorl	%esi, %esi
	callq	positive_difference
	imulq	$1089, %r14, %rcx               # imm = 0x441
	movl	%ebx, %edx
	movq	%rdx, %rsi
	shlq	$5, %rsi
	addq	%rdx, %rsi
	movl	%ebp, %edx
	addq	%rsi, %rdx
	addq	%rcx, %rdx
	movl	%eax, %r14d
	addq	%rdx, %r14
	shlq	$5, %rdx
	addq	%rdx, %r14
	movl	$-2147483647, %edi              # imm = 0x80000001
	movl	$1, %esi
	callq	nonnegative_difference
	movl	%eax, %ebx
	movl	$-2147483647, %edi              # imm = 0x80000001
	movl	$1, %esi
	callq	negative_difference
	movl	%eax, %ebp
	movl	$-2147483647, %edi              # imm = 0x80000001
	movl	$1, %esi
	callq	positive_difference
	imulq	$1089, %r14, %rcx               # imm = 0x441
	movl	%ebx, %edx
	movq	%rdx, %rsi
	shlq	$5, %rsi
	addq	%rdx, %rsi
	movl	%ebp, %edx
	addq	%rsi, %rdx
	addq	%rcx, %rdx
	movl	%eax, %r14d
	addq	%rdx, %r14
	shlq	$5, %rdx
	addq	%rdx, %r14
	movl	$-2147483647, %edi              # imm = 0x80000001
	movl	$2147483646, %esi               # imm = 0x7FFFFFFE
	callq	nonnegative_difference
	movl	%eax, %ebx
	movl	$-2147483647, %edi              # imm = 0x80000001
	movl	$2147483646, %esi               # imm = 0x7FFFFFFE
	callq	negative_difference
	movl	%eax, %ebp
	movl	$-2147483647, %edi              # imm = 0x80000001
	movl	$2147483646, %esi               # imm = 0x7FFFFFFE
	callq	positive_difference
	imulq	$1089, %r14, %rcx               # imm = 0x441
	movl	%ebx, %edx
	movq	%rdx, %rsi
	shlq	$5, %rsi
	addq	%rdx, %rsi
	movl	%ebp, %edx
	addq	%rsi, %rdx
	addq	%rcx, %rdx
	movl	%eax, %r14d
	addq	%rdx, %r14
	shlq	$5, %rdx
	addq	%rdx, %r14
	movl	$-2147483647, %edi              # imm = 0x80000001
	movl	$2147483647, %esi               # imm = 0x7FFFFFFF
	callq	nonnegative_difference
	movl	%eax, %ebx
	movl	$-2147483647, %edi              # imm = 0x80000001
	movl	$2147483647, %esi               # imm = 0x7FFFFFFF
	callq	negative_difference
	movl	%eax, %ebp
	movl	$-2147483647, %edi              # imm = 0x80000001
	movl	$2147483647, %esi               # imm = 0x7FFFFFFF
	callq	positive_difference
	imulq	$1089, %r14, %rcx               # imm = 0x441
	movl	%ebx, %edx
	movq	%rdx, %rsi
	shlq	$5, %rsi
	addq	%rdx, %rsi
	movl	%ebp, %edx
	addq	%rsi, %rdx
	addq	%rcx, %rdx
	movl	%eax, %r14d
	addq	%rdx, %r14
	shlq	$5, %rdx
	addq	%rdx, %r14
	movl	$-1, %edi
	movl	$-2147483648, %esi              # imm = 0x80000000
	callq	nonnegative_difference
	movl	%eax, %ebx
	movl	$-1, %edi
	movl	$-2147483648, %esi              # imm = 0x80000000
	callq	negative_difference
	movl	%eax, %ebp
	movl	$-1, %edi
	movl	$-2147483648, %esi              # imm = 0x80000000
	callq	positive_difference
	imulq	$1089, %r14, %rcx               # imm = 0x441
	movl	%ebx, %edx
	movq	%rdx, %rsi
	shlq	$5, %rsi
	addq	%rdx, %rsi
	movl	%ebp, %edx
	addq	%rsi, %rdx
	addq	%rcx, %rdx
	movl	%eax, %r14d
	addq	%rdx, %r14
	shlq	$5, %rdx
	addq	%rdx, %r14
	movl	$-1, %edi
	movl	$-2147483647, %esi              # imm = 0x80000001
	callq	nonnegative_difference
	movl	%eax, %ebx
	movl	$-1, %edi
	movl	$-2147483647, %esi              # imm = 0x80000001
	callq	negative_difference
	movl	%eax, %ebp
	movl	$-1, %edi
	movl	$-2147483647, %esi              # imm = 0x80000001
	callq	positive_difference
	imulq	$1089, %r14, %rcx               # imm = 0x441
	movl	%ebx, %edx
	movq	%rdx, %rsi
	shlq	$5, %rsi
	addq	%rdx, %rsi
	movl	%ebp, %edx
	addq	%rsi, %rdx
	addq	%rcx, %rdx
	movl	%eax, %r14d
	addq	%rdx, %r14
	shlq	$5, %rdx
	addq	%rdx, %r14
	movl	$-1, %edi
	movl	$-1, %esi
	callq	nonnegative_difference
	movl	%eax, %ebx
	movl	$-1, %edi
	movl	$-1, %esi
	callq	negative_difference
	movl	%eax, %ebp
	movl	$-1, %edi
	movl	$-1, %esi
	callq	positive_difference
	imulq	$1089, %r14, %rcx               # imm = 0x441
	movl	%ebx, %edx
	movq	%rdx, %rsi
	shlq	$5, %rsi
	addq	%rdx, %rsi
	movl	%ebp, %edx
	addq	%rsi, %rdx
	addq	%rcx, %rdx
	movl	%eax, %r14d
	addq	%rdx, %r14
	shlq	$5, %rdx
	addq	%rdx, %r14
	movl	$-1, %edi
	xorl	%esi, %esi
	callq	nonnegative_difference
	movl	%eax, %ebx
	movl	$-1, %edi
	xorl	%esi, %esi
	callq	negative_difference
	movl	%eax, %ebp
	movl	$-1, %edi
	xorl	%esi, %esi
	callq	positive_difference
	imulq	$1089, %r14, %rcx               # imm = 0x441
	movl	%ebx, %edx
	movq	%rdx, %rsi
	shlq	$5, %rsi
	addq	%rdx, %rsi
	movl	%ebp, %edx
	addq	%rsi, %rdx
	addq	%rcx, %rdx
	movl	%eax, %r14d
	addq	%rdx, %r14
	shlq	$5, %rdx
	addq	%rdx, %r14
	movl	$-1, %edi
	movl	$1, %esi
	callq	nonnegative_difference
	movl	%eax, %ebx
	movl	$-1, %edi
	movl	$1, %esi
	callq	negative_difference
	movl	%eax, %ebp
	movl	$-1, %edi
	movl	$1, %esi
	callq	positive_difference
	imulq	$1089, %r14, %rcx               # imm = 0x441
	movl	%ebx, %edx
	movq	%rdx, %rsi
	shlq	$5, %rsi
	addq	%rdx, %rsi
	movl	%ebp, %edx
	addq	%rsi, %rdx
	addq	%rcx, %rdx
	movl	%eax, %r14d
	addq	%rdx, %r14
	shlq	$5, %rdx
	addq	%rdx, %r14
	movl	$-1, %edi
	movl	$2147483646, %esi               # imm = 0x7FFFFFFE
	callq	nonnegative_difference
	movl	%eax, %ebx
	movl	$-1, %edi
	movl	$2147483646, %esi               # imm = 0x7FFFFFFE
	callq	negative_difference
	movl	%eax, %ebp
	movl	$-1, %edi
	movl	$2147483646, %esi               # imm = 0x7FFFFFFE
	callq	positive_difference
	imulq	$1089, %r14, %rcx               # imm = 0x441
	movl	%ebx, %edx
	movq	%rdx, %rsi
	shlq	$5, %rsi
	addq	%rdx, %rsi
	movl	%ebp, %edx
	addq	%rsi, %rdx
	addq	%rcx, %rdx
	movl	%eax, %r14d
	addq	%rdx, %r14
	shlq	$5, %rdx
	addq	%rdx, %r14
	movl	$-1, %edi
	movl	$2147483647, %esi               # imm = 0x7FFFFFFF
	callq	nonnegative_difference
	movl	%eax, %ebx
	movl	$-1, %edi
	movl	$2147483647, %esi               # imm = 0x7FFFFFFF
	callq	negative_difference
	movl	%eax, %ebp
	movl	$-1, %edi
	movl	$2147483647, %esi               # imm = 0x7FFFFFFF
	callq	positive_difference
	imulq	$1089, %r14, %rcx               # imm = 0x441
	movl	%ebx, %edx
	movq	%rdx, %rsi
	shlq	$5, %rsi
	addq	%rdx, %rsi
	movl	%ebp, %edx
	addq	%rsi, %rdx
	addq	%rcx, %rdx
	movl	%eax, %r14d
	addq	%rdx, %r14
	shlq	$5, %rdx
	addq	%rdx, %r14
	xorl	%edi, %edi
	movl	$-2147483648, %esi              # imm = 0x80000000
	callq	nonnegative_difference
	movl	%eax, %ebx
	xorl	%edi, %edi
	movl	$-2147483648, %esi              # imm = 0x80000000
	callq	negative_difference
	movl	%eax, %ebp
	xorl	%edi, %edi
	movl	$-2147483648, %esi              # imm = 0x80000000
	callq	positive_difference
	imulq	$1089, %r14, %rcx               # imm = 0x441
	movl	%ebx, %edx
	movq	%rdx, %rsi
	shlq	$5, %rsi
	addq	%rdx, %rsi
	movl	%ebp, %edx
	addq	%rsi, %rdx
	addq	%rcx, %rdx
	movl	%eax, %r14d
	addq	%rdx, %r14
	shlq	$5, %rdx
	addq	%rdx, %r14
	xorl	%edi, %edi
	movl	$-2147483647, %esi              # imm = 0x80000001
	callq	nonnegative_difference
	movl	%eax, %ebx
	xorl	%edi, %edi
	movl	$-2147483647, %esi              # imm = 0x80000001
	callq	negative_difference
	movl	%eax, %ebp
	xorl	%edi, %edi
	movl	$-2147483647, %esi              # imm = 0x80000001
	callq	positive_difference
	imulq	$1089, %r14, %rcx               # imm = 0x441
	movl	%ebx, %edx
	movq	%rdx, %rsi
	shlq	$5, %rsi
	addq	%rdx, %rsi
	movl	%ebp, %edx
	addq	%rsi, %rdx
	addq	%rcx, %rdx
	movl	%eax, %r14d
	addq	%rdx, %r14
	shlq	$5, %rdx
	addq	%rdx, %r14
	xorl	%edi, %edi
	movl	$-1, %esi
	callq	nonnegative_difference
	movl	%eax, %ebx
	xorl	%edi, %edi
	movl	$-1, %esi
	callq	negative_difference
	movl	%eax, %ebp
	xorl	%edi, %edi
	movl	$-1, %esi
	callq	positive_difference
	imulq	$1089, %r14, %rcx               # imm = 0x441
	movl	%ebx, %edx
	movq	%rdx, %rsi
	shlq	$5, %rsi
	addq	%rdx, %rsi
	movl	%ebp, %edx
	addq	%rsi, %rdx
	addq	%rcx, %rdx
	movl	%eax, %r14d
	addq	%rdx, %r14
	shlq	$5, %rdx
	addq	%rdx, %r14
	xorl	%edi, %edi
	xorl	%esi, %esi
	callq	nonnegative_difference
	movl	%eax, %ebx
	xorl	%edi, %edi
	xorl	%esi, %esi
	callq	negative_difference
	movl	%eax, %ebp
	xorl	%edi, %edi
	xorl	%esi, %esi
	callq	positive_difference
	imulq	$1089, %r14, %rcx               # imm = 0x441
	movl	%ebx, %edx
	movq	%rdx, %rsi
	shlq	$5, %rsi
	addq	%rdx, %rsi
	movl	%ebp, %edx
	addq	%rsi, %rdx
	addq	%rcx, %rdx
	movl	%eax, %r14d
	addq	%rdx, %r14
	shlq	$5, %rdx
	addq	%rdx, %r14
	xorl	%edi, %edi
	movl	$1, %esi
	callq	nonnegative_difference
	movl	%eax, %ebx
	xorl	%edi, %edi
	movl	$1, %esi
	callq	negative_difference
	movl	%eax, %ebp
	xorl	%edi, %edi
	movl	$1, %esi
	callq	positive_difference
	imulq	$1089, %r14, %rcx               # imm = 0x441
	movl	%ebx, %edx
	movq	%rdx, %rsi
	shlq	$5, %rsi
	addq	%rdx, %rsi
	movl	%ebp, %edx
	addq	%rsi, %rdx
	addq	%rcx, %rdx
	movl	%eax, %r14d
	addq	%rdx, %r14
	shlq	$5, %rdx
	addq	%rdx, %r14
	xorl	%edi, %edi
	movl	$2147483646, %esi               # imm = 0x7FFFFFFE
	callq	nonnegative_difference
	movl	%eax, %ebx
	xorl	%edi, %edi
	movl	$2147483646, %esi               # imm = 0x7FFFFFFE
	callq	negative_difference
	movl	%eax, %ebp
	xorl	%edi, %edi
	movl	$2147483646, %esi               # imm = 0x7FFFFFFE
	callq	positive_difference
	imulq	$1089, %r14, %rcx               # imm = 0x441
	movl	%ebx, %edx
	movq	%rdx, %rsi
	shlq	$5, %rsi
	addq	%rdx, %rsi
	movl	%ebp, %edx
	addq	%rsi, %rdx
	addq	%rcx, %rdx
	movl	%eax, %r14d
	addq	%rdx, %r14
	shlq	$5, %rdx
	addq	%rdx, %r14
	xorl	%edi, %edi
	movl	$2147483647, %esi               # imm = 0x7FFFFFFF
	callq	nonnegative_difference
	movl	%eax, %ebx
	xorl	%edi, %edi
	movl	$2147483647, %esi               # imm = 0x7FFFFFFF
	callq	negative_difference
	movl	%eax, %ebp
	xorl	%edi, %edi
	movl	$2147483647, %esi               # imm = 0x7FFFFFFF
	callq	positive_difference
	imulq	$1089, %r14, %rcx               # imm = 0x441
	movl	%ebx, %edx
	movq	%rdx, %rsi
	shlq	$5, %rsi
	addq	%rdx, %rsi
	movl	%ebp, %edx
	addq	%rsi, %rdx
	addq	%rcx, %rdx
	movl	%eax, %r14d
	addq	%rdx, %r14
	shlq	$5, %rdx
	addq	%rdx, %r14
	movl	$1, %edi
	movl	$-2147483648, %esi              # imm = 0x80000000
	callq	nonnegative_difference
	movl	%eax, %ebx
	movl	$1, %edi
	movl	$-2147483648, %esi              # imm = 0x80000000
	callq	negative_difference
	movl	%eax, %ebp
	movl	$1, %edi
	movl	$-2147483648, %esi              # imm = 0x80000000
	callq	positive_difference
	imulq	$1089, %r14, %rcx               # imm = 0x441
	movl	%ebx, %edx
	movq	%rdx, %rsi
	shlq	$5, %rsi
	addq	%rdx, %rsi
	movl	%ebp, %edx
	addq	%rsi, %rdx
	addq	%rcx, %rdx
	movl	%eax, %r14d
	addq	%rdx, %r14
	shlq	$5, %rdx
	addq	%rdx, %r14
	movl	$1, %edi
	movl	$-2147483647, %esi              # imm = 0x80000001
	callq	nonnegative_difference
	movl	%eax, %ebx
	movl	$1, %edi
	movl	$-2147483647, %esi              # imm = 0x80000001
	callq	negative_difference
	movl	%eax, %ebp
	movl	$1, %edi
	movl	$-2147483647, %esi              # imm = 0x80000001
	callq	positive_difference
	imulq	$1089, %r14, %rcx               # imm = 0x441
	movl	%ebx, %edx
	movq	%rdx, %rsi
	shlq	$5, %rsi
	addq	%rdx, %rsi
	movl	%ebp, %edx
	addq	%rsi, %rdx
	addq	%rcx, %rdx
	movl	%eax, %r14d
	addq	%rdx, %r14
	shlq	$5, %rdx
	addq	%rdx, %r14
	movl	$1, %edi
	movl	$-1, %esi
	callq	nonnegative_difference
	movl	%eax, %ebx
	movl	$1, %edi
	movl	$-1, %esi
	callq	negative_difference
	movl	%eax, %ebp
	movl	$1, %edi
	movl	$-1, %esi
	callq	positive_difference
	imulq	$1089, %r14, %rcx               # imm = 0x441
	movl	%ebx, %edx
	movq	%rdx, %rsi
	shlq	$5, %rsi
	addq	%rdx, %rsi
	movl	%ebp, %edx
	addq	%rsi, %rdx
	addq	%rcx, %rdx
	movl	%eax, %r14d
	addq	%rdx, %r14
	shlq	$5, %rdx
	addq	%rdx, %r14
	movl	$1, %edi
	xorl	%esi, %esi
	callq	nonnegative_difference
	movl	%eax, %ebx
	movl	$1, %edi
	xorl	%esi, %esi
	callq	negative_difference
	movl	%eax, %ebp
	movl	$1, %edi
	xorl	%esi, %esi
	callq	positive_difference
	imulq	$1089, %r14, %rcx               # imm = 0x441
	movl	%ebx, %edx
	movq	%rdx, %rsi
	shlq	$5, %rsi
	addq	%rdx, %rsi
	movl	%ebp, %edx
	addq	%rsi, %rdx
	addq	%rcx, %rdx
	movl	%eax, %r14d
	addq	%rdx, %r14
	shlq	$5, %rdx
	addq	%rdx, %r14
	movl	$1, %edi
	movl	$1, %esi
	callq	nonnegative_difference
	movl	%eax, %ebx
	movl	$1, %edi
	movl	$1, %esi
	callq	negative_difference
	movl	%eax, %ebp
	movl	$1, %edi
	movl	$1, %esi
	callq	positive_difference
	imulq	$1089, %r14, %rcx               # imm = 0x441
	movl	%ebx, %edx
	movq	%rdx, %rsi
	shlq	$5, %rsi
	addq	%rdx, %rsi
	movl	%ebp, %edx
	addq	%rsi, %rdx
	addq	%rcx, %rdx
	movl	%eax, %r14d
	addq	%rdx, %r14
	shlq	$5, %rdx
	addq	%rdx, %r14
	movl	$1, %edi
	movl	$2147483646, %esi               # imm = 0x7FFFFFFE
	callq	nonnegative_difference
	movl	%eax, %ebx
	movl	$1, %edi
	movl	$2147483646, %esi               # imm = 0x7FFFFFFE
	callq	negative_difference
	movl	%eax, %ebp
	movl	$1, %edi
	movl	$2147483646, %esi               # imm = 0x7FFFFFFE
	callq	positive_difference
	imulq	$1089, %r14, %rcx               # imm = 0x441
	movl	%ebx, %edx
	movq	%rdx, %rsi
	shlq	$5, %rsi
	addq	%rdx, %rsi
	movl	%ebp, %edx
	addq	%rsi, %rdx
	addq	%rcx, %rdx
	movl	%eax, %r14d
	addq	%rdx, %r14
	shlq	$5, %rdx
	addq	%rdx, %r14
	movl	$1, %edi
	movl	$2147483647, %esi               # imm = 0x7FFFFFFF
	callq	nonnegative_difference
	movl	%eax, %ebx
	movl	$1, %edi
	movl	$2147483647, %esi               # imm = 0x7FFFFFFF
	callq	negative_difference
	movl	%eax, %ebp
	movl	$1, %edi
	movl	$2147483647, %esi               # imm = 0x7FFFFFFF
	callq	positive_difference
	imulq	$1089, %r14, %rcx               # imm = 0x441
	movl	%ebx, %edx
	movq	%rdx, %rsi
	shlq	$5, %rsi
	addq	%rdx, %rsi
	movl	%ebp, %edx
	addq	%rsi, %rdx
	addq	%rcx, %rdx
	movl	%eax, %r14d
	addq	%rdx, %r14
	shlq	$5, %rdx
	addq	%rdx, %r14
	movl	$2147483646, %edi               # imm = 0x7FFFFFFE
	movl	$-2147483648, %esi              # imm = 0x80000000
	callq	nonnegative_difference
	movl	%eax, %ebx
	movl	$2147483646, %edi               # imm = 0x7FFFFFFE
	movl	$-2147483648, %esi              # imm = 0x80000000
	callq	negative_difference
	movl	%eax, %ebp
	movl	$2147483646, %edi               # imm = 0x7FFFFFFE
	movl	$-2147483648, %esi              # imm = 0x80000000
	callq	positive_difference
	imulq	$1089, %r14, %rcx               # imm = 0x441
	movl	%ebx, %edx
	movq	%rdx, %rsi
	shlq	$5, %rsi
	addq	%rdx, %rsi
	movl	%ebp, %edx
	addq	%rsi, %rdx
	addq	%rcx, %rdx
	movl	%eax, %r14d
	addq	%rdx, %r14
	shlq	$5, %rdx
	addq	%rdx, %r14
	movl	$2147483646, %edi               # imm = 0x7FFFFFFE
	movl	$-2147483647, %esi              # imm = 0x80000001
	callq	nonnegative_difference
	movl	%eax, %ebx
	movl	$2147483646, %edi               # imm = 0x7FFFFFFE
	movl	$-2147483647, %esi              # imm = 0x80000001
	callq	negative_difference
	movl	%eax, %ebp
	movl	$2147483646, %edi               # imm = 0x7FFFFFFE
	movl	$-2147483647, %esi              # imm = 0x80000001
	callq	positive_difference
	imulq	$1089, %r14, %rcx               # imm = 0x441
	movl	%ebx, %edx
	movq	%rdx, %rsi
	shlq	$5, %rsi
	addq	%rdx, %rsi
	movl	%ebp, %edx
	addq	%rsi, %rdx
	addq	%rcx, %rdx
	movl	%eax, %r14d
	addq	%rdx, %r14
	shlq	$5, %rdx
	addq	%rdx, %r14
	movl	$2147483646, %edi               # imm = 0x7FFFFFFE
	movl	$-1, %esi
	callq	nonnegative_difference
	movl	%eax, %ebx
	movl	$2147483646, %edi               # imm = 0x7FFFFFFE
	movl	$-1, %esi
	callq	negative_difference
	movl	%eax, %ebp
	movl	$2147483646, %edi               # imm = 0x7FFFFFFE
	movl	$-1, %esi
	callq	positive_difference
	imulq	$1089, %r14, %rcx               # imm = 0x441
	movl	%ebx, %edx
	movq	%rdx, %rsi
	shlq	$5, %rsi
	addq	%rdx, %rsi
	movl	%ebp, %edx
	addq	%rsi, %rdx
	addq	%rcx, %rdx
	movl	%eax, %r14d
	addq	%rdx, %r14
	shlq	$5, %rdx
	addq	%rdx, %r14
	movl	$2147483646, %edi               # imm = 0x7FFFFFFE
	xorl	%esi, %esi
	callq	nonnegative_difference
	movl	%eax, %ebx
	movl	$2147483646, %edi               # imm = 0x7FFFFFFE
	xorl	%esi, %esi
	callq	negative_difference
	movl	%eax, %ebp
	movl	$2147483646, %edi               # imm = 0x7FFFFFFE
	xorl	%esi, %esi
	callq	positive_difference
	imulq	$1089, %r14, %rcx               # imm = 0x441
	movl	%ebx, %edx
	movq	%rdx, %rsi
	shlq	$5, %rsi
	addq	%rdx, %rsi
	movl	%ebp, %edx
	addq	%rsi, %rdx
	addq	%rcx, %rdx
	movl	%eax, %r14d
	addq	%rdx, %r14
	shlq	$5, %rdx
	addq	%rdx, %r14
	movl	$2147483646, %edi               # imm = 0x7FFFFFFE
	movl	$1, %esi
	callq	nonnegative_difference
	movl	%eax, %ebx
	movl	$2147483646, %edi               # imm = 0x7FFFFFFE
	movl	$1, %esi
	callq	negative_difference
	movl	%eax, %ebp
	movl	$2147483646, %edi               # imm = 0x7FFFFFFE
	movl	$1, %esi
	callq	positive_difference
	imulq	$1089, %r14, %rcx               # imm = 0x441
	movl	%ebx, %edx
	movq	%rdx, %rsi
	shlq	$5, %rsi
	addq	%rdx, %rsi
	movl	%ebp, %edx
	addq	%rsi, %rdx
	addq	%rcx, %rdx
	movl	%eax, %r14d
	addq	%rdx, %r14
	shlq	$5, %rdx
	addq	%rdx, %r14
	movl	$2147483646, %edi               # imm = 0x7FFFFFFE
	movl	$2147483646, %esi               # imm = 0x7FFFFFFE
	callq	nonnegative_difference
	movl	%eax, %ebx
	movl	$2147483646, %edi               # imm = 0x7FFFFFFE
	movl	$2147483646, %esi               # imm = 0x7FFFFFFE
	callq	negative_difference
	movl	%eax, %ebp
	movl	$2147483646, %edi               # imm = 0x7FFFFFFE
	movl	$2147483646, %esi               # imm = 0x7FFFFFFE
	callq	positive_difference
	imulq	$1089, %r14, %rcx               # imm = 0x441
	movl	%ebx, %edx
	movq	%rdx, %rsi
	shlq	$5, %rsi
	addq	%rdx, %rsi
	movl	%ebp, %edx
	addq	%rsi, %rdx
	addq	%rcx, %rdx
	movl	%eax, %r14d
	addq	%rdx, %r14
	shlq	$5, %rdx
	addq	%rdx, %r14
	movl	$2147483646, %edi               # imm = 0x7FFFFFFE
	movl	$2147483647, %esi               # imm = 0x7FFFFFFF
	callq	nonnegative_difference
	movl	%eax, %ebx
	movl	$2147483646, %edi               # imm = 0x7FFFFFFE
	movl	$2147483647, %esi               # imm = 0x7FFFFFFF
	callq	negative_difference
	movl	%eax, %ebp
	movl	$2147483646, %edi               # imm = 0x7FFFFFFE
	movl	$2147483647, %esi               # imm = 0x7FFFFFFF
	callq	positive_difference
	imulq	$1089, %r14, %rcx               # imm = 0x441
	movl	%ebx, %edx
	movq	%rdx, %rsi
	shlq	$5, %rsi
	addq	%rdx, %rsi
	movl	%ebp, %edx
	addq	%rsi, %rdx
	addq	%rcx, %rdx
	movl	%eax, %r14d
	addq	%rdx, %r14
	shlq	$5, %rdx
	addq	%rdx, %r14
	movl	$2147483647, %edi               # imm = 0x7FFFFFFF
	movl	$-2147483648, %esi              # imm = 0x80000000
	callq	nonnegative_difference
	movl	%eax, %ebx
	movl	$2147483647, %edi               # imm = 0x7FFFFFFF
	movl	$-2147483648, %esi              # imm = 0x80000000
	callq	negative_difference
	movl	%eax, %ebp
	movl	$2147483647, %edi               # imm = 0x7FFFFFFF
	movl	$-2147483648, %esi              # imm = 0x80000000
	callq	positive_difference
	imulq	$1089, %r14, %rcx               # imm = 0x441
	movl	%ebx, %edx
	movq	%rdx, %rsi
	shlq	$5, %rsi
	addq	%rdx, %rsi
	movl	%ebp, %edx
	addq	%rsi, %rdx
	addq	%rcx, %rdx
	movl	%eax, %r14d
	addq	%rdx, %r14
	shlq	$5, %rdx
	addq	%rdx, %r14
	movl	$2147483647, %edi               # imm = 0x7FFFFFFF
	movl	$-2147483647, %esi              # imm = 0x80000001
	callq	nonnegative_difference
	movl	%eax, %ebx
	movl	$2147483647, %edi               # imm = 0x7FFFFFFF
	movl	$-2147483647, %esi              # imm = 0x80000001
	callq	negative_difference
	movl	%eax, %ebp
	movl	$2147483647, %edi               # imm = 0x7FFFFFFF
	movl	$-2147483647, %esi              # imm = 0x80000001
	callq	positive_difference
	imulq	$1089, %r14, %rcx               # imm = 0x441
	movl	%ebx, %edx
	movq	%rdx, %rsi
	shlq	$5, %rsi
	addq	%rdx, %rsi
	movl	%ebp, %edx
	addq	%rsi, %rdx
	addq	%rcx, %rdx
	movl	%eax, %r14d
	addq	%rdx, %r14
	shlq	$5, %rdx
	addq	%rdx, %r14
	movl	$2147483647, %edi               # imm = 0x7FFFFFFF
	movl	$-1, %esi
	callq	nonnegative_difference
	movl	%eax, %ebx
	movl	$2147483647, %edi               # imm = 0x7FFFFFFF
	movl	$-1, %esi
	callq	negative_difference
	movl	%eax, %ebp
	movl	$2147483647, %edi               # imm = 0x7FFFFFFF
	movl	$-1, %esi
	callq	positive_difference
	imulq	$1089, %r14, %rcx               # imm = 0x441
	movl	%ebx, %edx
	movq	%rdx, %rsi
	shlq	$5, %rsi
	addq	%rdx, %rsi
	movl	%ebp, %edx
	addq	%rsi, %rdx
	addq	%rcx, %rdx
	movl	%eax, %r14d
	addq	%rdx, %r14
	shlq	$5, %rdx
	addq	%rdx, %r14
	movl	$2147483647, %edi               # imm = 0x7FFFFFFF
	xorl	%esi, %esi
	callq	nonnegative_difference
	movl	%eax, %ebx
	movl	$2147483647, %edi               # imm = 0x7FFFFFFF
	xorl	%esi, %esi
	callq	negative_difference
	movl	%eax, %ebp
	movl	$2147483647, %edi               # imm = 0x7FFFFFFF
	xorl	%esi, %esi
	callq	positive_difference
	imulq	$1089, %r14, %rcx               # imm = 0x441
	movl	%ebx, %edx
	movq	%rdx, %rsi
	shlq	$5, %rsi
	addq	%rdx, %rsi
	movl	%ebp, %edx
	addq	%rsi, %rdx
	addq	%rcx, %rdx
	movl	%eax, %r14d
	addq	%rdx, %r14
	shlq	$5, %rdx
	addq	%rdx, %r14
	movl	$2147483647, %edi               # imm = 0x7FFFFFFF
	movl	$1, %esi
	callq	nonnegative_difference
	movl	%eax, %ebx
	movl	$2147483647, %edi               # imm = 0x7FFFFFFF
	movl	$1, %esi
	callq	negative_difference
	movl	%eax, %ebp
	movl	$2147483647, %edi               # imm = 0x7FFFFFFF
	movl	$1, %esi
	callq	positive_difference
	imulq	$1089, %r14, %rcx               # imm = 0x441
	movl	%ebx, %edx
	movq	%rdx, %rsi
	shlq	$5, %rsi
	addq	%rdx, %rsi
	movl	%ebp, %edx
	addq	%rsi, %rdx
	addq	%rcx, %rdx
	movl	%eax, %r14d
	addq	%rdx, %r14
	shlq	$5, %rdx
	addq	%rdx, %r14
	movl	$2147483647, %edi               # imm = 0x7FFFFFFF
	movl	$2147483646, %esi               # imm = 0x7FFFFFFE
	callq	nonnegative_difference
	movl	%eax, %ebx
	movl	$2147483647, %edi               # imm = 0x7FFFFFFF
	movl	$2147483646, %esi               # imm = 0x7FFFFFFE
	callq	negative_difference
	movl	%eax, %ebp
	movl	$2147483647, %edi               # imm = 0x7FFFFFFF
	movl	$2147483646, %esi               # imm = 0x7FFFFFFE
	callq	positive_difference
	imulq	$1089, %r14, %rcx               # imm = 0x441
	movl	%ebx, %edx
	movq	%rdx, %rsi
	shlq	$5, %rsi
	addq	%rdx, %rsi
	movl	%ebp, %edx
	addq	%rsi, %rdx
	addq	%rcx, %rdx
	movl	%eax, %r14d
	addq	%rdx, %r14
	shlq	$5, %rdx
	addq	%rdx, %r14
	movl	$2147483647, %edi               # imm = 0x7FFFFFFF
	movl	$2147483647, %esi               # imm = 0x7FFFFFFF
	callq	nonnegative_difference
	movl	%eax, %ebx
	movl	$2147483647, %edi               # imm = 0x7FFFFFFF
	movl	$2147483647, %esi               # imm = 0x7FFFFFFF
	callq	negative_difference
	movl	%eax, %ebp
	movl	$2147483647, %edi               # imm = 0x7FFFFFFF
	movl	$2147483647, %esi               # imm = 0x7FFFFFFF
	callq	positive_difference
	imulq	$1089, %r14, %rcx               # imm = 0x441
	movl	%ebx, %edx
	movq	%rdx, %rsi
	shlq	$5, %rsi
	addq	%rdx, %rsi
	movl	%ebp, %edx
	addq	%rsi, %rdx
	addq	%rcx, %rdx
	movl	%eax, %esi
	addq	%rdx, %rsi
	shlq	$5, %rdx
	addq	%rdx, %rsi
	movl	$.L.str, %edi
	xorl	%eax, %eax
	callq	printf
	xorl	%eax, %eax
	addq	$16, %rsp
	.cfi_def_cfa_offset 32
	popq	%rbx
	.cfi_def_cfa_offset 24
	popq	%r14
	.cfi_def_cfa_offset 16
	popq	%rbp
	.cfi_def_cfa_offset 8
	retq
.Lfunc_end3:
	.size	main, .Lfunc_end3-main
	.cfi_endproc
                                        # -- End function
	.type	.L.str,@object                  #
	.section	.rodata.str1.1,"aMS",@progbits,1
.L.str:
	.asciz	"%llu\n"
	.size	.L.str, 6

	.ident	"Intel(R) oneAPI DPC++/C++ Compiler 2026.2.0 (2026.2.0.20261001)"
	.section	".note.GNU-stack","",@progbits
