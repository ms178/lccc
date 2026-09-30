	.file	"linux_find_bit.c"
	.text
	.p2align 4
	.type	linux_find_next_andnot_bit.part.0, @function
linux_find_next_andnot_bit.part.0:
.LFB16:
	.cfi_startproc
	movq	%rsi, %r9
	movq	%rcx, %rsi
	movq	%rdx, %rax
	movl	%ecx, %edx
	shrq	$6, %rsi
	movq	$-1, %rcx
	movq	%rdi, %r8
	shlx	%rdx, %rcx, %rcx
	andq	(%rdi,%rsi,8), %rcx
	movq	(%r9,%rsi,8), %rdi
	andn	%rcx, %rdi, %rdx
	andn	%rcx, %rdi, %rdi
	jne	.L2
	leaq	1(%rsi), %rcx
	movq	%rcx, %rsi
	salq	$6, %rsi
	jmp	.L4
	.p2align 5
	.p2align 4,,10
	.p2align 3
.L16:
	movq	(%r9,%rcx,8), %rdx
	leaq	64(%rsi), %rdi
	andn	(%r8,%rcx,8), %rdx, %rdx
	testq	%rdx, %rdx
	jne	.L5
	movq	%rdi, %rsi
	addq	$1, %rcx
.L4:
	cmpq	%rax, %rsi
	jb	.L16
	ret
.L2:
	salq	$6, %rsi
	.p2align 4
	.p2align 3
.L5:
	testl	%edx, %edx
	je	.L17
	movl	$16, %edi
	xorl	%ecx, %ecx
.L6:
	testw	%dx, %dx
	jne	.L7
	shrq	$16, %rdx
	movl	%edi, %ecx
.L7:
	testb	%dl, %dl
	jne	.L8
	addl	$8, %ecx
	shrq	$8, %rdx
.L8:
	testb	$15, %dl
	jne	.L9
	addl	$4, %ecx
	shrq	$4, %rdx
.L9:
	testb	$3, %dl
	jne	.L10
	addl	$2, %ecx
	shrq	$2, %rdx
.L10:
	andl	$1, %edx
	cmpq	$1, %rdx
	adcl	$0, %ecx
	addq	%rsi, %rcx
	cmpq	%rax, %rcx
	cmovbe	%rcx, %rax
	ret
	.p2align 4,,10
	.p2align 3
.L17:
	shrq	$32, %rdx
	movl	$48, %edi
	movl	$32, %ecx
	jmp	.L6
	.cfi_endproc
.LFE16:
	.size	linux_find_next_andnot_bit.part.0, .-linux_find_next_andnot_bit.part.0
	.section	.rodata.str1.1,"aMS",@progbits,1
.LC2:
	.string	"%lu\n"
	.section	.text.startup,"ax",@progbits
	.p2align 4
	.globl	main
	.type	main, @function
main:
.LFB15:
	.cfi_startproc
	pushq	%r13
	.cfi_def_cfa_offset 16
	.cfi_offset 13, -16
	xorl	%ecx, %ecx
	movl	$192, %edx
	pushq	%r12
	.cfi_def_cfa_offset 24
	.cfi_offset 12, -24
	pushq	%rbp
	.cfi_def_cfa_offset 32
	.cfi_offset 6, -32
	pushq	%rbx
	.cfi_def_cfa_offset 40
	.cfi_offset 3, -40
	subq	$72, %rsp
	.cfi_def_cfa_offset 112
	vmovdqa	.LC0(%rip), %xmm0
	leaq	32(%rsp), %r11
	movq	%rsp, %rdi
	movq	$0, 16(%rsp)
	vmovdqa	%xmm0, (%rsp)
	movq	%r11, %rsi
	vmovdqa	.LC1(%rip), %xmm0
	movq	$-1, 48(%rsp)
	vmovdqa	%xmm0, 32(%rsp)
	call	linux_find_next_andnot_bit.part.0
	cmpq	$5, %rax
	jne	.L21
	movl	$6, %ecx
	movl	$192, %edx
	movq	%r11, %rsi
	movq	%rsp, %rdi
	call	linux_find_next_andnot_bit.part.0
	cmpq	$192, %rax
	jne	.L21
	movl	$7, %edx
	xorl	%eax, %eax
	leaq	linux_bitmap_a(%rip), %r8
	movl	$1, %esi
	leaq	linux_bitmap_b(%rip), %rdi
	.p2align 6
	.p2align 4
	.p2align 3
.L23:
	movq	%rax, %rcx
	movq	$0, (%r8,%rax,8)
	andl	$63, %ecx
	movq	$-1, (%rdi,%rax,8)
	cmpq	$5, %rcx
	jne	.L22
	shlx	%rdx, %rsi, %rcx
	movq	$0, (%rdi,%rax,8)
	movq	%rcx, (%r8,%rax,8)
