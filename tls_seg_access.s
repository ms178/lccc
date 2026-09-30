	.file	"tls_seg_access.c"
	.text
	.p2align 4
	.type	tls_pass, @function
tls_pass:
.LFB11:
	.cfi_startproc
	movl	%edi, %edi
	movl	$2, %eax
	xorl	%esi, %esi
	movq	%fs:0, %r9
	movq	%rdi, %fs:8+tls_slots@tpoff
	movq	%rdi, %rdx
	leaq	tls_slots@tpoff, %r8
	.p2align 6
	.p2align 4
	.p2align 3
.L2:
	movq	%rax, %rcx
	addq	%rdi, %rdx
	andl	$7, %ecx
	movq	%rdx, tls_slots@tpoff(%r9,%rax,8)
	addq	$1, %rax
	addq	$1, %rcx
	movq	%fs:(%r8,%rcx,8), %rcx
	xorq	%rdx, %rcx
	addq	%rcx, %rsi
	cmpq	$65, %rax
	jne	.L2
	leaq	(%rdi,%rsi), %rax
	ret
	.cfi_endproc
.LFE11:
	.size	tls_pass, .-tls_pass
	.section	.rodata.str1.1,"aMS",@progbits,1
.LC0:
	.string	"tls-seg-access: %lu\n"
	.section	.text.startup,"ax",@progbits
	.p2align 4
	.globl	main
	.type	main, @function
main:
.LFB12:
	.cfi_startproc
	subq	$8, %rsp
	.cfi_def_cfa_offset 16
	xorl	%r10d, %r10d
	xorl	%r11d, %r11d
	.p2align 4
	.p2align 3
.L6:
	movl	%r10d, %edi
	addl	$1, %r10d
	orl	$1, %edi
	call	tls_pass
	addq	%rax, %r11
	cmpl	$200000, %r10d
	jne	.L6
	movq	%r11, %rsi
	leaq	.LC0(%rip), %rdi
	xorl	%eax, %eax
	call	printf@PLT
	xorl	%eax, %eax
	addq	$8, %rsp
	.cfi_def_cfa_offset 8
	ret
	.cfi_endproc
.LFE12:
	.size	main, .-main
	.section	.tbss,"awT",@nobits
	.align 8
	.type	tls_slots, @object
	.size	tls_slots, 528
tls_slots:
	.zero	528
	.ident	"GCC: (Debian 14.2.0-19) 14.2.0"
	.section	.note.GNU-stack,"",@progbits
