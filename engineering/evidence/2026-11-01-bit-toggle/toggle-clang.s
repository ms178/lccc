	.att_syntax
	.file	"bit-toggle.c"
	.text
	.globl	btog32                          # -- Begin function btog32
	.prefalign	4, .Lfunc_end0, nop
	.type	btog32,@function
btog32:                                 # @btog32
	.cfi_startproc
# %bb.0:
	movl	%edi, %eax
	btcl	%esi, %eax
	retq
.Lfunc_end0:
	.size	btog32, .Lfunc_end0-btog32
	.cfi_endproc
                                        # -- End function
	.globl	btog64                          # -- Begin function btog64
	.prefalign	4, .Lfunc_end1, nop
	.type	btog64,@function
btog64:                                 # @btog64
	.cfi_startproc
# %bb.0:
	movq	%rdi, %rax
	btcq	%rsi, %rax
	retq
.Lfunc_end1:
	.size	btog64, .Lfunc_end1-btog64
	.cfi_endproc
                                        # -- End function
	.globl	btog_masked                     # -- Begin function btog_masked
	.prefalign	4, .Lfunc_end2, nop
	.type	btog_masked,@function
btog_masked:                            # @btog_masked
	.cfi_startproc
# %bb.0:
	movl	%edi, %eax
	btcl	%esi, %eax
	retq
.Lfunc_end2:
	.size	btog_masked, .Lfunc_end2-btog_masked
	.cfi_endproc
                                        # -- End function
	.globl	btog_imm                        # -- Begin function btog_imm
	.prefalign	4, .Lfunc_end3, nop
	.type	btog_imm,@function
btog_imm:                               # @btog_imm
	.cfi_startproc
# %bb.0:
	movl	%edi, %eax
	xorl	$32, %eax
	retq
.Lfunc_end3:
	.size	btog_imm, .Lfunc_end3-btog_imm
	.cfi_endproc
                                        # -- End function
	.globl	btog_mem                        # -- Begin function btog_mem
	.prefalign	4, .Lfunc_end4, nop
	.type	btog_mem,@function
btog_mem:                               # @btog_mem
	.cfi_startproc
# %bb.0:
	movl	(%rdi), %eax
	btcl	%esi, %eax
	retq
.Lfunc_end4:
	.size	btog_mem, .Lfunc_end4-btog_mem
	.cfi_endproc
                                        # -- End function
	.globl	btog_twice                      # -- Begin function btog_twice
	.prefalign	4, .Lfunc_end5, nop
	.type	btog_twice,@function
btog_twice:                             # @btog_twice
	.cfi_startproc
# %bb.0:
	movl	%edi, %eax
	retq
.Lfunc_end5:
	.size	btog_twice, .Lfunc_end5-btog_twice
	.cfi_endproc
                                        # -- End function
	.globl	btog_swapped                    # -- Begin function btog_swapped
	.prefalign	4, .Lfunc_end6, nop
	.type	btog_swapped,@function
btog_swapped:                           # @btog_swapped
	.cfi_startproc
# %bb.0:
	movl	%edi, %eax
	btcl	%esi, %eax
	retq
.Lfunc_end6:
	.size	btog_swapped, .Lfunc_end6-btog_swapped
	.cfi_endproc
                                        # -- End function
	.globl	btog_offset                     # -- Begin function btog_offset
	.prefalign	4, .Lfunc_end7, nop
	.type	btog_offset,@function
btog_offset:                            # @btog_offset
	.cfi_startproc
# %bb.0:
	movl	%edi, %eax
	incb	%sil
	btcl	%esi, %eax
	retq
.Lfunc_end7:
	.size	btog_offset, .Lfunc_end7-btog_offset
	.cfi_endproc
                                        # -- End function
	.globl	keep_xor_two                    # -- Begin function keep_xor_two
	.prefalign	4, .Lfunc_end8, nop
	.type	keep_xor_two,@function
keep_xor_two:                           # @keep_xor_two
	.cfi_startproc
# %bb.0:
	movl	%esi, %ecx
	movl	$3, %eax
                                        # kill: def $cl killed $cl killed $ecx
	shll	%cl, %eax
	xorl	%edi, %eax
	retq
.Lfunc_end8:
	.size	keep_xor_two, .Lfunc_end8-keep_xor_two
	.cfi_endproc
                                        # -- End function
	.globl	keep_xor_not                    # -- Begin function keep_xor_not
	.prefalign	4, .Lfunc_end9, nop
	.type	keep_xor_not,@function
keep_xor_not:                           # @keep_xor_not
	.cfi_startproc
# %bb.0:
	movl	%edi, %eax
	btcl	%esi, %eax
	notl	%eax
	retq
.Lfunc_end9:
	.size	keep_xor_not, .Lfunc_end9-keep_xor_not
	.cfi_endproc
                                        # -- End function
	.globl	keep_xor_hi                     # -- Begin function keep_xor_hi
	.prefalign	4, .Lfunc_end10, nop
	.type	keep_xor_hi,@function
keep_xor_hi:                            # @keep_xor_hi
	.cfi_startproc
# %bb.0:
	movq	%rsi, %rcx
	movl	$1, %eax
                                        # kill: def $cl killed $cl killed $rcx
	shlq	%cl, %rax
	xorl	%edi, %eax
                                        # kill: def $eax killed $eax killed $rax
	retq
.Lfunc_end10:
	.size	keep_xor_hi, .Lfunc_end10-keep_xor_hi
	.cfi_endproc
                                        # -- End function
	.ident	"clang version 23.1.4 (https://github.com/llvm/llvm-project 7a941dfb2877a0b8cf30323fefa85ad019e0b0f5)"
	.section	".note.GNU-stack","",@progbits
	.addrsig
