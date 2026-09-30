	.file	"zlib_ng_adler32.c"
	.text
	.section	.rodata.str1.1,"aMS",@progbits,1
.LC0:
	.string	"%08x\n"
	.section	.text.startup,"ax",@progbits
	.p2align 4
	.globl	main
	.type	main, @function
main:
.LFB15:
	.cfi_startproc
	pushq	%r15
	.cfi_def_cfa_offset 16
	.cfi_offset 15, -16
	leaq	zlib_ng_adler_data(%rip), %rsi
	movl	$-1640531527, %eax
	pushq	%r14
	.cfi_def_cfa_offset 24
	.cfi_offset 14, -24
	movq	%rsi, %rdx
	pushq	%r13
	.cfi_def_cfa_offset 32
	.cfi_offset 13, -32
	leaq	2097152(%rsi), %r13
	pushq	%r12
	.cfi_def_cfa_offset 40
	.cfi_offset 12, -40
	pushq	%rbp
	.cfi_def_cfa_offset 48
	.cfi_offset 6, -48
	pushq	%rbx
	.cfi_def_cfa_offset 56
	.cfi_offset 3, -56
	subq	$40, %rsp
	.cfi_def_cfa_offset 96
.L2:
	imull	$1103515245, %eax, %eax
	addq	$1, %rdx
	addl	$12345, %eax
	movl	%eax, %ecx
	shrl	$16, %ecx
	movb	%cl, -1(%rdx)
	cmpq	%r13, %rdx
	jne	.L2
	xorl	%r15d, %r15d
	xorl	%r9d, %r9d
	leaq	zlib_ng_adler_data(%rip), %r14
	movl	$2147975281, %r12d
	leaq	2098656(%rsi), %r8
	movl	%r15d, %r10d
.L6:
	movl	%r9d, %r11d
	addl	$1, %r9d
	movq	%r8, 16(%rsp)
	xorl	%esi, %esi
	movl	%r9d, 28(%rsp)
	leaq	5552+zlib_ng_adler_data(%rip), %r15
	movl	%r9d, %ecx
.L4:
	movq	%r15, 8(%rsp)
	leaq	-5552(%r15), %rdx
	.p2align 4
	.p2align 3
.L3:
	movzbl	(%rdx), %eax
	movzbl	1(%rdx), %r15d
	addq	$8, %rdx
	movzbl	-6(%rdx), %ebp
	movzbl	-3(%rdx), %ebx
	addl	%ecx, %eax
	movzbl	-5(%rdx), %ecx
	movzbl	-2(%rdx), %edi
	addl	%eax, %r15d
	addl	%r15d, %ebp
	addl	%r15d, %eax
	leal	(%rcx,%rbp), %r9d
	movzbl	-4(%rdx), %ecx
	addl	%ebp, %eax
	addl	%r9d, %eax
	leal	(%rcx,%r9), %r8d
	movzbl	-1(%rdx), %ecx
	addl	%r8d, %ebx
	addl	%r8d, %eax
	addl	%ebx, %edi
	addl	%ebx, %eax
	addl	%edi, %ecx
	addl	%edi, %eax
	addl	%ecx, %eax
	addl	%eax, %esi
	movq	8(%rsp), %rax
	cmpq	%rax, %rdx
	jne	.L3
	movl	%ecx, %eax
	leaq	5552(%rdx), %r15
	imulq	%r12, %rax
	shrq	$47, %rax
	imull	$65521, %eax, %eax
	subl	%eax, %ecx
	movl	%esi, %eax
	imulq	%r12, %rax
	shrq	$47, %rax
	imull	$65521, %eax, %eax
	subl	%eax, %esi
	movq	16(%rsp), %rax
	cmpq	%rax, %r15
	jne	.L4
	movl	28(%rsp), %r9d
	movl	%r10d, 16(%rsp)
	movq	%rax, %r8
	leaq	2093104+zlib_ng_adler_data(%rip), %rdx
	movl	%r9d, 8(%rsp)
	.p2align 4
	.p2align 3
.L5:
	movzbl	(%rdx), %eax
	movzbl	1(%rdx), %r15d
	addq	$8, %rdx
	movzbl	-6(%rdx), %ebp
	movzbl	-5(%rdx), %ebx
	addl	%ecx, %eax
	movzbl	-4(%rdx), %r10d
	movzbl	-3(%rdx), %r9d
	addl	%eax, %r15d
	movzbl	-2(%rdx), %edi
	movzbl	-1(%rdx), %ecx
	addl	%r15d, %ebp
	addl	%r15d, %eax
	addl	%ebp, %ebx
	addl	%ebp, %eax
	addl	%ebx, %r10d
	addl	%ebx, %eax
	addl	%r10d, %r9d
	addl	%r10d, %eax
	addl	%r9d, %edi
	addl	%r9d, %eax
	addl	%edi, %ecx
	addl	%edi, %eax
	addl	%ecx, %eax
	addl	%eax, %esi
	cmpq	%r13, %rdx
	jne	.L5
	movl	%esi, %edx
	movl	%esi, %eax
	movl	16(%rsp), %r10d
	movl	8(%rsp), %r9d
	imulq	%r12, %rdx
	shrq	$47, %rdx
	imull	$65521, %edx, %edx
	subl	%edx, %eax
	movl	%ecx, %edx
	imulq	%r12, %rdx
	sall	$16, %eax
	shrq	$47, %rdx
	imull	$65521, %edx, %edx
	subl	%edx, %ecx
	orl	%ecx, %eax
	addl	%eax, %r11d
	shrl	$9, %eax
	xorb	%al, (%r14)
	addq	$12289, %r14
	xorl	%r11d, %r10d
	cmpl	$48, %r9d
	jne	.L6
	movl	%r10d, %esi
	leaq	.LC0(%rip), %rdi
	xorl	%eax, %eax
	call	printf@PLT
	addq	$40, %rsp
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
	.cfi_endproc
.LFE15:
	.size	main, .-main
	.local	zlib_ng_adler_data
	.comm	zlib_ng_adler_data,2097152,32
	.ident	"GCC: (Debian 14.2.0-19) 14.2.0"
	.section	.note.GNU-stack,"",@progbits
