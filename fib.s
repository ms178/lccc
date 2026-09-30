	.file	"fib.c"
	.text
	.p2align 4
	.globl	fib
	.type	fib, @function
fib:
.LFB11:
	.cfi_startproc
	pushq	%r15
	.cfi_def_cfa_offset 16
	.cfi_offset 15, -16
	pushq	%r14
	.cfi_def_cfa_offset 24
	.cfi_offset 14, -24
	pushq	%r13
	.cfi_def_cfa_offset 32
	.cfi_offset 13, -32
	pushq	%r12
	.cfi_def_cfa_offset 40
	.cfi_offset 12, -40
	pushq	%rbp
	.cfi_def_cfa_offset 48
	.cfi_offset 6, -48
	movslq	%edi, %rbp
	pushq	%rbx
	.cfi_def_cfa_offset 56
	.cfi_offset 3, -56
	subq	$120, %rsp
	.cfi_def_cfa_offset 176
	cmpl	$1, %edi
	jle	.L1
	leal	-1(%rdi), %eax
	movl	%edi, %ebx
	xorl	%ebp, %ebp
	movl	%eax, %edx
	movl	%eax, %r14d
	andl	$-2, %edx
	subl	%edx, %ebx
	movl	%ebx, %r15d
	movq	%rbp, %rbx
	movl	%edi, %ebp
	cmpl	%r15d, %ebp
	je	.L37
.L3:
	subl	$2, %ebp
	movl	%r14d, %edi
	movl	%r15d, 64(%rsp)
	xorl	%r13d, %r13d
	movl	%ebp, %eax
	andl	$-2, %eax
	subl	%eax, %edi
	movl	%r14d, %eax
	movq	%rbx, %r14
	movl	%edi, 68(%rsp)
.L9:
	movl	68(%rsp), %edi
	leal	-1(%rax), %edx
	cmpl	%edi, %eax
	je	.L38
	leal	-2(%rax), %r15d
	movl	%edx, %eax
	xorl	%ebx, %ebx
	movq	%r13, 40(%rsp)
	movl	%r15d, %ecx
	movq	%rbx, %r12
	movl	%ebp, %r13d
	andl	$-2, %ecx
	subl	%ecx, %eax
	movl	%eax, 72(%rsp)
.L13:
	movl	72(%rsp), %eax
	leal	-1(%rdx), %edi
	cmpl	%eax, %edx
	je	.L39
	subl	$2, %edx
	movl	%edi, %eax
	movq	%r12, 48(%rsp)
	movl	%r15d, %ebp
	movl	%edx, %esi
	movl	%r13d, %r15d
	xorl	%ebx, %ebx
	movl	%edx, %ecx
	andl	$-2, %esi
	movq	%r14, %r13
	subl	%esi, %eax
	movl	%eax, 76(%rsp)
.L17:
	movl	76(%rsp), %eax
	leal	-1(%rdi), %r9d
	cmpl	%eax, %edi
	je	.L40
	leal	-2(%rdi), %r8d
	movl	%r9d, %r12d
	movq	%rbx, 56(%rsp)
	xorl	%r14d, %r14d
	movl	%r8d, %edi
	movl	%ebp, %ebx
	movl	%r8d, %r10d
	movl	%r9d, %ebp
	andl	$-2, %edi
	movq	%r13, %r8
	movl	%ecx, %r11d
	movl	%r15d, %r13d
	subl	%edi, %r12d
.L21:
	leal	-1(%rbp), %eax
	cmpl	%r12d, %ebp
	je	.L41
	subl	$2, %ebp
	movl	%eax, %ecx
	xorl	%r15d, %r15d
	movl	%ebp, %edx
	andl	$-2, %edx
	subl	%edx, %ecx
	movl	%ecx, %edi
.L25:
	cmpl	%edi, %eax
	je	.L24
	leal	-2(%rax), %r9d
	leal	-3(%rax), %edx
	subl	$5, %eax
	movl	%r10d, 4(%rsp)
	movl	%r9d, %esi
	movl	%edx, %ecx
	movl	%r9d, 8(%rsp)
	leal	1(%rdx), %r10d
	andl	$-2, %esi
	subl	%esi, %ecx
	movl	%edx, %esi
	andl	$-2, %esi
	movl	%ecx, 32(%rsp)
	subl	%esi, %eax
	xorl	%esi, %esi
	movl	%eax, 12(%rsp)
	cmpl	%edx, 32(%rsp)
	je	.L45
.L34:
	movl	%edx, 36(%rsp)
	movl	%edx, %ecx
	xorl	%r9d, %r9d
	movq	%r8, 16(%rsp)
	movl	%r13d, %r8d
	movl	%ebx, %r13d
	movq	%rsi, 24(%rsp)
	movl	%ebp, %esi
	movl	%edi, %ebp
.L26:
	movl	%ecx, %edx
	cmpl	$1, %ecx
	je	.L42
	xorl	%ebx, %ebx
