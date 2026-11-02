	.att_syntax
	.file	"sign_test_cmov.c"
	.text
	.globl	nonnegative_difference          # -- Begin function nonnegative_difference
	.prefalign	4, .Lfunc_end0, nop
	.type	nonnegative_difference,@function
nonnegative_difference:                 # @nonnegative_difference
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
negative_difference:                    # @negative_difference
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
positive_difference:                    # @positive_difference
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
main:                                   # @main
	.cfi_startproc
# %bb.0:
	pushq	%rbp
	.cfi_def_cfa_offset 16
	pushq	%r15
	.cfi_def_cfa_offset 24
	pushq	%r14
	.cfi_def_cfa_offset 32
	pushq	%r13
	.cfi_def_cfa_offset 40
	pushq	%r12
	.cfi_def_cfa_offset 48
	pushq	%rbx
	.cfi_def_cfa_offset 56
	pushq	%rax
	.cfi_def_cfa_offset 64
	.cfi_offset %rbx, -56
	.cfi_offset %r12, -48
	.cfi_offset %r13, -40
	.cfi_offset %r14, -32
	.cfi_offset %r15, -24
	.cfi_offset %rbp, -16
	xorl	%esi, %esi
	leaq	main.values(%rip), %r14
	xorl	%r15d, %r15d
	.p2align	4
.LBB3_1:                                # =>This Inner Loop Header: Depth=1
	movl	(%r15,%r14), %ebx
	movq	%rsi, %r12
	shlq	$5, %r12
	addq	%rsi, %r12
	movl	%ebx, %edi
	movl	$-2147483648, %esi              # imm = 0x80000000
	callq	nonnegative_difference
	movl	%eax, %r13d
	addq	%r12, %r13
	movq	%r13, %r12
	shlq	$5, %r12
	movl	%ebx, %edi
	movl	$-2147483648, %esi              # imm = 0x80000000
	callq	negative_difference
	movl	%eax, %ebp
	addq	%r13, %rbp
	addq	%r12, %rbp
	movq	%rbp, %r12
	shlq	$5, %r12
	movl	%ebx, %edi
	movl	$-2147483648, %esi              # imm = 0x80000000
	callq	positive_difference
	movl	%eax, %r13d
	addq	%rbp, %r13
	addq	%r12, %r13
	movq	%r13, %r12
	shlq	$5, %r12
	movl	%ebx, %edi
	movl	$-2147483647, %esi              # imm = 0x80000001
	callq	nonnegative_difference
	movl	%eax, %ebp
	addq	%r13, %rbp
	addq	%r12, %rbp
	movq	%rbp, %r12
	shlq	$5, %r12
	movl	%ebx, %edi
	movl	$-2147483647, %esi              # imm = 0x80000001
	callq	negative_difference
	movl	%eax, %r13d
	addq	%rbp, %r13
	addq	%r12, %r13
	movq	%r13, %r12
	shlq	$5, %r12
	movl	%ebx, %edi
	movl	$-2147483647, %esi              # imm = 0x80000001
	callq	positive_difference
	movl	%eax, %ebp
	addq	%r13, %rbp
	addq	%r12, %rbp
	movq	%rbp, %r12
	shlq	$5, %r12
	movl	%ebx, %edi
	movl	$-1, %esi
	callq	nonnegative_difference
	movl	%eax, %r13d
	addq	%rbp, %r13
	addq	%r12, %r13
	movq	%r13, %r12
	shlq	$5, %r12
	movl	%ebx, %edi
	movl	$-1, %esi
	callq	negative_difference
	movl	%eax, %ebp
	addq	%r13, %rbp
	addq	%r12, %rbp
	movq	%rbp, %r12
	shlq	$5, %r12
	movl	%ebx, %edi
	movl	$-1, %esi
	callq	positive_difference
	movl	%eax, %r13d
	addq	%rbp, %r13
	addq	%r12, %r13
	movq	%r13, %r12
	shlq	$5, %r12
	movl	%ebx, %edi
	xorl	%esi, %esi
	callq	nonnegative_difference
	movl	%eax, %ebp
	addq	%r13, %rbp
	addq	%r12, %rbp
	movq	%rbp, %r12
	shlq	$5, %r12
	movl	%ebx, %edi
	xorl	%esi, %esi
	callq	negative_difference
	movl	%eax, %r13d
	addq	%rbp, %r13
	addq	%r12, %r13
	movq	%r13, %r12
	shlq	$5, %r12
	movl	%ebx, %edi
	xorl	%esi, %esi
	callq	positive_difference
	movl	%eax, %ebp
	addq	%r13, %rbp
	addq	%r12, %rbp
	movq	%rbp, %r12
	shlq	$5, %r12
	movl	%ebx, %edi
	movl	$1, %esi
	callq	nonnegative_difference
	movl	%eax, %r13d
	addq	%rbp, %r13
	addq	%r12, %r13
	movq	%r13, %r12
	shlq	$5, %r12
	movl	%ebx, %edi
	movl	$1, %esi
	callq	negative_difference
	movl	%eax, %ebp
	addq	%r13, %rbp
	addq	%r12, %rbp
	movq	%rbp, %r12
	shlq	$5, %r12
	movl	%ebx, %edi
	movl	$1, %esi
	callq	positive_difference
	movl	%eax, %r13d
	addq	%rbp, %r13
	addq	%r12, %r13
	movq	%r13, %r12
	shlq	$5, %r12
	movl	%ebx, %edi
	movl	$2147483646, %esi               # imm = 0x7FFFFFFE
	callq	nonnegative_difference
	movl	%eax, %ebp
	addq	%r13, %rbp
	addq	%r12, %rbp
	movq	%rbp, %r12
	shlq	$5, %r12
	movl	%ebx, %edi
	movl	$2147483646, %esi               # imm = 0x7FFFFFFE
	callq	negative_difference
	movl	%eax, %r13d
	addq	%rbp, %r13
	addq	%r12, %r13
	movq	%r13, %r12
	shlq	$5, %r12
	movl	%ebx, %edi
	movl	$2147483646, %esi               # imm = 0x7FFFFFFE
	callq	positive_difference
	movl	%eax, %ebp
	addq	%r13, %rbp
	addq	%r12, %rbp
	movq	%rbp, %r12
	shlq	$5, %r12
	movl	%ebx, %edi
	movl	$2147483647, %esi               # imm = 0x7FFFFFFF
	callq	nonnegative_difference
	movl	%eax, %r13d
	addq	%rbp, %r13
	addq	%r12, %r13
	movq	%r13, %r12
	shlq	$5, %r12
	movl	%ebx, %edi
	movl	$2147483647, %esi               # imm = 0x7FFFFFFF
	callq	negative_difference
	movl	%eax, %ebp
	addq	%r13, %rbp
	addq	%r12, %rbp
	movq	%rbp, %r12
	shlq	$5, %r12
	movl	%ebx, %edi
	movl	$2147483647, %esi               # imm = 0x7FFFFFFF
	callq	positive_difference
	movl	%eax, %esi
	addq	%rbp, %rsi
	addq	%r12, %rsi
	addq	$4, %r15
	cmpq	$28, %r15
	jne	.LBB3_1
