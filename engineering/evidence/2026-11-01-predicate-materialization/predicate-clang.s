	.att_syntax
	.file	"predicate-materialize.c"
	.text
	.globl	from_cmp                        # -- Begin function from_cmp
	.prefalign	4, .Lfunc_end0, nop
	.type	from_cmp,@function
from_cmp:                               # @from_cmp
	.cfi_startproc
# %bb.0:
	xorl	%eax, %eax
	cmpl	%esi, %edi
	setne	%al
	retq
.Lfunc_end0:
	.size	from_cmp, .Lfunc_end0-from_cmp
	.cfi_endproc
                                        # -- End function
	.globl	from_cmp_neg                    # -- Begin function from_cmp_neg
	.prefalign	4, .Lfunc_end1, nop
	.type	from_cmp_neg,@function
from_cmp_neg:                           # @from_cmp_neg
	.cfi_startproc
# %bb.0:
	xorl	%eax, %eax
	cmpl	%esi, %edi
	sete	%al
	retq
.Lfunc_end1:
	.size	from_cmp_neg, .Lfunc_end1-from_cmp_neg
	.cfi_endproc
                                        # -- End function
	.globl	from_lt                         # -- Begin function from_lt
	.prefalign	4, .Lfunc_end2, nop
	.type	from_lt,@function
from_lt:                                # @from_lt
	.cfi_startproc
# %bb.0:
	xorl	%eax, %eax
	cmpl	%esi, %edi
	setl	%al
	retq
.Lfunc_end2:
	.size	from_lt, .Lfunc_end2-from_lt
	.cfi_endproc
                                        # -- End function
	.globl	from_lt_neg                     # -- Begin function from_lt_neg
	.prefalign	4, .Lfunc_end3, nop
	.type	from_lt_neg,@function
from_lt_neg:                            # @from_lt_neg
	.cfi_startproc
# %bb.0:
	xorl	%eax, %eax
	cmpl	%esi, %edi
	setge	%al
	retq
.Lfunc_end3:
	.size	from_lt_neg, .Lfunc_end3-from_lt_neg
	.cfi_endproc
                                        # -- End function
	.globl	zero_or_one                     # -- Begin function zero_or_one
	.prefalign	4, .Lfunc_end4, nop
	.type	zero_or_one,@function
zero_or_one:                            # @zero_or_one
	.cfi_startproc
# %bb.0:
	xorl	%eax, %eax
	testl	%edi, %edi
	setne	%al
	retq
.Lfunc_end4:
	.size	zero_or_one, .Lfunc_end4-zero_or_one
	.cfi_endproc
                                        # -- End function
	.globl	one_or_zero                     # -- Begin function one_or_zero
	.prefalign	4, .Lfunc_end5, nop
	.type	one_or_zero,@function
one_or_zero:                            # @one_or_zero
	.cfi_startproc
# %bb.0:
	xorl	%eax, %eax
	testl	%edi, %edi
	sete	%al
	retq
.Lfunc_end5:
	.size	one_or_zero, .Lfunc_end5-one_or_zero
	.cfi_endproc
                                        # -- End function
	.globl	mask_of_sign                    # -- Begin function mask_of_sign
	.prefalign	4, .Lfunc_end6, nop
	.type	mask_of_sign,@function
mask_of_sign:                           # @mask_of_sign
	.cfi_startproc
# %bb.0:
	movl	%edi, %eax
	notl	%eax
	shrl	$31, %eax
	retq
.Lfunc_end6:
	.size	mask_of_sign, .Lfunc_end6-mask_of_sign
	.cfi_endproc
                                        # -- End function
	.globl	zero_or_one64                   # -- Begin function zero_or_one64
	.prefalign	4, .Lfunc_end7, nop
	.type	zero_or_one64,@function
zero_or_one64:                          # @zero_or_one64
	.cfi_startproc
# %bb.0:
	xorl	%eax, %eax
	testq	%rdi, %rdi
	setne	%al
	retq
.Lfunc_end7:
	.size	zero_or_one64, .Lfunc_end7-zero_or_one64
	.cfi_endproc
                                        # -- End function
	.globl	or_or                           # -- Begin function or_or
	.prefalign	4, .Lfunc_end8, nop
	.type	or_or,@function
or_or:                                  # @or_or
	.cfi_startproc
# %bb.0:
	xorl	%eax, %eax
	orl	%esi, %edi
	setne	%al
	retq
.Lfunc_end8:
	.size	or_or, .Lfunc_end8-or_or
	.cfi_endproc
                                        # -- End function
	.globl	and_eq                          # -- Begin function and_eq
	.prefalign	4, .Lfunc_end9, nop
	.type	and_eq,@function
