	.file	"bit-selection.c"
	.text
	.p2align 4
	.globl	bset32
	.type	bset32, @function
bset32:
.LFB0:
	.cfi_startproc
	movl	%edi, %eax
	btsl	%esi, %eax
	ret
	.cfi_endproc
.LFE0:
	.size	bset32, .-bset32
	.p2align 4
	.globl	bset64
	.type	bset64, @function
bset64:
.LFB1:
	.cfi_startproc
	movq	%rdi, %rax
	btsq	%rsi, %rax
	ret
	.cfi_endproc
.LFE1:
	.size	bset64, .-bset64
	.p2align 4
	.globl	bset_imm
	.type	bset_imm, @function
bset_imm:
.LFB2:
	.cfi_startproc
	movl	%edi, %eax
	orb	$-128, %al
	ret
	.cfi_endproc
.LFE2:
	.size	bset_imm, .-bset_imm
	.p2align 4
	.globl	bset_masked
	.type	bset_masked, @function
bset_masked:
.LFB3:
	.cfi_startproc
	movl	%edi, %eax
	btsl	%esi, %eax
	ret
	.cfi_endproc
.LFE3:
	.size	bset_masked, .-bset_masked
	.p2align 4
	.globl	bclr32
	.type	bclr32, @function
bclr32:
.LFB4:
	.cfi_startproc
	movl	%edi, %eax
	btrl	%esi, %eax
	ret
	.cfi_endproc
.LFE4:
	.size	bclr32, .-bclr32
	.p2align 4
	.globl	bclr64
	.type	bclr64, @function
bclr64:
.LFB5:
	.cfi_startproc
	movq	%rdi, %rax
	btrq	%rsi, %rax
	ret
	.cfi_endproc
.LFE5:
	.size	bclr64, .-bclr64
	.p2align 4
	.globl	bclr_imm
	.type	bclr_imm, @function
bclr_imm:
.LFB6:
	.cfi_startproc
	movl	%edi, %eax
	andb	$127, %al
	ret
	.cfi_endproc
.LFE6:
	.size	bclr_imm, .-bclr_imm
	.p2align 4
	.globl	btest32
	.type	btest32, @function
btest32:
.LFB7:
	.cfi_startproc
	xorl	%eax, %eax
	btl	%esi, %edi
	setc	%al
	ret
	.cfi_endproc
.LFE7:
	.size	btest32, .-btest32
	.p2align 4
	.globl	btest32z
	.type	btest32z, @function
btest32z:
.LFB8:
	.cfi_startproc
	xorl	%eax, %eax
	btl	%esi, %edi
	setnc	%al
	ret
	.cfi_endproc
.LFE8:
	.size	btest32z, .-btest32z
	.p2align 4
	.globl	btest64
	.type	btest64, @function
btest64:
.LFB9:
	.cfi_startproc
	movl	%esi, %ecx
	shrq	%cl, %rdi
	movl	%edi, %eax
	andl	$1, %eax
	ret
	.cfi_endproc
.LFE9:
	.size	btest64, .-btest64
	.p2align 4
	.globl	btest_imm
	.type	btest_imm, @function
btest_imm:
.LFB10:
	.cfi_startproc
	movl	%edi, %eax
	shrl	$7, %eax
	andl	$1, %eax
	ret
	.cfi_endproc
.LFE10:
	.size	btest_imm, .-btest_imm
	.p2align 4
	.globl	bset_mem
	.type	bset_mem, @function
bset_mem:
.LFB11:
	.cfi_startproc
	movl	$1, %eax
	movl	%esi, %ecx
	sall	%cl, %eax
	orl	%eax, (%rdi)
	ret
	.cfi_endproc
.LFE11:
	.size	bset_mem, .-bset_mem
	.p2align 4
	.globl	bclr_mem
	.type	bclr_mem, @function
bclr_mem:
.LFB12:
	.cfi_startproc
	movl	$-2, %eax
	movl	%esi, %ecx
	roll	%cl, %eax
	andl	%eax, (%rdi)
	ret
	.cfi_endproc