# %bb.2:
	leaq	.L.str(%rip), %rdi
	xorl	%eax, %eax
	callq	printf@PLT
	xorl	%eax, %eax
	addq	$8, %rsp
	.cfi_def_cfa_offset 56
	popq	%rbx
	.cfi_def_cfa_offset 48
	popq	%r12
	.cfi_def_cfa_offset 40
	popq	%r13
	.cfi_def_cfa_offset 32
	popq	%r14
	.cfi_def_cfa_offset 24
	popq	%r15
	.cfi_def_cfa_offset 16
	popq	%rbp
	.cfi_def_cfa_offset 8
	retq
.Lfunc_end3:
	.size	main, .Lfunc_end3-main
	.cfi_endproc
                                        # -- End function
	.type	main.values,@object             # @main.values
	.section	.rodata,"a",@progbits
	.p2align	4, 0x0
main.values:
	.long	2147483648                      # 0x80000000
	.long	2147483649                      # 0x80000001
	.long	4294967295                      # 0xffffffff
	.long	0                               # 0x0
	.long	1                               # 0x1
	.long	2147483646                      # 0x7ffffffe
	.long	2147483647                      # 0x7fffffff
	.size	main.values, 28

	.type	.L.str,@object                  # @.str
	.section	.rodata.str1.1,"aMS",@progbits,1
.L.str:
	.asciz	"%llu\n"
	.size	.L.str, 6

	.ident	"clang version 23.1.3 (https://github.com/llvm/llvm-project 0d261d1ca552c95a8f007e061c787ac7132fbcbc)"
	.section	".note.GNU-stack","",@progbits
	.addrsig