and_eq:                                 # @and_eq
	.cfi_startproc
# %bb.0:
	xorl	%eax, %eax
	orl	%esi, %edi
	sete	%al
	retq
.Lfunc_end9:
	.size	and_eq, .Lfunc_end9-and_eq
	.cfi_endproc
                                        # -- End function
	.globl	or3                             # -- Begin function or3
	.prefalign	4, .Lfunc_end10, nop
	.type	or3,@function
or3:                                    # @or3
	.cfi_startproc
# %bb.0:
	orl	%esi, %edi
	xorl	%eax, %eax
	orl	%edx, %edi
	setne	%al
	retq
.Lfunc_end10:
	.size	or3, .Lfunc_end10-or3
	.cfi_endproc
                                        # -- End function
	.globl	keep_and_ne                     # -- Begin function keep_and_ne
	.prefalign	4, .Lfunc_end11, nop
	.type	keep_and_ne,@function
keep_and_ne:                            # @keep_and_ne
	.cfi_startproc
# %bb.0:
	testl	%edi, %edi
	setne	%al
	testl	%esi, %esi
	setne	%cl
	andb	%al, %cl
	movzbl	%cl, %eax
	retq
.Lfunc_end11:
	.size	keep_and_ne, .Lfunc_end11-keep_and_ne
	.cfi_endproc
                                        # -- End function
	.globl	keep_or_eq                      # -- Begin function keep_or_eq
	.prefalign	4, .Lfunc_end12, nop
	.type	keep_or_eq,@function
keep_or_eq:                             # @keep_or_eq
	.cfi_startproc
# %bb.0:
	testl	%edi, %edi
	sete	%al
	testl	%esi, %esi
	sete	%cl
	orb	%al, %cl
	movzbl	%cl, %eax
	retq
.Lfunc_end12:
	.size	keep_or_eq, .Lfunc_end12-keep_or_eq
	.cfi_endproc
                                        # -- End function
	.globl	keep_xor                        # -- Begin function keep_xor
	.prefalign	4, .Lfunc_end13, nop
	.type	keep_xor,@function
keep_xor:                               # @keep_xor
	.cfi_startproc
# %bb.0:
	testl	%edi, %edi
	setne	%al
	testl	%esi, %esi
	setne	%cl
	xorb	%al, %cl
	movzbl	%cl, %eax
	retq
.Lfunc_end13:
	.size	keep_xor, .Lfunc_end13-keep_xor
	.cfi_endproc
                                        # -- End function
	.globl	keep_three_two                  # -- Begin function keep_three_two
	.prefalign	4, .Lfunc_end14, nop
	.type	keep_three_two,@function
keep_three_two:                         # @keep_three_two
	.cfi_startproc
# %bb.0:
	cmpl	$1, %edi
	movl	$3, %eax
	sbbl	$0, %eax
	retq
.Lfunc_end14:
	.size	keep_three_two, .Lfunc_end14-keep_three_two
	.cfi_endproc
                                        # -- End function
	.globl	keep_one_two                    # -- Begin function keep_one_two
	.prefalign	4, .Lfunc_end15, nop
	.type	keep_one_two,@function
keep_one_two:                           # @keep_one_two
	.cfi_startproc
# %bb.0:
	xorl	%eax, %eax
	cmpl	$1, %edi
	adcl	$1, %eax
	retq
.Lfunc_end15:
	.size	keep_one_two, .Lfunc_end15-keep_one_two
	.cfi_endproc
                                        # -- End function
	.globl	keep_shared                     # -- Begin function keep_shared
	.prefalign	4, .Lfunc_end16, nop
	.type	keep_shared,@function
keep_shared:                            # @keep_shared
	.cfi_startproc
# %bb.0:
	xorl	%eax, %eax
	xorl	%ecx, %ecx
	cmpl	%esi, %edi
	sete	%al
	setne	%cl
	movl	%ecx, g_shared(%rip)
	retq
.Lfunc_end16:
	.size	keep_shared, .Lfunc_end16-keep_shared
	.cfi_endproc
                                        # -- End function
	.type	g_shared,@object                # @g_shared
	.bss
	.globl	g_shared
	.p2align	2, 0x0
g_shared:
	.long	0                               # 0x0
	.size	g_shared, 4

	.ident	"clang version 23.1.4 (https://github.com/llvm/llvm-project 7a941dfb2877a0b8cf30323fefa85ad019e0b0f5)"
	.section	".note.GNU-stack","",@progbits
	.addrsig