.LFE12:
	.size	bclr_mem, .-bclr_mem
	.p2align 4
	.globl	pop32
	.type	pop32, @function
pop32:
.LFB13:
	.cfi_startproc
	subq	$8, %rsp
	.cfi_def_cfa_offset 16
	movl	%edi, %edi
	call	__popcountdi2
	addq	$8, %rsp
	.cfi_def_cfa_offset 8
	ret
	.cfi_endproc
.LFE13:
	.size	pop32, .-pop32
	.p2align 4
	.globl	pop64
	.type	pop64, @function
pop64:
.LFB14:
	.cfi_startproc
	subq	$8, %rsp
	.cfi_def_cfa_offset 16
	call	__popcountdi2
	addq	$8, %rsp
	.cfi_def_cfa_offset 8
	ret
	.cfi_endproc
.LFE14:
	.size	pop64, .-pop64
	.p2align 4
	.globl	parity32
	.type	parity32, @function
parity32:
.LFB15:
	.cfi_startproc
	movl	%edi, %eax
	shrl	$16, %eax
	xorl	%edi, %eax
	xorb	%ah, %al
	setnp	%al
	movzbl	%al, %eax
	ret
	.cfi_endproc
.LFE15:
	.size	parity32, .-parity32
	.p2align 4
	.globl	parity64
	.type	parity64, @function
parity64:
.LFB16:
	.cfi_startproc
	movq	%rdi, %rax
	shrq	$32, %rax
	xorl	%edi, %eax
	movl	%eax, %edx
	shrl	$16, %edx
	xorl	%edx, %eax
	xorb	%ah, %al
	setnp	%al
	movzbl	%al, %eax
	ret
	.cfi_endproc
.LFE16:
	.size	parity64, .-parity64
	.p2align 4
	.globl	pop_and1
	.type	pop_and1, @function
pop_and1:
.LFB24:
	.cfi_startproc
	movl	%edi, %eax
	shrl	$16, %eax
	xorl	%edi, %eax
	xorb	%ah, %al
	setnp	%al
	movzbl	%al, %eax
	ret
	.cfi_endproc
.LFE24:
	.size	pop_and1, .-pop_and1
	.p2align 4
	.globl	keep_shl
	.type	keep_shl, @function
keep_shl:
.LFB18:
	.cfi_startproc
	movl	%edi, %eax
	movl	%esi, %ecx
	sall	%cl, %eax
	orl	%edi, %eax
	ret
	.cfi_endproc
.LFE18:
	.size	keep_shl, .-keep_shl
	.p2align 4
	.globl	keep_two
	.type	keep_two, @function
keep_two:
.LFB19:
	.cfi_startproc
	movl	%esi, %ecx
	movl	$3, %eax
	sall	%cl, %eax
	andl	%edi, %eax
	ret
	.cfi_endproc
.LFE19:
	.size	keep_two, .-keep_two
	.p2align 4
	.globl	keep_xor
	.type	keep_xor, @function
keep_xor:
.LFB20:
	.cfi_startproc
	movl	%edi, %eax
	btcl	%esi, %eax
	ret
	.cfi_endproc
.LFE20:
	.size	keep_xor, .-keep_xor
	.p2align 4
	.globl	keep_eq4
	.type	keep_eq4, @function
keep_eq4:
.LFB21:
	.cfi_startproc
	movl	%esi, %ecx
	movl	$1, %eax
	sall	%cl, %eax
	andl	%edi, %eax
	cmpl	$4, %eax
	sete	%al
	movzbl	%al, %eax
	ret
	.cfi_endproc
.LFE21:
	.size	keep_eq4, .-keep_eq4
	.p2align 4
	.globl	keep_and
	.type	keep_and, @function
keep_and:
.LFB22:
	.cfi_startproc
	movl	%esi, %ecx
	movl	$1, %eax
	sall	%cl, %eax
	andl	%edi, %eax
	ret
	.cfi_endproc
.LFE22:
	.size	keep_and, .-keep_and
	.globl	__popcountdi2
	.ident	"GCC: (GNU) 16.2.0"
	.section	.note.GNU-stack,"",@progbits
