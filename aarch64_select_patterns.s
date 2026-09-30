	.file	"aarch64_select_patterns.c"
	.text
	.p2align 4
	.type	select_pressure.constprop.0, @function
select_pressure.constprop.0:
.LFB4:
	.cfi_startproc
	testl	%edi, %edi
	je	.L2
	movl	%esi, %edx
	movl	%ecx, %esi
.L2:
	addl	%edx, %esi
	leal	(%rsi,%r8), %eax
	ret
	.cfi_endproc
.LFE4:
	.size	select_pressure.constprop.0, .-select_pressure.constprop.0
	.p2align 4
	.globl	conditional_increment
	.type	conditional_increment, @function
conditional_increment:
.LFB0:
	.cfi_startproc
	cmpl	$1, %esi
	movl	%edi, %eax
	sbbl	$-1, %eax
	ret
	.cfi_endproc
.LFE0:
	.size	conditional_increment, .-conditional_increment
	.p2align 4
	.globl	narrow_high_constant
	.type	narrow_high_constant, @function
narrow_high_constant:
.LFB1:
	.cfi_startproc
	xorl	%eax, %eax
	cmpw	$-2, %di
	sete	%al
	ret
	.cfi_endproc
.LFE1:
	.size	narrow_high_constant, .-narrow_high_constant
	.p2align 4
	.globl	select_pressure
	.type	select_pressure, @function
select_pressure:
.LFB2:
	.cfi_startproc
	testl	%edi, %edi
	je	.L13
	movl	%esi, %edx
	movl	%ecx, %esi
.L13:
	addl	%edx, %esi
	leal	(%rsi,%r9), %eax
	ret
	.cfi_endproc
.LFE2:
	.size	select_pressure, .-select_pressure
	.section	.rodata.str1.1,"aMS",@progbits,1
.LC0:
	.string	"%u %u\n"
	.section	.text.startup,"ax",@progbits
	.p2align 4
	.globl	main
	.type	main, @function
main:
.LFB3:
	.cfi_startproc
	subq	$8, %rsp
	.cfi_def_cfa_offset 16
	xorl	%r10d, %r10d
	xorl	%r8d, %r8d
	xorl	%ecx, %ecx
	movl	$1, %r9d
	.p2align 4
	.p2align 3
.L18:
	imull	$1664525, %r9d, %r9d
	movl	%ecx, %edi
	movl	%r10d, %edx
	addl	$1, %r10d
	addl	$1013904223, %r9d
	movl	%r9d, %esi
	andl	$1, %esi
	call	conditional_increment
	movl	%r9d, %edi
	movl	%r9d, %esi
	movl	%eax, %ecx
	andl	$8, %edi
	call	select_pressure.constprop.0
	addl	%eax, %r8d
	cmpl	$50000000, %r10d
	jne	.L18
	movl	$65534, %edi
	movl	%ecx, %esi
	call	narrow_high_constant
	leaq	.LC0(%rip), %rdi
	xorl	%r8d, %eax
	movl	%eax, %edx
	xorl	%eax, %eax
	call	printf@PLT
	xorl	%eax, %eax
	addq	$8, %rsp
	.cfi_def_cfa_offset 8
	ret
	.cfi_endproc
.LFE3:
	.size	main, .-main
	.ident	"GCC: (Debian 14.2.0-19) 14.2.0"
	.section	.note.GNU-stack,"",@progbits