.L22:
	addq	$1, %rax
	addq	$13, %rdx
	cmpq	$16384, %rax
	jne	.L23
	xorl	%r10d, %r10d
	xorl	%r12d, %r12d
	xorl	%ebp, %ebp
	xorl	%r9d, %r9d
	xorl	%ebx, %ebx
	orq	$-1, %r11
	movl	$1, %r13d
	.p2align 4
	.p2align 3
.L36:
	movl	%ebx, %eax
	andl	$63, %eax
	.p2align 4
	.p2align 3
.L35:
	movq	%rax, %rdx
	shlx	%rax, %r11, %rcx
	shrq	$6, %rdx
	movq	(%rdi,%rdx,8), %rsi
	andq	(%r8,%rdx,8), %rcx
	andn	%rcx, %rsi, %rax
	andn	%rcx, %rsi, %rsi
	jne	.L24
	addq	$1, %rdx
	movq	%rdx, %rcx
	salq	$6, %rcx
	jmp	.L26
	.p2align 6
	.p2align 4,,10
	.p2align 3
.L45:
	movq	(%rdi,%rdx,8), %rax
	leaq	64(%rcx), %rsi
	andn	(%r8,%rdx,8), %rax, %rax
	testq	%rax, %rax
	jne	.L27
	movq	%rsi, %rcx
	addq	$1, %rdx
.L26:
	cmpq	$1048575, %rcx
	jbe	.L45
.L25:
	movzbl	%bpl, %eax
	addl	$1, %ebx
	shlx	%r12, %r13, %rdx
	addq	$13, %rbp
	salq	$6, %rax
	addl	$7, %r12d
	addq	$524288, %r10
	xorq	%rdx, 40(%rdi,%rax,8)
	cmpl	$1024, %ebx
	jne	.L36
	movq	%r9, %rsi
	leaq	.LC2(%rip), %rdi
	xorl	%eax, %eax
	call	printf@PLT
	xorl	%eax, %eax
	jmp	.L18
.L21:
	movl	$2, %eax
.L18:
	addq	$72, %rsp
	.cfi_remember_state
	.cfi_def_cfa_offset 40
	popq	%rbx
	.cfi_def_cfa_offset 32
	popq	%rbp
	.cfi_def_cfa_offset 24
	popq	%r12
	.cfi_def_cfa_offset 16
	popq	%r13
	.cfi_def_cfa_offset 8
	ret
.L24:
	.cfi_restore_state
	movq	%rdx, %rcx
	salq	$6, %rcx
	.p2align 4
	.p2align 3
.L27:
	testl	%eax, %eax
	jne	.L38
	shrq	$32, %rax
	movl	$48, %esi
	movl	$32, %edx
.L28:
	testw	%ax, %ax
	jne	.L29
	shrq	$16, %rax
	movl	%esi, %edx
.L29:
	testb	%al, %al
	jne	.L30
	addl	$8, %edx
	shrq	$8, %rax
.L30:
	testb	$15, %al
	jne	.L31
	addl	$4, %edx
	shrq	$4, %rax
.L31:
	testb	$3, %al
	jne	.L32
	addl	$2, %edx
	shrq	$2, %rax
.L32:
	testb	$1, %al
	je	.L33
	movl	%edx, %eax
	movl	$1048576, %edx
	addq	%rcx, %rax
	cmpq	%rdx, %rax
	cmova	%rdx, %rax
	leaq	(%rax,%r10), %rdx
	addq	$1, %rax
	xorq	%rdx, %r9
	jmp	.L35
	.p2align 4,,10
	.p2align 3
.L33:
	addl	$1, %edx
	movl	$1048576, %eax
	addq	%rcx, %rdx
	cmpq	%rax, %rdx
	cmova	%rax, %rdx
	leaq	(%r10,%rdx), %rax
	xorq	%rax, %r9
	leaq	1(%rdx), %rax
	cmpq	$1048575, %rdx
	jne	.L35
	jmp	.L25
	.p2align 4,,10
	.p2align 3
.L38:
	movl	$16, %esi
	xorl	%edx, %edx
	jmp	.L28
	.cfi_endproc
.LFE15:
	.size	main, .-main
	.local	linux_bitmap_b
	.comm	linux_bitmap_b,131072,32
	.local	linux_bitmap_a
	.comm	linux_bitmap_a,131072,32
	.section	.rodata.cst16,"aM",@progbits,16
	.align 16
.LC0:
	.quad	32
	.quad	4
	.align 16
.LC1:
	.quad	0
	.quad	-1
	.ident	"GCC: (Debian 14.2.0-19) 14.2.0"
	.section	.note.GNU-stack,"",@progbits