.L30:
	leal	-1(%rdx), %edi
	movl	%r8d, 108(%rsp)
	movl	%esi, 104(%rsp)
	movl	%r11d, 100(%rsp)
	movl	%ecx, 96(%rsp)
	movq	%r9, 88(%rsp)
	movl	%r10d, 84(%rsp)
	movl	%edx, 80(%rsp)
	call	fib
	movl	80(%rsp), %edx
	movl	84(%rsp), %r10d
	addq	%rax, %rbx
	movq	88(%rsp), %r9
	movl	96(%rsp), %ecx
	subl	$2, %edx
	movl	100(%rsp), %r11d
	movl	104(%rsp), %esi
	cmpl	$1, %edx
	movl	108(%rsp), %r8d
	jg	.L30
	leal	-3(%r10), %edi
	subl	$2, %ecx
	subl	$2, %r10d
	andl	$-2, %edi
	movl	%ecx, %eax
	subl	%edi, %eax
	cltq
	addq	%rbx, %rax
	addq	%rax, %r9
	cmpl	$1, %r10d
	jne	.L26
	.p2align 4
	.p2align 3
.L42:
	movslq	36(%rsp), %rdx
	movl	%ebp, %edi
	movl	%esi, %ebp
	movq	24(%rsp), %rsi
	leaq	1(%r9), %rcx
	movl	%r13d, %ebx
	movl	%r8d, %r13d
	movq	16(%rsp), %r8
	addq	%rcx, %rsi
	leal	-2(%rdx), %ecx
	cmpl	%ecx, 12(%rsp)
	je	.L28
	movl	%ecx, %edx
	leal	1(%rdx), %r10d
	cmpl	%edx, 32(%rsp)
	jne	.L34
.L45:
	movl	4(%rsp), %r10d
	movl	8(%rsp), %r9d
	leaq	1(%rsi), %rdx
.L27:
	movl	%r9d, %eax
	addq	%rdx, %r15
	cmpl	$1, %r9d
	jne	.L25
.L24:
	addq	$1, %r15
	addq	%r15, %r14
	cmpl	$1, %ebp
	jne	.L21
.L41:
	movl	%ebx, %ebp
	movq	56(%rsp), %rbx
	addq	$1, %r14
	movl	%r13d, %r15d
	movl	%r11d, %ecx
	movq	%r8, %r13
	movl	%r10d, %edi
	addq	%r14, %rbx
	cmpl	$1, %r10d
	jne	.L17
.L40:
	movq	48(%rsp), %r12
	leaq	1(%rbx), %rsi
	movq	%r13, %r14
	movl	%ecx, %edx
	movl	%r15d, %r13d
	movl	%ebp, %r15d
	addq	%rsi, %r12
	cmpl	$1, %ecx
	jne	.L13
.L39:
	movl	%r13d, %ebp
	movq	40(%rsp), %r13
	leaq	1(%r12), %rdx
	movl	%r15d, %eax
	addq	%rdx, %r13
	cmpl	$1, %r15d
	jne	.L9
.L38:
	movq	%r14, %rbx
	leaq	1(%r13), %rax
	movl	64(%rsp), %r15d
	addq	%rax, %rbx
	cmpl	$1, %ebp
	je	.L37
	leal	-1(%rbp), %r14d
	cmpl	%r15d, %ebp
	jne	.L3
.L37:
	movq	%rbx, %rbp
	addq	$1, %rbp
.L1:
	addq	$120, %rsp
	.cfi_remember_state
	.cfi_def_cfa_offset 56
	movq	%rbp, %rax
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
.L28:
	.cfi_restore_state
	movl	4(%rsp), %r10d
	movl	8(%rsp), %r9d
	addq	%rsi, %rdx
	jmp	.L27
	.cfi_endproc
.LFE11:
	.size	fib, .-fib
	.section	.rodata.str1.1,"aMS",@progbits,1
.LC0:
	.string	"fib(40) = %ld\n"
	.section	.text.startup,"ax",@progbits
	.p2align 4
	.globl	main
	.type	main, @function
main:
.LFB12:
	.cfi_startproc
	pushq	%r13
	.cfi_def_cfa_offset 16
	.cfi_offset 13, -16
	xorl	%r13d, %r13d
	pushq	%r12
	.cfi_def_cfa_offset 24
	.cfi_offset 12, -24
	movl	$39, %r12d
	pushq	%rbp
	.cfi_def_cfa_offset 32
	.cfi_offset 6, -32
	pushq	%rbx
	.cfi_def_cfa_offset 40
	.cfi_offset 3, -40
	subq	$24, %rsp
	.cfi_def_cfa_offset 64
.L47:
	movl	%r12d, %ebx
	xorl	%ebp, %ebp
.L50:
	leal	-1(%rbx), %edi
	subl	$2, %ebx
	call	fib
	addq	%rax, %rbp
	cmpl	$1, %ebx
	jne	.L50
	subl	$2, %r12d
	leaq	1(%r13,%rbp), %r13
	cmpl	$1, %r12d
	jne	.L47
	addq	$1, %r13
	leaq	.LC0(%rip), %rdi
	xorl	%eax, %eax
	movq	%r13, 8(%rsp)
	movq	8(%rsp), %rsi
	call	printf@PLT
	addq	$24, %rsp
	.cfi_def_cfa_offset 40
	xorl	%eax, %eax
	popq	%rbx
	.cfi_def_cfa_offset 32
	popq	%rbp
	.cfi_def_cfa_offset 24
	popq	%r12
	.cfi_def_cfa_offset 16
	popq	%r13
	.cfi_def_cfa_offset 8
	ret
	.cfi_endproc
.LFE12:
	.size	main, .-main
	.ident	"GCC: (Debian 14.2.0-19) 14.2.0"
	.section	.note.GNU-stack,"",@progbits
