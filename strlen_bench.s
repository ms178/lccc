	.file	"strlen_bench.c"
	.text
	.section	.rodata.str1.8,"aMS",@progbits,1
	.align 8
.LC0:
	.string	"strlen total: %ld, cmp_sum: %ld, found: %ld, copy_sum: %ld\n"
	.section	.text.startup,"ax",@progbits
	.p2align 4
	.globl	main
	.type	main, @function
main:
.LFB24:
	.cfi_startproc
	pushq	%r15
	.cfi_def_cfa_offset 16
	.cfi_offset 15, -16
	leaq	strings(%rip), %r15
	movl	$42, %edx
	xorl	%r9d, %r9d
	pushq	%r14
	.cfi_def_cfa_offset 24
	.cfi_offset 14, -24
	movq	%r15, %r8
	pushq	%r13
	.cfi_def_cfa_offset 32
	.cfi_offset 13, -32
	pushq	%r12
	.cfi_def_cfa_offset 40
	.cfi_offset 12, -40
	pushq	%rbp
	.cfi_def_cfa_offset 48
	.cfi_offset 6, -48
	pushq	%rbx
	.cfi_def_cfa_offset 56
	.cfi_offset 3, -56
	subq	$264, %rsp
	.cfi_def_cfa_offset 320
	.p2align 4
	.p2align 3
.L3:
	imull	$1664525, %edx, %edx
	xorl	%ecx, %ecx
	addl	$1013904223, %edx
	movl	%edx, %eax
	movl	%edx, %edi
	shrl	$2, %eax
	imulq	$381774871, %rax, %rax
	shrq	$34, %rax
	imull	$180, %eax, %eax
	subl	%eax, %edi
	addl	$10, %edi
	.p2align 6
	.p2align 4
	.p2align 3
.L2:
	imull	$1664525, %edx, %edx
	leal	1013904223(%rdx), %eax
	movq	%rax, %rdx
	imulq	$1321528399, %rax, %rax
	movl	%edx, %esi
	shrq	$35, %rax
	imull	$26, %eax, %eax
	subl	%eax, %esi
	leal	97(%rsi), %eax
	movb	%al, (%r8,%rcx)
	addq	$1, %rcx
	cmpl	%ecx, %edi
	jg	.L2
	movslq	%r9d, %rax
	movslq	%edi, %rdi
	addl	$1, %r9d
	addq	$200, %r8
	leaq	(%rax,%rax,4), %rax
	leaq	(%rax,%rax,4), %rax
	leaq	(%r15,%rax,8), %rax
	movb	$0, (%rax,%rdi)
	cmpl	$100000, %r9d
	jne	.L3
	movl	$50, %r14d
	xorl	%r13d, %r13d
	leaq	20000000(%r15), %r12
.L4:
	movq	%r15, %rbx
	.p2align 4
	.p2align 3
.L5:
	movq	%rbx, %rdi
	addq	$200, %rbx
	call	strlen@PLT
	leaq	0(%r13,%rax), %rbp
	movq	%rbp, %r13
	cmpq	%r12, %rbx
	jne	.L5
	subl	$1, %r14d
	jne	.L4
	movq	$0, 8(%rsp)
	leaq	19999800(%r15), %rbx
	leaq	-19999800(%rbx), %r13
	.p2align 4
	.p2align 3
.L7:
	movq	%r13, %rdi
	addq	$200, %r13
	movq	%r13, %rsi
	call	strcmp@PLT
	xorl	%edx, %edx
	testl	%eax, %eax
	setg	%dl
	shrl	$31, %eax
	subl	%eax, %edx
	movslq	%edx, %rax
	addq	%rax, 8(%rsp)
	cmpq	%rbx, %r13
	jne	.L7
	movl	$6513249, 44(%rsp)
	leaq	strings(%rip), %r10
	leaq	44(%rsp), %r9
	movq	$0, 16(%rsp)
	.p2align 4
	.p2align 3
.L17:
	cmpb	$0, (%r10)
	movq	%r10, %r8
	je	.L9
	.p2align 4
	.p2align 3
.L8:
	movzbl	(%r8), %edx
	movq	%r8, %rcx
	movl	$97, %eax
	movq	%r9, %rsi
	testb	%dl, %dl
	jne	.L15
	jmp	.L16
	.p2align 4,,10
	.p2align 3
.L36:
	cmpb	%al, %dl
	jne	.L11
	movzbl	1(%rcx), %edx
	addq	$1, %rcx
	movzbl	1(%rsi), %eax
	leaq	1(%rsi), %rdi
	testb	%dl, %dl
	je	.L11
	movq	%rdi, %rsi
.L15:
	testb	%al, %al
	jne	.L36
.L14:
	addq	$1, 16(%rsp)
.L9:
	addq	$200, %r10
	cmpq	%r12, %r10
	jne	.L17
	movl	$50, 28(%rsp)
	xorl	%r13d, %r13d
	leaq	48(%rsp), %r14
.L18:
	movq	%r15, %rbx
	.p2align 4
	.p2align 3
.L19:
	movq	%rbx, %rdi
	call	strlen@PLT
	movq	%rbx, %rsi
	movq	%r14, %rdi
	addq	$200, %rbx
	leal	1(%rax), %edx
	movslq	%edx, %rdx
	call	memcpy@PLT
	movsbq	48(%rsp), %rax
	addq	%rax, %r13
	cmpq	%r12, %rbx
	jne	.L19
	subl	$1, 28(%rsp)
	jne	.L18
	movq	16(%rsp), %rcx
	movq	8(%rsp), %rdx
	movq	%r13, %r8
	movq	%rbp, %rsi
	leaq	.LC0(%rip), %rdi
	xorl	%eax, %eax
	call	printf@PLT
	addq	$264, %rsp
	.cfi_remember_state
	.cfi_def_cfa_offset 56
	xorl	%eax, %eax
	popq	%rbx
	.cfi_def_cfa_offset 48
	popq	%rbp
	.cfi_def_cfa_offset 40
	popq	%r12
	.cfi_def_cfa_offset 32
	popq	%r13
	.cfi_def_cfa_offset 24
	popq	%r14
	.cfi_def_cfa_offset 16
	popq	%r15
	.cfi_def_cfa_offset 8
	ret
	.p2align 4,,10
	.p2align 3
.L11:
	.cfi_restore_state
	testb	%al, %al
	je	.L14
.L16:
	addq	$1, %r8
	cmpb	$0, (%r8)
	jne	.L8
	jmp	.L9
	.cfi_endproc
.LFE24:
	.size	main, .-main
	.local	strings
	.comm	strings,20000000,32
	.ident	"GCC: (Debian 14.2.0-19) 14.2.0"
	.section	.note.GNU-stack,"",@progbits
