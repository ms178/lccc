	.att_syntax
	.file	"bit-selection.c"
	.text
	.globl	bset32                          # -- Begin function bset32
	.prefalign	4, .Lfunc_end0, nop
	.type	bset32,@function
bset32:                                 # @bset32
	.cfi_startproc
# %bb.0:
	movl	%edi, %eax
	btsl	%esi, %eax
	retq
.Lfunc_end0:
	.size	bset32, .Lfunc_end0-bset32
	.cfi_endproc
                                        # -- End function
	.globl	bset64                          # -- Begin function bset64
	.prefalign	4, .Lfunc_end1, nop
	.type	bset64,@function
bset64:                                 # @bset64
	.cfi_startproc
# %bb.0:
	movq	%rdi, %rax
	btsq	%rsi, %rax
	retq
.Lfunc_end1:
	.size	bset64, .Lfunc_end1-bset64
	.cfi_endproc
                                        # -- End function
	.globl	bset_imm                        # -- Begin function bset_imm
	.prefalign	4, .Lfunc_end2, nop
	.type	bset_imm,@function
bset_imm:                               # @bset_imm
	.cfi_startproc
# %bb.0:
	movl	%edi, %eax
	orl	$128, %eax
	retq
.Lfunc_end2:
	.size	bset_imm, .Lfunc_end2-bset_imm
	.cfi_endproc
                                        # -- End function
	.globl	bset_masked                     # -- Begin function bset_masked
	.prefalign	4, .Lfunc_end3, nop
	.type	bset_masked,@function
bset_masked:                            # @bset_masked
	.cfi_startproc
# %bb.0:
	movl	%edi, %eax
	btsl	%esi, %eax
	retq
.Lfunc_end3:
	.size	bset_masked, .Lfunc_end3-bset_masked
	.cfi_endproc
                                        # -- End function
	.globl	bclr32                          # -- Begin function bclr32
	.prefalign	4, .Lfunc_end4, nop
	.type	bclr32,@function
bclr32:                                 # @bclr32
	.cfi_startproc
# %bb.0:
	movl	%edi, %eax
	btrl	%esi, %eax
	retq
.Lfunc_end4:
	.size	bclr32, .Lfunc_end4-bclr32
	.cfi_endproc
                                        # -- End function
	.globl	bclr64                          # -- Begin function bclr64
	.prefalign	4, .Lfunc_end5, nop
	.type	bclr64,@function
bclr64:                                 # @bclr64
	.cfi_startproc
# %bb.0:
	movq	%rdi, %rax
	btrq	%rsi, %rax
	retq
.Lfunc_end5:
	.size	bclr64, .Lfunc_end5-bclr64
	.cfi_endproc
                                        # -- End function
	.globl	bclr_imm                        # -- Begin function bclr_imm
	.prefalign	4, .Lfunc_end6, nop
	.type	bclr_imm,@function
bclr_imm:                               # @bclr_imm
	.cfi_startproc
# %bb.0:
	movl	%edi, %eax
	andl	$-129, %eax
	retq
.Lfunc_end6:
	.size	bclr_imm, .Lfunc_end6-bclr_imm
	.cfi_endproc
                                        # -- End function
	.globl	btest32                         # -- Begin function btest32
	.prefalign	4, .Lfunc_end7, nop
	.type	btest32,@function
btest32:                                # @btest32
	.cfi_startproc
# %bb.0:
	xorl	%eax, %eax
	btl	%esi, %edi
	setb	%al
	retq
.Lfunc_end7:
	.size	btest32, .Lfunc_end7-btest32
	.cfi_endproc
                                        # -- End function
	.globl	btest32z                        # -- Begin function btest32z
	.prefalign	4, .Lfunc_end8, nop
	.type	btest32z,@function
btest32z:                               # @btest32z
	.cfi_startproc
# %bb.0:
	xorl	%eax, %eax
	btl	%esi, %edi
	setae	%al
	retq
.Lfunc_end8:
	.size	btest32z, .Lfunc_end8-btest32z
	.cfi_endproc
                                        # -- End function
	.globl	btest64                         # -- Begin function btest64
	.prefalign	4, .Lfunc_end9, nop
	.type	btest64,@function
btest64:                                # @btest64
	.cfi_startproc
# %bb.0:
	xorl	%eax, %eax
	btq	%rsi, %rdi
	setb	%al
	retq
.Lfunc_end9:
	.size	btest64, .Lfunc_end9-btest64
	.cfi_endproc
                                        # -- End function
	.globl	btest_imm                       # -- Begin function btest_imm
	.prefalign	4, .Lfunc_end10, nop
	.type	btest_imm,@function
btest_imm:                              # @btest_imm
	.cfi_startproc
# %bb.0:
	movl	%edi, %eax
	shrl	$7, %eax
	andl	$1, %eax
	retq
.Lfunc_end10:
	.size	btest_imm, .Lfunc_end10-btest_imm
	.cfi_endproc
                                        # -- End function
	.globl	bset_mem                        # -- Begin function bset_mem
	.prefalign	4, .Lfunc_end11, nop
	.type	bset_mem,@function
bset_mem:                               # @bset_mem
	.cfi_startproc
# %bb.0:
	movl	%esi, %ecx
	movl	$1, %eax
                                        # kill: def $cl killed $cl killed $ecx
	shll	%cl, %eax
	orl	%eax, (%rdi)
	retq
.Lfunc_end11:
	.size	bset_mem, .Lfunc_end11-bset_mem
	.cfi_endproc
                                        # -- End function
	.globl	bclr_mem                        # -- Begin function bclr_mem
	.prefalign	4, .Lfunc_end12, nop
	.type	bclr_mem,@function
