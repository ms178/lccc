	.file	"k_namechars.c"
	.text
	.section	.rodata.str1.1,"aMS",@progbits,1
.LC0:
	.string	"abcXYZ019_-.<>&\" \t\n"
	.text
	.p2align 4
	.globl	bench_setup
	.type	bench_setup, @function
bench_setup:
.LFB0:
	.cfi_startproc
	movb	$97, s(%rip)
	movl	$1, %edx
	leaq	.LC0(%rip), %rdi
	leaq	s(%rip), %rsi
	.p2align 6
	.p2align 4
	.p2align 3
.L2:
	movslq	%edx, %rax
	movl	%edx, %ecx
	imulq	$1808407283, %rax, %rax
	sarl	$31, %ecx
	sarq	$35, %rax
	subl	%ecx, %eax
	leal	(%rax,%rax,8), %ecx
	leal	(%rax,%rcx,2), %ecx
	movl	%edx, %eax
	subl	%ecx, %eax
	cltq
	movzbl	(%rdi,%rax), %eax
	movb	%al, (%rsi,%rdx)
	addq	$1, %rdx
	cmpq	$4096, %rdx
	jne	.L2
	ret
	.cfi_endproc
.LFE0:
	.size	bench_setup, .-bench_setup
	.p2align 4
	.globl	bench_run
	.type	bench_run, @function
bench_run:
.LFB2:
	.cfi_startproc
	leaq	s(%rip), %rdx
	xorl	%ecx, %ecx
	movabsq	$288230372997595135, %rdi
	leaq	4096(%rdx), %rsi
	jmp	.L10
	.p2align 4,,10
	.p2align 3
.L14:
	cmpb	$46, %al
	jg	.L8
	cmpb	$44, %al
	jle	.L6
.L9:
	addq	$1, %rcx
.L6:
	addq	$1, %rdx
	cmpq	%rsi, %rdx
	je	.L13
.L10:
	movzbl	(%rdx), %eax
	cmpb	$122, %al
	jg	.L6
	cmpb	$64, %al
	jle	.L14
	subl	$65, %eax
	btq	%rax, %rdi
	jc	.L9
	addq	$1, %rdx
	cmpq	%rsi, %rdx
	jne	.L10
.L13:
	movq	%rcx, %rax
	ret
	.p2align 4,,10
	.p2align 3
.L8:
	subl	$48, %eax
	cmpb	$9, %al
	jbe	.L9
	jmp	.L6
	.cfi_endproc
.LFE2:
	.size	bench_run, .-bench_run
	.local	s
	.comm	s,4096,32
	.ident	"GCC: (Debian 14.2.0-19) 14.2.0"
	.section	.note.GNU-stack,"",@progbits
