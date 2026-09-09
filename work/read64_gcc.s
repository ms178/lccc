	.file	"read64_min.c"
	.text
	.p2align 4
	.globl	f
	.type	f, @function
f:
.LFB1:
	.cfi_startproc
	testq	%rdx, %rdx
	je	.L4
	xorl	%eax, %eax
	xorl	%r8d, %r8d
	.p2align 5
	.p2align 4
	.p2align 3
.L3:
	movq	(%rdi,%rax,8), %rcx
	xorq	(%rsi,%rax,8), %rcx
	addq	$1, %rax
	addq	%rcx, %r8
	cmpq	%rax, %rdx
	jne	.L3
	movq	%r8, %rax
	ret
	.p2align 4,,10
	.p2align 3
.L4:
	xorl	%r8d, %r8d
	movq	%r8, %rax
	ret
	.cfi_endproc
.LFE1:
	.size	f, .-f
	.ident	"GCC: (Debian 14.2.0-19) 14.2.0"
	.section	.note.GNU-stack,"",@progbits
