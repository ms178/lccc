	.file	"bit-toggle.c"
	.text
	.p2align 4
	.globl	btog32
	.type	btog32, @function
btog32:
.LFB0:
	.cfi_startproc
	movl	%edi, %eax
	btcl	%esi, %eax
	ret
	.cfi_endproc
.LFE0:
	.size	btog32, .-btog32
	.p2align 4
	.globl	btog64
	.type	btog64, @function
btog64:
.LFB1:
	.cfi_startproc
	movq	%rdi, %rax
	btcq	%rsi, %rax
	ret
	.cfi_endproc
.LFE1:
	.size	btog64, .-btog64
	.p2align 4
	.globl	btog_masked
	.type	btog_masked, @function
btog_masked:
.LFB2:
	.cfi_startproc
	movl	%edi, %eax
	btcl	%esi, %eax
	ret
	.cfi_endproc
.LFE2:
	.size	btog_masked, .-btog_masked
	.p2align 4
	.globl	btog_imm
	.type	btog_imm, @function
btog_imm:
.LFB3:
	.cfi_startproc
	movl	%edi, %eax
	xorl	$32, %eax
	ret
	.cfi_endproc
.LFE3:
	.size	btog_imm, .-btog_imm
	.p2align 4
	.globl	btog_mem
	.type	btog_mem, @function
btog_mem:
.LFB4:
	.cfi_startproc
	movl	%esi, %ecx
	movl	$1, %eax
	sall	%cl, %eax
	xorl	(%rdi), %eax
	ret
	.cfi_endproc
.LFE4:
	.size	btog_mem, .-btog_mem
	.p2align 4
	.globl	btog_twice
	.type	btog_twice, @function
btog_twice:
.LFB5:
	.cfi_startproc
	movl	%edi, %eax
	ret
	.cfi_endproc
.LFE5:
	.size	btog_twice, .-btog_twice
	.p2align 4
	.globl	btog_swapped
	.type	btog_swapped, @function
btog_swapped:
.LFB12:
	.cfi_startproc
	movl	%edi, %eax
	btcl	%esi, %eax
	ret
	.cfi_endproc
.LFE12:
	.size	btog_swapped, .-btog_swapped
	.p2align 4
	.globl	btog_offset
	.type	btog_offset, @function
btog_offset:
.LFB7:
	.cfi_startproc
	addl	$1, %esi
	movl	%edi, %eax
	btcl	%esi, %eax
	ret
	.cfi_endproc
.LFE7:
	.size	btog_offset, .-btog_offset
	.p2align 4
	.globl	keep_xor_two
	.type	keep_xor_two, @function
keep_xor_two:
.LFB8:
	.cfi_startproc
	movl	%esi, %ecx
	movl	$3, %eax
	sall	%cl, %eax
	xorl	%edi, %eax
	ret
	.cfi_endproc
.LFE8:
	.size	keep_xor_two, .-keep_xor_two
	.p2align 4
	.globl	keep_xor_not
	.type	keep_xor_not, @function
keep_xor_not:
.LFB9:
	.cfi_startproc
	btcl	%esi, %edi
	movl	%edi, %eax
	notl	%eax
	ret
	.cfi_endproc
.LFE9:
	.size	keep_xor_not, .-keep_xor_not
	.p2align 4
	.globl	keep_xor_hi
	.type	keep_xor_hi, @function
keep_xor_hi:
.LFB10:
	.cfi_startproc
	movl	%esi, %ecx
	movl	$1, %eax
	salq	%cl, %rax
	xorl	%edi, %eax
	ret
	.cfi_endproc
.LFE10:
	.size	keep_xor_hi, .-keep_xor_hi
	.ident	"GCC: (GNU) 16.2.0"
	.section	.note.GNU-stack,"",@progbits
