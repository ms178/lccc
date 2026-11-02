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
                                        # -- End function
	.globl	negative_difference             # -- Begin function negative_difference
	.prefalign	4, .Lfunc_end1, nop
	.type	negative_difference,@function