bclr_mem:                               # @bclr_mem
	.cfi_startproc
# %bb.0:
	movl	%esi, %ecx
	movl	$-2, %eax
                                        # kill: def $cl killed $cl killed $ecx
	roll	%cl, %eax
	andl	%eax, (%rdi)
	retq
.Lfunc_end12:
	.size	bclr_mem, .Lfunc_end12-bclr_mem
	.cfi_endproc
                                        # -- End function
	.globl	pop32                           # -- Begin function pop32
	.prefalign	4, .Lfunc_end13, nop
	.type	pop32,@function
pop32:                                  # @pop32
	.cfi_startproc
# %bb.0:
	popcntl	%edi, %eax
	retq
.Lfunc_end13:
	.size	pop32, .Lfunc_end13-pop32
	.cfi_endproc
                                        # -- End function
	.globl	pop64                           # -- Begin function pop64
	.prefalign	4, .Lfunc_end14, nop
	.type	pop64,@function
pop64:                                  # @pop64
	.cfi_startproc
# %bb.0:
	popcntq	%rdi, %rax
                                        # kill: def $eax killed $eax killed $rax
	retq
.Lfunc_end14:
	.size	pop64, .Lfunc_end14-pop64
	.cfi_endproc
                                        # -- End function
	.globl	parity32                        # -- Begin function parity32
	.prefalign	4, .Lfunc_end15, nop
	.type	parity32,@function
parity32:                               # @parity32
	.cfi_startproc
# %bb.0:
	popcntl	%edi, %eax
	andl	$1, %eax
	retq
.Lfunc_end15:
	.size	parity32, .Lfunc_end15-parity32
	.cfi_endproc
                                        # -- End function
	.globl	parity64                        # -- Begin function parity64
	.prefalign	4, .Lfunc_end16, nop
	.type	parity64,@function
parity64:                               # @parity64
	.cfi_startproc
# %bb.0:
	popcntq	%rdi, %rax
	andl	$1, %eax
                                        # kill: def $eax killed $eax killed $rax
	retq
.Lfunc_end16:
	.size	parity64, .Lfunc_end16-parity64
	.cfi_endproc
                                        # -- End function
	.globl	pop_and1                        # -- Begin function pop_and1
	.prefalign	4, .Lfunc_end17, nop
	.type	pop_and1,@function
pop_and1:                               # @pop_and1
	.cfi_startproc
# %bb.0:
	popcntl	%edi, %eax
	andl	$1, %eax
	retq
.Lfunc_end17:
	.size	pop_and1, .Lfunc_end17-pop_and1
	.cfi_endproc
                                        # -- End function
	.globl	keep_shl                        # -- Begin function keep_shl
	.prefalign	4, .Lfunc_end18, nop
	.type	keep_shl,@function
keep_shl:                               # @keep_shl
	.cfi_startproc
# %bb.0:
	movl	%esi, %ecx
	movl	%edi, %eax
                                        # kill: def $cl killed $cl killed $ecx
	shll	%cl, %eax
	orl	%edi, %eax
	retq
.Lfunc_end18:
	.size	keep_shl, .Lfunc_end18-keep_shl
	.cfi_endproc
                                        # -- End function
	.globl	keep_two                        # -- Begin function keep_two
	.prefalign	4, .Lfunc_end19, nop
	.type	keep_two,@function
keep_two:                               # @keep_two
	.cfi_startproc
# %bb.0:
	movl	%esi, %ecx
	movl	$3, %eax
                                        # kill: def $cl killed $cl killed $ecx
	shll	%cl, %eax
	andl	%edi, %eax
	retq
.Lfunc_end19:
	.size	keep_two, .Lfunc_end19-keep_two
	.cfi_endproc
                                        # -- End function
	.globl	keep_xor                        # -- Begin function keep_xor
	.prefalign	4, .Lfunc_end20, nop
	.type	keep_xor,@function
keep_xor:                               # @keep_xor
	.cfi_startproc
# %bb.0:
	movl	%edi, %eax
	btcl	%esi, %eax
	retq
.Lfunc_end20:
	.size	keep_xor, .Lfunc_end20-keep_xor
	.cfi_endproc
                                        # -- End function
	.globl	keep_eq4                        # -- Begin function keep_eq4
	.prefalign	4, .Lfunc_end21, nop
	.type	keep_eq4,@function
keep_eq4:                               # @keep_eq4
	.cfi_startproc
# %bb.0:
	movl	%esi, %ecx
	movl	$1, %edx
                                        # kill: def $cl killed $cl killed $ecx
	shll	%cl, %edx
	andl	%edi, %edx
	xorl	%eax, %eax
	cmpl	$4, %edx
	sete	%al
	retq
.Lfunc_end21:
	.size	keep_eq4, .Lfunc_end21-keep_eq4
	.cfi_endproc
                                        # -- End function
	.globl	keep_and                        # -- Begin function keep_and
	.prefalign	4, .Lfunc_end22, nop
	.type	keep_and,@function
keep_and:                               # @keep_and
	.cfi_startproc
# %bb.0:
	movl	%esi, %ecx
	movl	$1, %eax
                                        # kill: def $cl killed $cl killed $ecx
	shll	%cl, %eax
	andl	%edi, %eax
	retq
.Lfunc_end22:
	.size	keep_and, .Lfunc_end22-keep_and
	.cfi_endproc
                                        # -- End function
	.ident	"clang version 23.1.4 (https://github.com/llvm/llvm-project 7a941dfb2877a0b8cf30323fefa85ad019e0b0f5)"
	.section	".note.GNU-stack","",@progbits
	.addrsig
