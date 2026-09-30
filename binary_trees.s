	.file	"binary_trees.c"
	.text
	.p2align 4
	.type	check, @function
check:
.LFB23:
	.cfi_startproc
	pushq	%r15
	.cfi_def_cfa_offset 16
	.cfi_offset 15, -16
	movl	$1, %eax
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
	pushq	%rbx
	.cfi_def_cfa_offset 56
	.cfi_offset 3, -56
	subq	$72, %rsp
	.cfi_def_cfa_offset 128
	movq	(%rdi), %r12
	testq	%r12, %r12
	je	.L1
	xorl	%r15d, %r15d
	movq	%r12, %r14
	movl	%r15d, %r12d
	movq	%rdi, %r15
.L7:
	movq	(%r14), %rbx
	movl	$2, %r13d
	testq	%rbx, %rbx
	je	.L3
	movl	%r12d, %eax
	movq	%r15, %rbp
	xorl	%r13d, %r13d
	movq	%rbx, %r12
	movq	%r14, %r15
.L4:
	movq	(%r12), %rbx
	movl	$2, %edx
	testq	%rbx, %rbx
	je	.L5
	xorl	%edx, %edx
	movl	%eax, %ecx
	movl	%r13d, %eax
	movq	%rbp, %r13
	movl	%edx, %r14d
	movq	%rbx, %rbp
.L6:
	movq	0(%rbp), %rbx
	movl	$2, %edx
	testq	%rbx, %rbx
	je	.L8
	movq	%r15, 24(%rsp)
	xorl	%edx, %edx
	movq	%r13, %rsi
	movl	%ecx, %r8d
	movl	%edx, %r13d
	movl	%eax, %ecx
	movq	%rsi, %rdx
	movl	%r14d, %eax
	movq	%rbp, %r14
	movq	%rbx, %rbp
.L9:
	movq	0(%rbp), %r15
	movl	$2, %ebx
	testq	%r15, %r15
	je	.L10
	xorl	%ebx, %ebx
	movl	%eax, %r10d
	movq	%rbp, %rax
	movl	%r8d, %r11d
	movq	%r12, %rbp
	movl	%r13d, %r8d
	movl	%ebx, %r9d
	movq	%r14, %r13
	movq	%rax, %r12
.L11:
	movq	(%r15), %r14
	movl	$2, %ebx
	testq	%r14, %r14
	je	.L12
	movl	%r11d, %eax
	xorl	%ebx, %ebx
	movq	%r14, %r11
.L13:
	movq	(%r11), %r14
	movl	$2, %edi
	testq	%r14, %r14
	je	.L14
	movq	%rdx, (%rsp)
	xorl	%edi, %edi
	movl	%eax, %esi
	movq	%r11, 8(%rsp)
	movq	%r14, %r11
	movl	%ebx, 16(%rsp)
	movl	%edi, %ebx
.L15:
	movq	(%r11), %r14
	movl	$2, %edi
	testq	%r14, %r14
	je	.L16
	movl	%esi, 20(%rsp)
	xorl	%edx, %edx
	movl	%ebx, %esi
	movq	%r14, %rbx
.L17:
	movq	(%rbx), %rdi
	movl	$2, %r14d
	testq	%rdi, %rdi
	je	.L18
	xorl	%r14d, %r14d
.L19:
	movl	%edx, 60(%rsp)
	movl	%esi, 56(%rsp)
	movl	%r9d, 52(%rsp)
	movl	%r8d, 48(%rsp)
	movl	%r10d, 44(%rsp)
	movq	%r11, 32(%rsp)
	movl	%ecx, 40(%rsp)
	call	check
	movq	8(%rbx), %rbx
	movl	40(%rsp), %ecx
	movq	32(%rsp), %r11
	movl	44(%rsp), %r10d
	leal	1(%r14,%rax), %r14d
	movq	(%rbx), %rdi
	movl	48(%rsp), %r8d
	movl	52(%rsp), %r9d
	movl	56(%rsp), %esi
	testq	%rdi, %rdi
	movl	60(%rsp), %edx
	jne	.L19
	addl	$2, %r14d
.L18:
	movq	8(%r11), %r11
	addl	%r14d, %edx
	movq	(%r11), %rbx
	testq	%rbx, %rbx
	jne	.L17
	movl	%esi, %ebx
	movl	20(%rsp), %esi
	leal	2(%rdx), %edi
.L16:
	movq	8(%rsp), %rax
	addl	%edi, %ebx
	movq	8(%rax), %rax
	movq	(%rax), %r11
	movq	%rax, 8(%rsp)
	testq	%r11, %r11
	jne	.L15
	movl	%ebx, %edi
	movq	(%rsp), %rdx
	movl	16(%rsp), %ebx
	movl	%esi, %eax
	addl	$2, %edi
.L14:
	movq	8(%r15), %r15
	addl	%edi, %ebx
	movq	(%r15), %r11
	testq	%r11, %r11
	jne	.L13
	movl	%eax, %r11d
	addl	$2, %ebx
.L12:
	movq	8(%r12), %r12
	addl	%ebx, %r9d
	movq	(%r12), %r15
	testq	%r15, %r15
	jne	.L11
	movl	%r9d, %ebx
	movq	%r13, %r14
	movq	%rbp, %r12
	movl	%r8d, %r13d
	movl	%r10d, %eax
	movl	%r11d, %r8d
	addl	$2, %ebx
.L10:
	movq	8(%r14), %r14
	addl	%ebx, %r13d
	movq	(%r14), %rbp
	testq	%rbp, %rbp
	jne	.L9
	movq	%rdx, %rsi
	movq	24(%rsp), %r15
	movl	%r13d, %edx
	movl	%eax, %r14d
	movq	%rsi, %r13
	movl	%ecx, %eax
	addl	$2, %edx
	movl	%r8d, %ecx
.L8:
	movq	8(%r12), %r12
	addl	%edx, %r14d
	movq	(%r12), %rbp
	testq	%rbp, %rbp
	jne	.L6
	movl	%r14d, %edx
	movq	%r13, %rbp
	movl	%eax, %r13d
	movl	%ecx, %eax
	addl	$2, %edx
.L5:
	movq	8(%r15), %r15
	addl	%edx, %r13d
	movq	(%r15), %r12
	testq	%r12, %r12
	jne	.L4
	movl	%eax, %r12d
	movq	%rbp, %r15
	addl	$2, %r13d
.L3:
	movq	8(%r15), %r15
	addl	%r13d, %r12d
	movq	(%r15), %r14
	testq	%r14, %r14
	jne	.L7
	leal	1(%r12), %eax
.L1:
	addq	$72, %rsp
	.cfi_def_cfa_offset 56
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
.LFE23:
	.size	check, .-check
	.p2align 4
	.type	destroy, @function
destroy:
.LFB24:
	.cfi_startproc
	pushq	%r13
	.cfi_def_cfa_offset 16
	.cfi_offset 13, -16
	pushq	%r12
	.cfi_def_cfa_offset 24
	.cfi_offset 12, -24
	pushq	%rbp
	.cfi_def_cfa_offset 32
	.cfi_offset 6, -32
	pushq	%rbx
	.cfi_def_cfa_offset 40
	.cfi_offset 3, -40
	movq	%rdi, %rbx
	subq	$8, %rsp
	.cfi_def_cfa_offset 48
	movq	(%rdi), %rbp
	testq	%rbp, %rbp
	je	.L51
	movq	0(%rbp), %r12
	testq	%r12, %r12
	je	.L52
	movq	(%r12), %r13
	testq	%r13, %r13
	je	.L53
	movq	0(%r13), %rdi
	testq	%rdi, %rdi
	je	.L54
	call	destroy
	movq	8(%r13), %rdi
	call	destroy
.L54:
	movq	%r13, %rdi
	call	free@PLT
	movq	8(%r12), %r13
	movq	0(%r13), %rdi
	testq	%rdi, %rdi
	je	.L55
	call	destroy
	movq	8(%r13), %rdi
	call	destroy
.L55:
	movq	%r13, %rdi
	call	free@PLT
.L53:
	movq	%r12, %rdi
	call	free@PLT
	movq	8(%rbp), %r12
	movq	(%r12), %r13
	testq	%r13, %r13
	je	.L56
	movq	0(%r13), %rdi
	testq	%rdi, %rdi
	je	.L57
	call	destroy
	movq	8(%r13), %rdi
	call	destroy
.L57:
	movq	%r13, %rdi
	call	free@PLT
	movq	8(%r12), %r13
	movq	0(%r13), %rdi
	testq	%rdi, %rdi
	je	.L58
	call	destroy
	movq	8(%r13), %rdi
	call	destroy
.L58:
	movq	%r13, %rdi
	call	free@PLT
.L56:
	movq	%r12, %rdi
	call	free@PLT
.L52:
	movq	%rbp, %rdi
	call	free@PLT
	movq	8(%rbx), %rbp
	movq	0(%rbp), %r12
	testq	%r12, %r12
	je	.L59
	movq	(%r12), %r13
	testq	%r13, %r13
	je	.L60
	movq	0(%r13), %rdi
	testq	%rdi, %rdi
	je	.L61
	call	destroy
	movq	8(%r13), %rdi
	call	destroy
.L61:
	movq	%r13, %rdi
	call	free@PLT
	movq	8(%r12), %r13
	movq	0(%r13), %rdi
	testq	%rdi, %rdi
	je	.L62
	call	destroy
	movq	8(%r13), %rdi
	call	destroy
.L62:
	movq	%r13, %rdi
	call	free@PLT
.L60:
	movq	%r12, %rdi
	call	free@PLT
	movq	8(%rbp), %r12
	movq	(%r12), %r13
	testq	%r13, %r13
	je	.L63
	movq	0(%r13), %rdi
	testq	%rdi, %rdi
	je	.L64
	call	destroy
	movq	8(%r13), %rdi
	call	destroy
.L64:
	movq	%r13, %rdi
	call	free@PLT
	movq	8(%r12), %r13
	movq	0(%r13), %rdi
	testq	%rdi, %rdi
	je	.L65
	call	destroy
	movq	8(%r13), %rdi
	call	destroy
.L65:
	movq	%r13, %rdi
	call	free@PLT
.L63:
	movq	%r12, %rdi
	call	free@PLT
.L59:
	movq	%rbp, %rdi
	call	free@PLT
.L51:
	addq	$8, %rsp
	.cfi_def_cfa_offset 40
	movq	%rbx, %rdi
	popq	%rbx
	.cfi_def_cfa_offset 32
	popq	%rbp
	.cfi_def_cfa_offset 24
	popq	%r12
	.cfi_def_cfa_offset 16
	popq	%r13
	.cfi_def_cfa_offset 8
	jmp	free@PLT
	.cfi_endproc
.LFE24:
	.size	destroy, .-destroy
	.p2align 4
	.type	make, @function
make:
.LFB22:
	.cfi_startproc
	pushq	%r13
	.cfi_def_cfa_offset 16
	.cfi_offset 13, -16
	pushq	%r12
	.cfi_def_cfa_offset 24
	.cfi_offset 12, -24
	movl	%edi, %r12d
	movl	$16, %edi
	pushq	%rbp
	.cfi_def_cfa_offset 32
	.cfi_offset 6, -32
	pushq	%rbx
	.cfi_def_cfa_offset 40
	.cfi_offset 3, -40
	subq	$8, %rsp
	.cfi_def_cfa_offset 48
	call	malloc@PLT
	movq	%rax, %rbx
	testl	%r12d, %r12d
	jne	.L120
	vpxor	%xmm0, %xmm0, %xmm0
	vmovdqu	%xmm0, (%rax)
	addq	$8, %rsp
	.cfi_remember_state
	.cfi_def_cfa_offset 40
	movq	%rbx, %rax
	popq	%rbx
	.cfi_def_cfa_offset 32
	popq	%rbp
	.cfi_def_cfa_offset 24
	popq	%r12
	.cfi_def_cfa_offset 16
	popq	%r13
	.cfi_def_cfa_offset 8
	ret
	.p2align 4,,10
	.p2align 3
.L120:
	.cfi_restore_state
	movl	$16, %edi
	call	malloc@PLT
	movq	%rax, %rbp
	cmpl	$1, %r12d
	jne	.L121
	movq	%rax, (%rbx)
	vpxor	%xmm0, %xmm0, %xmm0
	movl	$16, %edi
	vmovdqu	%xmm0, (%rax)
	call	malloc@PLT
	vpxor	%xmm0, %xmm0, %xmm0
	movq	%rax, %r13
	vmovdqu	%xmm0, (%rax)
.L117:
	movq	%r13, 8(%rbx)
	addq	$8, %rsp
	.cfi_remember_state
	.cfi_def_cfa_offset 40
	movq	%rbx, %rax
	popq	%rbx
	.cfi_def_cfa_offset 32
	popq	%rbp
	.cfi_def_cfa_offset 24
	popq	%r12
	.cfi_def_cfa_offset 16
	popq	%r13
	.cfi_def_cfa_offset 8
	ret
	.p2align 4,,10
	.p2align 3
.L121:
	.cfi_restore_state
	movl	$16, %edi
	call	malloc@PLT
	movq	%rax, %r13
	cmpl	$2, %r12d
	jne	.L122
	movq	%rax, 0(%rbp)
	vpxor	%xmm1, %xmm1, %xmm1
	movl	$16, %edi
	vmovdqu	%xmm1, (%rax)
	call	malloc@PLT
	movq	%rbp, (%rbx)
	vpxor	%xmm1, %xmm1, %xmm1
	movl	$16, %edi
	movq	%rax, 8(%rbp)
	vmovdqu	%xmm1, (%rax)
	call	malloc@PLT
	movl	$16, %edi
	movq	%rax, %r13
	call	malloc@PLT
	vpxor	%xmm1, %xmm1, %xmm1
	movl	$16, %edi
	movq	%rax, 0(%r13)
	vmovdqu	%xmm1, (%rax)
	call	malloc@PLT
	vpxor	%xmm1, %xmm1, %xmm1
	movq	%rax, %rbp
	vmovdqu	%xmm1, (%rax)
.L116:
	movq	%rbp, 8(%r13)
	jmp	.L117
	.p2align 4,,10
	.p2align 3
.L122:
	subl	$3, %r12d
	movl	%r12d, %edi
	call	make
	movl	%r12d, %edi
	movq	%rax, 0(%r13)
	call	make
	movq	%r13, 0(%rbp)
	movl	$16, %edi
	movq	%rax, 8(%r13)
	call	malloc@PLT
	movl	%r12d, %edi
	movq	%rax, %r13
	call	make
	movl	%r12d, %edi
	movq	%rax, 0(%r13)
	call	make
	movq	%r13, 8(%rbp)
	movl	$16, %edi
	movq	%rax, 8(%r13)
	movq	%rbp, (%rbx)
	call	malloc@PLT
	movl	$16, %edi
	movq	%rax, %r13
	call	malloc@PLT
	movl	%r12d, %edi
	movq	%rax, %rbp
	call	make
	movl	%r12d, %edi
	movq	%rax, 0(%rbp)
	call	make
	movq	%rbp, 0(%r13)
	movl	$16, %edi
	movq	%rax, 8(%rbp)
	call	malloc@PLT
	movl	%r12d, %edi
	movq	%rax, %rbp
	call	make
	movl	%r12d, %edi
	movq	%rax, 0(%rbp)
	call	make
	movq	%rax, 8(%rbp)
	jmp	.L116
	.cfi_endproc
.LFE22:
	.size	make, .-make
	.p2align 4
	.type	make.constprop.3, @function
make.constprop.3:
.LFB30:
	.cfi_startproc
	pushq	%r13
	.cfi_def_cfa_offset 16
	.cfi_offset 13, -16
	movl	$16, %edi
	pushq	%r12
	.cfi_def_cfa_offset 24
	.cfi_offset 12, -24
	pushq	%rbp
	.cfi_def_cfa_offset 32
	.cfi_offset 6, -32
	pushq	%rbx
	.cfi_def_cfa_offset 40
	.cfi_offset 3, -40
	subq	$8, %rsp
	.cfi_def_cfa_offset 48
	call	malloc@PLT
	movl	$16, %edi
	movq	%rax, %rbx
	call	malloc@PLT
	movl	$16, %edi
	movq	%rax, %rbp
	call	malloc@PLT
	movl	$13, %edi
	movq	%rax, %r12
	call	make
	movl	$13, %edi
	movq	%rax, (%r12)
	call	make
	movq	%r12, 0(%rbp)
	movl	$16, %edi
	movq	%rax, 8(%r12)
	call	malloc@PLT
	movl	$13, %edi
	movq	%rax, %r12
	call	make
	movl	$13, %edi
	movq	%rax, (%r12)
	call	make
	movq	%r12, 8(%rbp)
	movl	$16, %edi
	movq	%rbp, (%rbx)
	movq	%rax, 8(%r12)
	call	malloc@PLT
	movl	$16, %edi
	movq	%rax, %rbp
	call	malloc@PLT
	movl	$13, %edi
	movq	%rax, %r12
	call	make
	movl	$13, %edi
	movq	%rax, (%r12)
	call	make
	movq	%r12, 0(%rbp)
	movl	$16, %edi
	movq	%rax, 8(%r12)
	call	malloc@PLT
	movl	$16, %edi
	movq	%rax, %r12
	call	malloc@PLT
	movl	$12, %edi
	movq	%rax, %r13
	call	make
	movl	$12, %edi
	movq	%rax, 0(%r13)
	call	make
	movq	%r13, (%r12)
	movl	$16, %edi
	movq	%rax, 8(%r13)
	call	malloc@PLT
	movl	$12, %edi
	movq	%rax, %r13
	call	make
	movl	$12, %edi
	movq	%rax, 0(%r13)
	call	make
	movq	%r13, 8(%r12)
	movq	%rax, 8(%r13)
	movq	%rbx, %rax
	movq	%r12, 8(%rbp)
	movq	%rbp, 8(%rbx)
	addq	$8, %rsp
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
	.cfi_endproc
.LFE30:
	.size	make.constprop.3, .-make.constprop.3
	.p2align 4
	.type	make.constprop.0, @function
make.constprop.0:
.LFB33:
	.cfi_startproc
	pushq	%r14
	.cfi_def_cfa_offset 16
	.cfi_offset 14, -16
	movl	$16, %edi
	pushq	%r13
	.cfi_def_cfa_offset 24
	.cfi_offset 13, -24
	pushq	%r12
	.cfi_def_cfa_offset 32
	.cfi_offset 12, -32
	pushq	%rbp
	.cfi_def_cfa_offset 40
	.cfi_offset 6, -40
	pushq	%rbx
	.cfi_def_cfa_offset 48
	.cfi_offset 3, -48
	call	malloc@PLT
	movl	$16, %edi
	movq	%rax, %rbx
	call	malloc@PLT
	movq	%rax, %rbp
	call	make.constprop.3
	movl	$16, %edi
	movq	%rax, 0(%rbp)
	call	malloc@PLT
	movl	$16, %edi
	movq	%rax, %r12
	call	malloc@PLT
	movl	$16, %edi
	movq	%rax, %r13
	call	malloc@PLT
	movl	$13, %edi
	movq	%rax, %r14
	call	make
	movl	$13, %edi
	movq	%rax, (%r14)
	call	make
	movq	%r14, 0(%r13)
	movl	$16, %edi
	movq	%rax, 8(%r14)
	call	malloc@PLT
	movl	$13, %edi
	movq	%rax, %r14
	call	make
	movl	$13, %edi
	movq	%rax, (%r14)
	call	make
	movq	%r14, 8(%r13)
	movl	$16, %edi
	movq	%r13, (%r12)
	movq	%rax, 8(%r14)
	call	malloc@PLT
	movl	$16, %edi
	movq	%rax, %r13
	call	malloc@PLT
	movl	$13, %edi
	movq	%rax, %r14
	call	make
	movl	$13, %edi
	movq	%rax, (%r14)
	call	make
	movq	%r14, 0(%r13)
	movl	$16, %edi
	movq	%rax, 8(%r14)
	call	malloc@PLT
	movl	$13, %edi
	movq	%rax, %r14
	call	make
	movl	$13, %edi
	movq	%rax, (%r14)
	call	make
	movq	%r14, 8(%r13)
	movl	$16, %edi
	movq	%r13, 8(%r12)
	movq	%r12, 8(%rbp)
	movq	%rbp, (%rbx)
	movq	%rax, 8(%r14)
	call	malloc@PLT
	movq	%rax, %rbp
	call	make.constprop.3
	movq	%rax, 0(%rbp)
	call	make.constprop.3
	movq	%rbp, 8(%rbx)
	movq	%rax, 8(%rbp)
	movq	%rbx, %rax
	popq	%rbx
	.cfi_def_cfa_offset 40
	popq	%rbp
	.cfi_def_cfa_offset 32
	popq	%r12
	.cfi_def_cfa_offset 24
	popq	%r13
	.cfi_def_cfa_offset 16
	popq	%r14
	.cfi_def_cfa_offset 8
	ret
	.cfi_endproc
.LFE33:
	.size	make.constprop.0, .-make.constprop.0
	.section	.rodata.str1.8,"aMS",@progbits,1
	.align 8
.LC0:
	.string	"stretch tree of depth %d\t check: %d\n"
	.align 8
.LC1:
	.string	"%d\t trees of depth %d\t check: %d\n"
	.align 8
.LC2:
	.string	"long lived tree of depth %d\t check: %d\n"
	.section	.text.startup,"ax",@progbits
	.p2align 4
	.globl	main
	.type	main, @function
main:
.LFB25:
	.cfi_startproc
	pushq	%r15
	.cfi_def_cfa_offset 16
	.cfi_offset 15, -16
	movl	$16, %edi
	pushq	%r14
	.cfi_def_cfa_offset 24
	.cfi_offset 14, -24
	xorl	%r14d, %r14d
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
	subq	$248, %rsp
	.cfi_def_cfa_offset 304
	call	malloc@PLT
	movq	%rax, %r12
	call	make.constprop.0
	movq	%r12, %r13
	movq	%rax, (%r12)
	movq	%rax, %rbx
	call	make.constprop.0
	movq	%rax, 8(%r12)
.L132:
	movq	(%rbx), %rdi
	movl	$2, %ebp
	testq	%rdi, %rdi
	je	.L128
	xorl	%ebp, %ebp
.L129:
	call	check
	movq	8(%rbx), %rbx
	leal	1(%rbp,%rax), %ebp
	movq	(%rbx), %rdi
	testq	%rdi, %rdi
	jne	.L129
	addl	$2, %ebp
.L128:
	movq	8(%r13), %r13
	addl	%ebp, %r14d
	movq	0(%r13), %rbx
	testq	%rbx, %rbx
	jne	.L132
	movl	$19, %esi
	leal	1(%r14), %edx
	leaq	.LC0(%rip), %rdi
	xorl	%eax, %eax
	call	printf@PLT
	movq	%r12, %rdi
	call	destroy
	call	make.constprop.0
	xorl	%esi, %esi
	movl	$0, 8(%rsp)
	movq	%rax, 216(%rsp)
	movl	%esi, 16(%rsp)
	.p2align 4
	.p2align 3
.L130:
	movl	$16, %edi
	call	malloc@PLT
	movl	$16, %edi
	movq	%rax, %r12
	movq	%rax, 48(%rsp)
	call	malloc@PLT
	movl	$16, %edi
	movq	%rax, %r14
	call	malloc@PLT
	movl	$16, %edi
	movq	%rax, %rbp
	call	malloc@PLT
	movl	$16, %edi
	movq	%rax, %rbx
	movq	%rax, 176(%rsp)
	call	malloc@PLT
	vpxor	%xmm0, %xmm0, %xmm0
	movl	$16, %edi
	movq	%rax, (%rbx)
	vmovdqu	%xmm0, (%rax)
	movq	%rax, 184(%rsp)
	call	malloc@PLT
	movq	%rbx, 0(%rbp)
	vpxor	%xmm0, %xmm0, %xmm0
	movl	$16, %edi
	movq	%rax, 8(%rbx)
	vmovdqu	%xmm0, (%rax)
	movq	%rax, 192(%rsp)
	call	malloc@PLT
	movl	$16, %edi
	movq	%rax, %rbx
	movq	%rax, 104(%rsp)
	call	malloc@PLT
	vpxor	%xmm0, %xmm0, %xmm0
	movl	$16, %edi
	movq	%rax, (%rbx)
	vmovdqu	%xmm0, (%rax)
	movq	%rax, 144(%rsp)
	call	malloc@PLT
	movq	%rbx, 8(%rbp)
	vpxor	%xmm0, %xmm0, %xmm0
	movl	$16, %edi
	movq	%rax, 8(%rbx)
	movq	%rbp, (%r14)
	vmovdqu	%xmm0, (%rax)
	call	malloc@PLT
	movl	$16, %edi
	movq	%rax, %r15
	movq	%rax, 72(%rsp)
	call	malloc@PLT
	movl	$16, %edi
	movq	%rax, %rbx
	movq	%rax, 80(%rsp)
	call	malloc@PLT
	vpxor	%xmm0, %xmm0, %xmm0
	movl	$16, %edi
	movq	%rax, (%rbx)
	vmovdqu	%xmm0, (%rax)
	call	malloc@PLT
	movq	%rbx, (%r15)
	vpxor	%xmm0, %xmm0, %xmm0
	movl	$16, %edi
	movq	%rax, 8(%rbx)
	vmovdqu	%xmm0, (%rax)
	call	malloc@PLT
	movl	$16, %edi
	movq	%rax, %rbx
	movq	%rax, 56(%rsp)
	call	malloc@PLT
	vpxor	%xmm0, %xmm0, %xmm0
	movl	$16, %edi
	movq	%rax, (%rbx)
	vmovdqu	%xmm0, (%rax)
	call	malloc@PLT
	movq	%rbx, 8(%r15)
	vpxor	%xmm0, %xmm0, %xmm0
	movl	$16, %edi
	movq	%rax, 8(%rbx)
	movq	%r12, %rbx
	movq	%r15, 8(%r14)
	movq	%r14, (%r12)
	vmovdqu	%xmm0, (%rax)
	call	malloc@PLT
	movl	$16, %edi
	movq	%rax, %r13
	movq	%rax, 40(%rsp)
	call	malloc@PLT
	movl	$16, %edi
	movq	%rax, %r12
	movq	%rax, 128(%rsp)
	call	malloc@PLT
	movl	$16, %edi
	movq	%rax, %r15
	movq	%rax, 168(%rsp)
	call	malloc@PLT
	vpxor	%xmm0, %xmm0, %xmm0
	movl	$16, %edi
	movq	%rax, (%r15)
	vmovdqu	%xmm0, (%rax)
	movq	%rax, 64(%rsp)
	call	malloc@PLT
	movq	%r15, (%r12)
	vpxor	%xmm0, %xmm0, %xmm0
	movl	$16, %edi
	movq	%rax, 8(%r15)
	vmovdqu	%xmm0, (%rax)
	movq	%rax, 152(%rsp)
	call	malloc@PLT
	movl	$16, %edi
	movq	%rax, %r15
	movq	%rax, 136(%rsp)
	call	malloc@PLT
	vpxor	%xmm0, %xmm0, %xmm0
	movl	$16, %edi
	movq	%rax, (%r15)
	vmovdqu	%xmm0, (%rax)
	movq	%rax, 120(%rsp)
	call	malloc@PLT
	movq	%r15, 8(%r12)
	vpxor	%xmm0, %xmm0, %xmm0
	movl	$16, %edi
	movq	%r12, 0(%r13)
	movq	%rax, 8(%r15)
	vmovdqu	%xmm0, (%rax)
	movq	%rax, 96(%rsp)
	call	malloc@PLT
	movl	$16, %edi
	movq	%rax, %r15
	call	malloc@PLT
	movl	$16, %edi
	movq	%rax, %r12
	movq	%rax, 200(%rsp)
	call	malloc@PLT
	vpxor	%xmm0, %xmm0, %xmm0
	movl	$16, %edi
	movq	%rax, (%r12)
	vmovdqu	%xmm0, (%rax)
	movq	%rax, 88(%rsp)
	call	malloc@PLT
	movq	%r12, (%r15)
	vpxor	%xmm0, %xmm0, %xmm0
	movl	$16, %edi
	movq	%rax, 8(%r12)
	vmovdqu	%xmm0, (%rax)
	movq	%rax, 160(%rsp)
	call	malloc@PLT
	movl	$16, %edi
	movq	%rax, %r12
	movq	%rax, 32(%rsp)
	call	malloc@PLT
	vpxor	%xmm0, %xmm0, %xmm0
	movl	$16, %edi
	movq	%rax, (%r12)
	vmovdqu	%xmm0, (%rax)
	movq	%rax, 112(%rsp)
	call	malloc@PLT
	movq	%r13, 8(%rbx)
	vpxor	%xmm0, %xmm0, %xmm0
	movq	%rbp, %rdi
	movq	%rax, 24(%rsp)
	movq	%r15, 8(%r13)
	movq	%rbx, %r13
	xorl	%ebx, %ebx
	movq	%rax, 8(%r12)
	movq	%r12, 8(%r15)
	movq	%r14, %r12
	movq	%rbp, 208(%rsp)
	movl	%ebx, %ebp
	movq	%r13, %rbx
	vmovdqu	%xmm0, (%rax)
.L181:
	movl	$2, %eax
	xorl	%r13d, %r13d
	testq	%rdi, %rdi
	je	.L133
.L134:
	call	check
	movq	8(%r12), %r12
	leal	1(%r13,%rax), %r13d
	movq	(%r12), %rdi
	testq	%rdi, %rdi
	jne	.L134
	leal	2(%r13), %eax
.L133:
	movq	8(%rbx), %rbx
	addl	%eax, %ebp
	movq	(%rbx), %r12
	testq	%r12, %r12
	je	.L180
	movq	(%r12), %rdi
	jmp	.L181
	.p2align 4,,10
	.p2align 3
.L180:
	movq	184(%rsp), %rdi
	movl	%ebp, %ebx
	movq	208(%rsp), %rbp
	call	free@PLT
	movq	192(%rsp), %rdi
	call	free@PLT
	movq	176(%rsp), %rdi
	call	free@PLT
	movq	144(%rsp), %rax
	movq	(%rax), %r13
	testq	%r13, %r13
	je	.L135
	movq	0(%r13), %rdi
	testq	%rdi, %rdi
	je	.L136
	call	destroy
	movq	8(%r13), %rdi
	call	destroy
.L136:
	movq	%r13, %rdi
	call	free@PLT
	movq	144(%rsp), %rax
	movq	8(%rax), %r13
	movq	0(%r13), %rdi
	testq	%rdi, %rdi
	je	.L137
	call	destroy
	movq	8(%r13), %rdi
	call	destroy
.L137:
	movq	%r13, %rdi
	call	free@PLT
.L135:
	movq	144(%rsp), %rdi
	call	free@PLT
	movq	104(%rsp), %rax
	movq	8(%rax), %r13
	movq	0(%r13), %r12
	testq	%r12, %r12
	je	.L138
	movq	(%r12), %rdi
	testq	%rdi, %rdi
	je	.L139
	call	destroy
	movq	8(%r12), %rdi
	call	destroy
.L139:
	movq	%r12, %rdi
	call	free@PLT
	movq	8(%r13), %r12
	movq	(%r12), %rdi
	testq	%rdi, %rdi
	je	.L140
	call	destroy
	movq	8(%r12), %rdi
	call	destroy
.L140:
	movq	%r12, %rdi
	call	free@PLT
.L138:
	movq	%r13, %rdi
	call	free@PLT
	movq	104(%rsp), %rdi
	call	free@PLT
	movq	%rbp, %rdi
	call	free@PLT
	movq	80(%rsp), %rax
	movq	(%rax), %rbp
	testq	%rbp, %rbp
	je	.L141
	movq	0(%rbp), %r12
	testq	%r12, %r12
	je	.L142
	movq	(%r12), %rdi
	testq	%rdi, %rdi
	je	.L143
	call	destroy
	movq	8(%r12), %rdi
	call	destroy
.L143:
	movq	%r12, %rdi
	call	free@PLT
	movq	8(%rbp), %r12
	movq	(%r12), %rdi
	testq	%rdi, %rdi
	je	.L144
	call	destroy
	movq	8(%r12), %rdi
	call	destroy
.L144:
	movq	%r12, %rdi
	call	free@PLT
.L142:
	movq	%rbp, %rdi
	call	free@PLT
	movq	80(%rsp), %rax
	movq	8(%rax), %rbp
	movq	0(%rbp), %r12
	testq	%r12, %r12
	je	.L145
	movq	(%r12), %rdi
	testq	%rdi, %rdi
	je	.L146
	call	destroy
	movq	8(%r12), %rdi
	call	destroy
.L146:
	movq	%r12, %rdi
	call	free@PLT
	movq	8(%rbp), %r12
	movq	(%r12), %rdi
	testq	%rdi, %rdi
	je	.L147
	call	destroy
	movq	8(%r12), %rdi
	call	destroy
.L147:
	movq	%r12, %rdi
	call	free@PLT
.L145:
	movq	%rbp, %rdi
	call	free@PLT
.L141:
	movq	80(%rsp), %rdi
	call	free@PLT
	movq	56(%rsp), %rax
	movq	(%rax), %rbp
	testq	%rbp, %rbp
	je	.L148
	movq	0(%rbp), %r12
	testq	%r12, %r12
	je	.L149
	movq	(%r12), %rdi
	testq	%rdi, %rdi
	je	.L150
	call	destroy
	movq	8(%r12), %rdi
	call	destroy
.L150:
	movq	%r12, %rdi
	call	free@PLT
	movq	8(%rbp), %r12
	movq	(%r12), %rdi
	testq	%rdi, %rdi
	je	.L151
	call	destroy
	movq	8(%r12), %rdi
	call	destroy
.L151:
	movq	%r12, %rdi
	call	free@PLT
.L149:
	movq	%rbp, %rdi
	call	free@PLT
	movq	56(%rsp), %rax
	movq	8(%rax), %rbp
	movq	0(%rbp), %r12
	testq	%r12, %r12
	je	.L152
	movq	(%r12), %rdi
	testq	%rdi, %rdi
	je	.L153
	call	destroy
	movq	8(%r12), %rdi
	call	destroy
.L153:
	movq	%r12, %rdi
	call	free@PLT
	movq	8(%rbp), %r12
	movq	(%r12), %rdi
	testq	%rdi, %rdi
	je	.L154
	call	destroy
	movq	8(%r12), %rdi
	call	destroy
.L154:
	movq	%r12, %rdi
	call	free@PLT
.L152:
	movq	%rbp, %rdi
	call	free@PLT
.L148:
	movq	56(%rsp), %rdi
	call	free@PLT
	movq	72(%rsp), %rdi
	call	free@PLT
	movq	%r14, %rdi
	call	free@PLT
	movq	64(%rsp), %rax
	movq	(%rax), %rbp
	testq	%rbp, %rbp
	je	.L155
	movq	0(%rbp), %rdi
	testq	%rdi, %rdi
	je	.L156
	call	destroy
	movq	8(%rbp), %rdi
	call	destroy
.L156:
	movq	%rbp, %rdi
	call	free@PLT
	movq	64(%rsp), %rax
	movq	8(%rax), %rbp
	movq	0(%rbp), %rdi
	testq	%rdi, %rdi
	je	.L157
	call	destroy
	movq	8(%rbp), %rdi
	call	destroy
.L157:
	movq	%rbp, %rdi
	call	free@PLT
.L155:
	movq	64(%rsp), %rdi
	call	free@PLT
	movq	152(%rsp), %rax
	movq	(%rax), %rbp
	testq	%rbp, %rbp
	je	.L158
	movq	0(%rbp), %rdi
	testq	%rdi, %rdi
	je	.L159
	call	destroy
	movq	8(%rbp), %rdi
	call	destroy
.L159:
	movq	%rbp, %rdi
	call	free@PLT
	movq	152(%rsp), %rax
	movq	8(%rax), %rbp
	movq	0(%rbp), %rdi
	testq	%rdi, %rdi
	je	.L160
	call	destroy
	movq	8(%rbp), %rdi
	call	destroy
.L160:
	movq	%rbp, %rdi
	call	free@PLT
.L158:
	movq	152(%rsp), %rdi
	call	free@PLT
	movq	168(%rsp), %rdi
	call	free@PLT
	movq	120(%rsp), %rax
	movq	(%rax), %rbp
	testq	%rbp, %rbp
	je	.L161
	movq	0(%rbp), %rdi
	testq	%rdi, %rdi
	je	.L162
	call	destroy
	movq	8(%rbp), %rdi
	call	destroy
.L162:
	movq	%rbp, %rdi
	call	free@PLT
	movq	120(%rsp), %rax
	movq	8(%rax), %rbp
	movq	0(%rbp), %rdi
	testq	%rdi, %rdi
	je	.L163
	call	destroy
	movq	8(%rbp), %rdi
	call	destroy
.L163:
	movq	%rbp, %rdi
	call	free@PLT
.L161:
	movq	120(%rsp), %rdi
	call	free@PLT
	movq	96(%rsp), %rax
	movq	(%rax), %rbp
	testq	%rbp, %rbp
	je	.L164
	movq	0(%rbp), %rdi
	testq	%rdi, %rdi
	je	.L165
	call	destroy
	movq	8(%rbp), %rdi
	call	destroy
.L165:
	movq	%rbp, %rdi
	call	free@PLT
	movq	96(%rsp), %rax
	movq	8(%rax), %rbp
	movq	0(%rbp), %rdi
	testq	%rdi, %rdi
	je	.L166
	call	destroy
	movq	8(%rbp), %rdi
	call	destroy
.L166:
	movq	%rbp, %rdi
	call	free@PLT
.L164:
	movq	96(%rsp), %rdi
	call	free@PLT
	movq	136(%rsp), %rdi
	call	free@PLT
	movq	128(%rsp), %rdi
	call	free@PLT
	movq	88(%rsp), %rax
	movq	(%rax), %rbp
	testq	%rbp, %rbp
	je	.L167
	movq	0(%rbp), %rdi
	testq	%rdi, %rdi
	je	.L168
	call	destroy
	movq	8(%rbp), %rdi
	call	destroy
.L168:
	movq	%rbp, %rdi
	call	free@PLT
	movq	88(%rsp), %rax
	movq	8(%rax), %rbp
	movq	0(%rbp), %rdi
	testq	%rdi, %rdi
	je	.L169
	call	destroy
	movq	8(%rbp), %rdi
	call	destroy
.L169:
	movq	%rbp, %rdi
	call	free@PLT
.L167:
	movq	88(%rsp), %rdi
	call	free@PLT
	movq	160(%rsp), %rax
	movq	(%rax), %rbp
	testq	%rbp, %rbp
	je	.L170
	movq	0(%rbp), %rdi
	testq	%rdi, %rdi
	je	.L171
	call	destroy
	movq	8(%rbp), %rdi
	call	destroy
.L171:
	movq	%rbp, %rdi
	call	free@PLT
	movq	160(%rsp), %rax
	movq	8(%rax), %rbp
	movq	0(%rbp), %rdi
	testq	%rdi, %rdi
	je	.L172
	call	destroy
	movq	8(%rbp), %rdi
	call	destroy
.L172:
	movq	%rbp, %rdi
	call	free@PLT
.L170:
	movq	160(%rsp), %rdi
	call	free@PLT
	movq	200(%rsp), %rdi
	call	free@PLT
	movq	112(%rsp), %rax
	movq	(%rax), %rbp
	testq	%rbp, %rbp
	je	.L173
	movq	0(%rbp), %rdi
	testq	%rdi, %rdi
	je	.L174
	call	destroy
	movq	8(%rbp), %rdi
	call	destroy
.L174:
	movq	%rbp, %rdi
	call	free@PLT
	movq	112(%rsp), %rax
	movq	8(%rax), %rbp
	movq	0(%rbp), %rdi
	testq	%rdi, %rdi
	je	.L175
	call	destroy
	movq	8(%rbp), %rdi
	call	destroy
.L175:
	movq	%rbp, %rdi
	call	free@PLT
.L173:
	movq	112(%rsp), %rdi
	call	free@PLT
	movq	24(%rsp), %rax
	movq	(%rax), %rbp
	testq	%rbp, %rbp
	je	.L176
	movq	0(%rbp), %rdi
	testq	%rdi, %rdi
	je	.L177
	call	destroy
	movq	8(%rbp), %rdi
	call	destroy
.L177:
	movq	%rbp, %rdi
	call	free@PLT
	movq	24(%rsp), %rax
	movq	8(%rax), %rbp
	movq	0(%rbp), %rdi
	testq	%rdi, %rdi
	je	.L178
	call	destroy
	movq	8(%rbp), %rdi
	call	destroy
.L178:
	movq	%rbp, %rdi
	call	free@PLT
.L176:
	movq	24(%rsp), %rdi
	call	free@PLT
	movq	32(%rsp), %rdi
	call	free@PLT
	movq	%r15, %rdi
	call	free@PLT
	movl	16(%rsp), %eax
	movq	40(%rsp), %rdi
	leal	1(%rax,%rbx), %eax
	movl	%eax, 16(%rsp)
	call	free@PLT
	movq	48(%rsp), %rdi
	call	free@PLT
	addl	$1, 8(%rsp)
	movl	8(%rsp), %eax
	cmpl	$262144, %eax
	jne	.L130
	movl	16(%rsp), %ecx
	movl	$4, %edx
	movl	$262144, %esi
	xorl	%eax, %eax
	movq	216(%rsp), %rbx
	leaq	.LC1(%rip), %rdi
	movl	$6, %r12d
	call	printf@PLT
	movq	%rbx, 224(%rsp)
	movl	%r12d, %ebx
.L179:
	movl	$22, %edx
	xorl	%eax, %eax
	movl	$1, %esi
	subl	%ebx, %edx
	testb	$1, %bl
	je	.L182
	movl	$1, %eax
	movl	$2, %esi
	cmpl	%edx, %eax
	je	.L561
	.p2align 4
	.p2align 4
	.p2align 3
.L182:
	addl	$2, %eax
	sall	$2, %esi
	cmpl	%edx, %eax
	jne	.L182
.L561:
	testl	%esi, %esi
	jle	.L249
	xorl	%ecx, %ecx
	movl	%esi, 208(%rsp)
	leal	-5(%rbx), %eax
	leal	-6(%rbx), %r15d
	movl	$0, 128(%rsp)
	movl	%ecx, 160(%rsp)
	movl	%ebx, 216(%rsp)
	movl	%eax, %ebx
	.p2align 4
	.p2align 3
.L210:
	movl	$16, %edi
	call	malloc@PLT
	movl	$16, %edi
	movq	%rax, 136(%rsp)
	call	malloc@PLT
	movl	$16, %edi
	movq	%rax, %r14
	call	malloc@PLT
	movl	$16, %edi
	movq	%rax, %r12
	movq	%rax, 200(%rsp)
	call	malloc@PLT
	movl	$16, %edi
	movq	%rax, 72(%rsp)
	call	malloc@PLT
	movl	$16, %edi
	movq	%rax, %rbp
	call	malloc@PLT
	movl	%r15d, %edi
	movq	%rax, %r13
	call	make
	movl	%r15d, %edi
	movq	%rax, 0(%r13)
	call	make
	movq	%r13, 0(%rbp)
	movl	$16, %edi
	movq	%rax, 8(%r13)
	call	malloc@PLT
	movl	%r15d, %edi
	movq	%rax, %r13
	call	make
	movl	%r15d, %edi
	movq	%rax, 0(%r13)
	call	make
	movq	72(%rsp), %rcx
	movq	%r13, 8(%rbp)
	movl	$16, %edi
	movq	%rax, 8(%r13)
	movq	%rbp, (%rcx)
	call	malloc@PLT
	movl	$16, %edi
	movq	%rax, %rbp
	call	malloc@PLT
	movl	%r15d, %edi
	movq	%rax, %r13
	call	make
	movl	%r15d, %edi
	movq	%rax, 0(%r13)
	call	make
	movq	%r13, 0(%rbp)
	movl	$16, %edi
	movq	%rax, 8(%r13)
	call	malloc@PLT
	movl	%r15d, %edi
	movq	%rax, %r13
	call	make
	movl	%r15d, %edi
	movq	%rax, 0(%r13)
	call	make
	movq	72(%rsp), %rcx
	movq	%r13, 8(%rbp)
	movl	$16, %edi
	movq	%rax, 8(%r13)
	movq	%rbp, 8(%rcx)
	movq	%rcx, (%r12)
	call	malloc@PLT
	movl	$16, %edi
	movq	%rax, 48(%rsp)
	call	malloc@PLT
	movl	$16, %edi
	movq	%rax, %rbp
	call	malloc@PLT
	movl	%r15d, %edi
	movq	%rax, %r13
	call	make
	movl	%r15d, %edi
	movq	%rax, 0(%r13)
	call	make
	movq	%r13, 0(%rbp)
	movl	$16, %edi
	movq	%rax, 8(%r13)
	call	malloc@PLT
	movl	%r15d, %edi
	movq	%rax, %r13
	call	make
	movl	%r15d, %edi
	movq	%rax, 0(%r13)
	call	make
	movq	48(%rsp), %rsi
	movq	%r13, 8(%rbp)
	movl	$16, %edi
	movq	%rax, 8(%r13)
	movq	%rbp, (%rsi)
	call	malloc@PLT
	movl	$16, %edi
	movq	%rax, %rbp
	call	malloc@PLT
	movl	%r15d, %edi
	movq	%rax, %r13
	call	make
	movl	%r15d, %edi
	movq	%rax, 0(%r13)
	call	make
	movq	%r13, 0(%rbp)
	movl	$16, %edi
	movq	%rax, 8(%r13)
	call	malloc@PLT
	movl	%r15d, %edi
	movq	%rax, %r13
	call	make
	movl	%r15d, %edi
	movq	%rax, 0(%r13)
	call	make
	movq	48(%rsp), %rsi
	movq	%r13, 8(%rbp)
	movl	$16, %edi
	movq	%rax, 8(%r13)
	movq	%rbp, 8(%rsi)
	movq	%rsi, 8(%r12)
	movq	%r12, (%r14)
	call	malloc@PLT
	movl	$16, %edi
	movq	%rax, 40(%rsp)
	call	malloc@PLT
	movl	$16, %edi
	movq	%rax, %r13
	movq	%rax, 192(%rsp)
	call	malloc@PLT
	movl	%ebx, %edi
	movq	%rax, %rbp
	call	make
	movl	%ebx, %edi
	movq	%rax, 0(%rbp)
	call	make
	movq	%rbp, 0(%r13)
	movl	$16, %edi
	movq	%rax, 8(%rbp)
	call	malloc@PLT
	movl	%ebx, %edi
	movq	%rax, %rbp
	call	make
	movl	%ebx, %edi
	movq	%rax, 0(%rbp)
	call	make
	movq	40(%rsp), %rdx
	movq	%rbp, 8(%r13)
	movl	$16, %edi
	movq	%rax, 8(%rbp)
	movq	%r13, (%rdx)
	call	malloc@PLT
	movl	$16, %edi
	movq	%rax, 32(%rsp)
	call	malloc@PLT
	movl	$16, %edi
	movq	%rax, %rbp
	call	malloc@PLT
	movl	%r15d, %edi
	movq	%rax, %r13
	call	make
	movl	%r15d, %edi
	movq	%rax, 0(%r13)
	call	make
	movq	%r13, 0(%rbp)
	movl	$16, %edi
	movq	%rax, 8(%r13)
	call	malloc@PLT
	movl	%r15d, %edi
	movq	%rax, %r13
	call	make
	movl	%r15d, %edi
	movq	%rax, 0(%r13)
	call	make
	movq	32(%rsp), %rcx
	movq	%r13, 8(%rbp)
	movl	$16, %edi
	movq	%rax, 8(%r13)
	movq	%rbp, (%rcx)
	call	malloc@PLT
	movl	$16, %edi
	movq	%rax, %rbp
	call	malloc@PLT
	movl	%r15d, %edi
	movq	%rax, %r13
	call	make
	movl	%r15d, %edi
	movq	%rax, 0(%r13)
	call	make
	movq	%r13, 0(%rbp)
	movl	$16, %edi
	movq	%rax, 8(%r13)
	call	malloc@PLT
	movl	%r15d, %edi
	movq	%rax, %r13
	call	make
	movl	%r15d, %edi
	movq	%rax, 0(%r13)
	call	make
	movq	32(%rsp), %rcx
	movq	40(%rsp), %rdx
	movq	%r13, 8(%rbp)
	movq	%rax, 8(%r13)
	movq	136(%rsp), %r13
	movl	$16, %edi
	movq	%rbp, 8(%rcx)
	movq	%rcx, 8(%rdx)
	movq	%rdx, 8(%r14)
	movq	%r14, 0(%r13)
	call	malloc@PLT
	movl	$16, %edi
	movq	%rax, 144(%rsp)
	call	malloc@PLT
	movl	$16, %edi
	movq	%rax, 80(%rsp)
	call	malloc@PLT
	movl	$16, %edi
	movq	%rax, 168(%rsp)
	call	malloc@PLT
	movl	%ebx, %edi
	movq	%rax, %rbp
	movq	%rax, 184(%rsp)
	call	make
	movl	%ebx, %edi
	movq	%rax, 0(%rbp)
	call	make
	movq	%rbp, %rsi
	movl	$16, %edi
	movq	%rax, 8(%rbp)
	movq	168(%rsp), %rbp
	movq	%rsi, 0(%rbp)
	call	malloc@PLT
	movl	%ebx, %edi
	movq	%rax, 88(%rsp)
	call	make
	movq	88(%rsp), %rdx
	movl	%ebx, %edi
	movq	%rax, (%rdx)
	call	make
	movq	88(%rsp), %rdx
	movq	80(%rsp), %rsi
	movl	$16, %edi
	movq	%rbp, (%rsi)
	movq	%rdx, 8(%rbp)
	movq	%rax, 8(%rdx)
	call	malloc@PLT
	movl	$16, %edi
	movq	%rax, 152(%rsp)
	call	malloc@PLT
	movl	$16, %edi
	movq	%rax, 96(%rsp)
	call	malloc@PLT
	movl	%r15d, %edi
	movq	%rax, %rbp
	call	make
	movl	%r15d, %edi
	movq	%rax, 0(%rbp)
	call	make
	movq	96(%rsp), %rcx
	movl	$16, %edi
	movq	%rax, 8(%rbp)
	movq	%rbp, (%rcx)
	call	malloc@PLT
	movl	%r15d, %edi
	movq	%rax, %rbp
	call	make
	movl	%r15d, %edi
	movq	%rax, 0(%rbp)
	call	make
	movq	96(%rsp), %rcx
	movl	$16, %edi
	movq	%rax, 8(%rbp)
	movq	%rbp, 8(%rcx)
	movq	152(%rsp), %rbp
	movq	%rcx, 0(%rbp)
	call	malloc@PLT
	movl	%ebx, %edi
	movq	%rax, 56(%rsp)
	call	make
	movq	56(%rsp), %rdx
	movl	%ebx, %edi
	movq	%rax, (%rdx)
	call	make
	movq	56(%rsp), %rdx
	movq	80(%rsp), %rsi
	movl	$16, %edi
	movq	%rdx, 8(%rbp)
	movq	%rbp, 8(%rsi)
	movq	144(%rsp), %rbp
	movq	%rax, 8(%rdx)
	movq	%rsi, 0(%rbp)
	call	malloc@PLT
	movl	$16, %edi
	movq	%rax, 16(%rsp)
	call	malloc@PLT
	movl	$16, %edi
	movq	%rax, 120(%rsp)
	call	malloc@PLT
	movl	%ebx, %edi
	movq	%rax, 64(%rsp)
	call	make
	movq	64(%rsp), %rsi
	movl	%ebx, %edi
	movq	%rax, (%rsi)
	call	make
	movq	64(%rsp), %rsi
	movq	120(%rsp), %rcx
	movl	$16, %edi
	movq	%rsi, (%rcx)
	movq	%rax, 8(%rsi)
	call	malloc@PLT
	movl	%ebx, %edi
	movq	%rax, 104(%rsp)
	call	make
	movq	104(%rsp), %rdx
	movl	%ebx, %edi
	movq	%rax, (%rdx)
	call	make
	movq	104(%rsp), %rdx
	movq	120(%rsp), %rcx
	movl	$16, %edi
	movq	%rax, 8(%rdx)
	movq	16(%rsp), %rax
	movq	%rdx, 8(%rcx)
	movq	%rcx, (%rax)
	call	malloc@PLT
	movl	$16, %edi
	movq	%rax, 24(%rsp)
	call	malloc@PLT
	movl	%ebx, %edi
	movq	%rax, 112(%rsp)
	call	make
	movq	112(%rsp), %rsi
	movl	%ebx, %edi
	movq	%rax, (%rsi)
	call	make
	movq	112(%rsp), %rsi
	movq	24(%rsp), %rcx
	movl	$16, %edi
	movq	%rsi, (%rcx)
	movq	%rax, 8(%rsi)
	call	malloc@PLT
	movl	%ebx, %edi
	movq	%rax, 8(%rsp)
	call	make
	movq	8(%rsp), %rdx
	movl	%ebx, %edi
	movq	%rax, (%rdx)
	call	make
	movq	8(%rsp), %rdx
	movq	24(%rsp), %rcx
	movq	%r12, %rdi
	movq	%rbp, 8(%r13)
	movq	%r14, %r12
	movq	%rax, 8(%rdx)
	movq	16(%rsp), %rax
	movl	%ebx, 176(%rsp)
	movq	%rax, 8(%rbp)
	xorl	%ebp, %ebp
	movl	%ebp, %ebx
	movq	%rdx, 8(%rcx)
	movq	%r13, %rbp
	xorl	%r13d, %r13d
	movq	%rcx, 8(%rax)
	testq	%rdi, %rdi
	je	.L566
	.p2align 4
	.p2align 3
.L184:
	call	check
	movq	8(%r12), %r12
	leal	1(%r13,%rax), %r13d
	movq	(%r12), %rdi
	testq	%rdi, %rdi
	jne	.L184
	movq	8(%rbp), %rbp
	leal	2(%r13), %eax
	addl	%eax, %ebx
	movq	0(%rbp), %r12
	testq	%r12, %r12
	je	.L567
.L559:
	movq	(%r12), %rdi
	xorl	%r13d, %r13d
	testq	%rdi, %rdi
	jne	.L184
.L566:
	movq	8(%rbp), %rbp
	movl	$2, %eax
	addl	%eax, %ebx
	movq	0(%rbp), %r12
	testq	%r12, %r12
	jne	.L559
.L567:
	movq	72(%rsp), %rax
	movl	%ebx, %ebp
	movl	176(%rsp), %ebx
	movq	(%rax), %rax
	testq	%rax, %rax
	je	.L211
	movq	(%rax), %rdx
	testq	%rdx, %rdx
	je	.L212
	movq	(%rdx), %rdi
	testq	%rdi, %rdi
	je	.L213
	movq	%rax, 232(%rsp)
	movq	%rdx, 176(%rsp)
	call	destroy
	movq	176(%rsp), %rdx
	movq	8(%rdx), %rdi
	call	destroy
	movq	232(%rsp), %rax
	movq	176(%rsp), %rdx
.L213:
	movq	%rdx, %rdi
	movq	%rax, 176(%rsp)
	call	free@PLT
	movq	176(%rsp), %rax
	movq	8(%rax), %rdx
	movq	(%rdx), %rdi
	testq	%rdi, %rdi
	je	.L214
	movq	%rax, 232(%rsp)
	movq	%rdx, 176(%rsp)
	call	destroy
	movq	176(%rsp), %rdx
	movq	8(%rdx), %rdi
	call	destroy
	movq	232(%rsp), %rax
	movq	176(%rsp), %rdx
.L214:
	movq	%rdx, %rdi
	movq	%rax, 176(%rsp)
	call	free@PLT
	movq	176(%rsp), %rax
.L212:
	movq	%rax, %rdi
	call	free@PLT
	movq	72(%rsp), %rax
	movq	8(%rax), %rax
	movq	(%rax), %rdx
	testq	%rdx, %rdx
	je	.L215
	movq	(%rdx), %rdi
	testq	%rdi, %rdi
	je	.L216
	movq	%rax, 232(%rsp)
	movq	%rdx, 176(%rsp)
	call	destroy
	movq	176(%rsp), %rdx
	movq	8(%rdx), %rdi
	call	destroy
	movq	232(%rsp), %rax
	movq	176(%rsp), %rdx
.L216:
	movq	%rdx, %rdi
	movq	%rax, 176(%rsp)
	call	free@PLT
	movq	176(%rsp), %rax
	movq	8(%rax), %rdx
	movq	(%rdx), %rdi
	testq	%rdi, %rdi
	je	.L217
	movq	%rax, 232(%rsp)
	movq	%rdx, 176(%rsp)
	call	destroy
	movq	176(%rsp), %rdx
	movq	8(%rdx), %rdi
	call	destroy
	movq	232(%rsp), %rax
	movq	176(%rsp), %rdx
.L217:
	movq	%rdx, %rdi
	movq	%rax, 176(%rsp)
	call	free@PLT
	movq	176(%rsp), %rax
.L215:
	movq	%rax, %rdi
	call	free@PLT
	.p2align 4
	.p2align 3
.L211:
	movq	72(%rsp), %rdi
	call	free@PLT
	movq	48(%rsp), %rax
	movq	(%rax), %rax
	testq	%rax, %rax
	je	.L218
	movq	(%rax), %rdx
	testq	%rdx, %rdx
	je	.L219
	movq	(%rdx), %rdi
	testq	%rdi, %rdi
	je	.L220
	movq	%rax, 176(%rsp)
	movq	%rdx, 72(%rsp)
	call	destroy
	movq	72(%rsp), %rdx
	movq	8(%rdx), %rdi
	call	destroy
	movq	176(%rsp), %rax
	movq	72(%rsp), %rdx
.L220:
	movq	%rdx, %rdi
	movq	%rax, 72(%rsp)
	call	free@PLT
	movq	72(%rsp), %rax
	movq	8(%rax), %rdx
	movq	(%rdx), %rdi
	testq	%rdi, %rdi
	je	.L221
	movq	%rax, 176(%rsp)
	movq	%rdx, 72(%rsp)
	call	destroy
	movq	72(%rsp), %rdx
	movq	8(%rdx), %rdi
	call	destroy
	movq	176(%rsp), %rax
	movq	72(%rsp), %rdx
.L221:
	movq	%rdx, %rdi
	movq	%rax, 72(%rsp)
	call	free@PLT
	movq	72(%rsp), %rax
.L219:
	movq	%rax, %rdi
	call	free@PLT
	movq	48(%rsp), %rax
	movq	8(%rax), %rax
	movq	(%rax), %rdx
	testq	%rdx, %rdx
	je	.L222
	movq	(%rdx), %rdi
	testq	%rdi, %rdi
	je	.L223
	movq	%rax, 176(%rsp)
	movq	%rdx, 72(%rsp)
	call	destroy
	movq	72(%rsp), %rdx
	movq	8(%rdx), %rdi
	call	destroy
	movq	176(%rsp), %rax
	movq	72(%rsp), %rdx
.L223:
	movq	%rdx, %rdi
	movq	%rax, 72(%rsp)
	call	free@PLT
	movq	72(%rsp), %rax
	movq	8(%rax), %rdx
	movq	(%rdx), %rdi
	testq	%rdi, %rdi
	je	.L224
	movq	%rax, 176(%rsp)
	movq	%rdx, 72(%rsp)
	call	destroy
	movq	72(%rsp), %rdx
	movq	8(%rdx), %rdi
	call	destroy
	movq	176(%rsp), %rax
	movq	72(%rsp), %rdx
.L224:
	movq	%rdx, %rdi
	movq	%rax, 72(%rsp)
	call	free@PLT
	movq	72(%rsp), %rax
.L222:
	movq	%rax, %rdi
	call	free@PLT
.L218:
	movq	48(%rsp), %rdi
	call	free@PLT
	movq	200(%rsp), %rdi
	call	free@PLT
	movq	192(%rsp), %rax
	movq	(%rax), %r12
	testq	%r12, %r12
	je	.L225
	movq	(%r12), %rax
	testq	%rax, %rax
	je	.L226
	movq	(%rax), %rdi
	testq	%rdi, %rdi
	je	.L227
	movq	%rax, 48(%rsp)
	call	destroy
	movq	48(%rsp), %rax
	movq	8(%rax), %rdi
	call	destroy
	movq	48(%rsp), %rax
.L227:
	movq	%rax, %rdi
	call	free@PLT
	movq	8(%r12), %rax
	movq	(%rax), %rdi
	testq	%rdi, %rdi
	je	.L228
	movq	%rax, 48(%rsp)
	call	destroy
	movq	48(%rsp), %rax
	movq	8(%rax), %rdi
	call	destroy
	movq	48(%rsp), %rax
.L228:
	movq	%rax, %rdi
	call	free@PLT
.L226:
	movq	%r12, %rdi
	call	free@PLT
	movq	192(%rsp), %rax
	movq	8(%rax), %r12
	movq	(%r12), %rax
	testq	%rax, %rax
	je	.L229
	movq	(%rax), %rdi
	testq	%rdi, %rdi
	je	.L230
	movq	%rax, 48(%rsp)
	call	destroy
	movq	48(%rsp), %rax
	movq	8(%rax), %rdi
	call	destroy
	movq	48(%rsp), %rax
.L230:
	movq	%rax, %rdi
	call	free@PLT
	movq	8(%r12), %rax
	movq	(%rax), %rdi
	testq	%rdi, %rdi
	je	.L231
	movq	%rax, 48(%rsp)
	call	destroy
	movq	48(%rsp), %rax
	movq	8(%rax), %rdi
	call	destroy
	movq	48(%rsp), %rax
.L231:
	movq	%rax, %rdi
	call	free@PLT
.L229:
	movq	%r12, %rdi
	call	free@PLT
.L225:
	movq	192(%rsp), %rdi
	call	free@PLT
	movq	32(%rsp), %rax
	movq	(%rax), %r12
	testq	%r12, %r12
	je	.L232
	movq	(%r12), %rax
	testq	%rax, %rax
	je	.L233
	movq	(%rax), %rdi
	testq	%rdi, %rdi
	je	.L234
	movq	%rax, 48(%rsp)
	call	destroy
	movq	48(%rsp), %rax
	movq	8(%rax), %rdi
	call	destroy
	movq	48(%rsp), %rax
.L234:
	movq	%rax, %rdi
	call	free@PLT
	movq	8(%r12), %rax
	movq	(%rax), %rdi
	testq	%rdi, %rdi
	je	.L235
	movq	%rax, 48(%rsp)
	call	destroy
	movq	48(%rsp), %rax
	movq	8(%rax), %rdi
	call	destroy
	movq	48(%rsp), %rax
.L235:
	movq	%rax, %rdi
	call	free@PLT
.L233:
	movq	%r12, %rdi
	call	free@PLT
	movq	32(%rsp), %rax
	movq	8(%rax), %r12
	movq	(%r12), %rax
	testq	%rax, %rax
	je	.L236
	movq	(%rax), %rdi
	testq	%rdi, %rdi
	je	.L237
	movq	%rax, 48(%rsp)
	call	destroy
	movq	48(%rsp), %rax
	movq	8(%rax), %rdi
	call	destroy
	movq	48(%rsp), %rax
.L237:
	movq	%rax, %rdi
	call	free@PLT
	movq	8(%r12), %rax
	movq	(%rax), %rdi
	testq	%rdi, %rdi
	je	.L238
	movq	%rax, 48(%rsp)
	call	destroy
	movq	48(%rsp), %rax
	movq	8(%rax), %rdi
	call	destroy
	movq	48(%rsp), %rax
.L238:
	movq	%rax, %rdi
	call	free@PLT
.L236:
	movq	%r12, %rdi
	call	free@PLT
.L232:
	movq	32(%rsp), %rdi
	call	free@PLT
	movq	40(%rsp), %rdi
	call	free@PLT
	movq	%r14, %rdi
	call	free@PLT
	movq	184(%rsp), %rax
	movq	(%rax), %r12
	movq	%rax, %rdi
	testq	%r12, %r12
	je	.L240
	movq	(%r12), %rdi
	testq	%rdi, %rdi
	je	.L187
	call	destroy
	movq	8(%r12), %rdi
	call	destroy
.L187:
	movq	%r12, %rdi
	call	free@PLT
	movq	184(%rsp), %rax
	movq	8(%rax), %r12
	movq	(%r12), %rdi
	testq	%rdi, %rdi
	je	.L188
	call	destroy
	movq	8(%r12), %rdi
	call	destroy
.L188:
	movq	%r12, %rdi
	call	free@PLT
	movq	184(%rsp), %rdi
	.p2align 4
	.p2align 3
.L240:
	call	free@PLT
	movq	88(%rsp), %rax
	movq	(%rax), %r12
	testq	%r12, %r12
	je	.L189
	movq	(%r12), %rdi
	testq	%rdi, %rdi
	je	.L190
	call	destroy
	movq	8(%r12), %rdi
	call	destroy
.L190:
	movq	%r12, %rdi
	call	free@PLT
	movq	88(%rsp), %rax
	movq	8(%rax), %r12
	movq	(%r12), %rdi
	testq	%rdi, %rdi
	je	.L191
	call	destroy
	movq	8(%r12), %rdi
	call	destroy
.L191:
	movq	%r12, %rdi
	call	free@PLT
.L189:
	movq	88(%rsp), %rdi
	call	free@PLT
	movq	168(%rsp), %rdi
	call	free@PLT
	movq	96(%rsp), %rax
	movq	(%rax), %r12
	testq	%r12, %r12
	je	.L192
	movq	(%r12), %rdi
	testq	%rdi, %rdi
	je	.L193
	call	destroy
	movq	8(%r12), %rdi
	call	destroy
.L193:
	movq	%r12, %rdi
	call	free@PLT
	movq	96(%rsp), %rax
	movq	8(%rax), %r12
	movq	(%r12), %rdi
	testq	%rdi, %rdi
	je	.L194
	call	destroy
	movq	8(%r12), %rdi
	call	destroy
.L194:
	movq	%r12, %rdi
	call	free@PLT
.L192:
	movq	96(%rsp), %rdi
	call	free@PLT
	movq	56(%rsp), %rax
	movq	(%rax), %r12
	testq	%r12, %r12
	je	.L195
	movq	(%r12), %rdi
	testq	%rdi, %rdi
	je	.L196
	call	destroy
	movq	8(%r12), %rdi
	call	destroy
.L196:
	movq	%r12, %rdi
	call	free@PLT
	movq	56(%rsp), %rax
	movq	8(%rax), %r12
	movq	(%r12), %rdi
	testq	%rdi, %rdi
	je	.L197
	call	destroy
	movq	8(%r12), %rdi
	call	destroy
.L197:
	movq	%r12, %rdi
	call	free@PLT
.L195:
	movq	56(%rsp), %rdi
	call	free@PLT
	movq	152(%rsp), %rdi
	call	free@PLT
	movq	80(%rsp), %rdi
	call	free@PLT
	movq	64(%rsp), %rax
	movq	(%rax), %r12
	testq	%r12, %r12
	je	.L198
	movq	(%r12), %rdi
	testq	%rdi, %rdi
	je	.L199
	call	destroy
	movq	8(%r12), %rdi
	call	destroy
.L199:
	movq	%r12, %rdi
	call	free@PLT
	movq	64(%rsp), %rax
	movq	8(%rax), %r12
	movq	(%r12), %rdi
	testq	%rdi, %rdi
	je	.L200
	call	destroy
	movq	8(%r12), %rdi
	call	destroy
.L200:
	movq	%r12, %rdi
	call	free@PLT
.L198:
	movq	64(%rsp), %rdi
	call	free@PLT
	movq	104(%rsp), %rax
	movq	(%rax), %r12
	testq	%r12, %r12
	je	.L201
	movq	(%r12), %rdi
	testq	%rdi, %rdi
	je	.L202
	call	destroy
	movq	8(%r12), %rdi
	call	destroy
.L202:
	movq	%r12, %rdi
	call	free@PLT
	movq	104(%rsp), %rax
	movq	8(%rax), %r12
	movq	(%r12), %rdi
	testq	%rdi, %rdi
	je	.L203
	call	destroy
	movq	8(%r12), %rdi
	call	destroy
.L203:
	movq	%r12, %rdi
	call	free@PLT
.L201:
	movq	104(%rsp), %rdi
	call	free@PLT
	movq	120(%rsp), %rdi
	call	free@PLT
	movq	112(%rsp), %rax
	movq	(%rax), %r12
	testq	%r12, %r12
	je	.L204
	movq	(%r12), %rdi
	testq	%rdi, %rdi
	je	.L205
	call	destroy
	movq	8(%r12), %rdi
	call	destroy
.L205:
	movq	%r12, %rdi
	call	free@PLT
	movq	112(%rsp), %rax
	movq	8(%rax), %r12
	movq	(%r12), %rdi
	testq	%rdi, %rdi
	je	.L206
	call	destroy
	movq	8(%r12), %rdi
	call	destroy
.L206:
	movq	%r12, %rdi
	call	free@PLT
.L204:
	movq	112(%rsp), %rdi
	call	free@PLT
	movq	8(%rsp), %rax
	movq	(%rax), %r12
	testq	%r12, %r12
	je	.L207
	movq	(%r12), %rdi
	testq	%rdi, %rdi
	je	.L208
	call	destroy
	movq	8(%r12), %rdi
	call	destroy
.L208:
	movq	%r12, %rdi
	call	free@PLT
	movq	8(%rsp), %rax
	movq	8(%rax), %r12
	movq	(%r12), %rdi
	testq	%rdi, %rdi
	je	.L209
	call	destroy
	movq	8(%r12), %rdi
	call	destroy
.L209:
	movq	%r12, %rdi
	call	free@PLT
.L207:
	movq	8(%rsp), %rdi
	call	free@PLT
	movq	24(%rsp), %rdi
	call	free@PLT
	movq	16(%rsp), %rdi
	call	free@PLT
	movl	160(%rsp), %eax
	movq	144(%rsp), %rdi
	leal	1(%rax,%rbp), %eax
	movl	%eax, 160(%rsp)
	call	free@PLT
	movq	136(%rsp), %rdi
	call	free@PLT
	addl	$1, 128(%rsp)
	movl	128(%rsp), %eax
	cmpl	%eax, 208(%rsp)
	jne	.L210
	movl	160(%rsp), %ecx
	movl	208(%rsp), %esi
	movl	216(%rsp), %ebx
.L183:
	movl	%ebx, %edx
	leaq	.LC1(%rip), %rdi
	xorl	%eax, %eax
	addl	$2, %ebx
	call	printf@PLT
	cmpl	$20, %ebx
	jne	.L179
	movq	224(%rsp), %rbx
	xorl	%r14d, %r14d
	movq	(%rbx), %rbp
	testq	%rbp, %rbp
	je	.L243
	movq	%rbx, %r13
.L246:
	movq	0(%rbp), %rdi
	movl	$2, %r12d
	testq	%rdi, %rdi
	je	.L244
	xorl	%r12d, %r12d
.L245:
	call	check
	movq	8(%rbp), %rbp
	leal	1(%r12,%rax), %r12d
	movq	0(%rbp), %rdi
	testq	%rdi, %rdi
	jne	.L245
	addl	$2, %r12d
.L244:
	movq	8(%r13), %r13
	addl	%r12d, %r14d
	movq	0(%r13), %rbp
	testq	%rbp, %rbp
	jne	.L246
.L243:
	leal	1(%r14), %edx
	movl	$18, %esi
	leaq	.LC2(%rip), %rdi
	xorl	%eax, %eax
	call	printf@PLT
	movq	%rbx, %rdi
	call	destroy
	addq	$248, %rsp
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
.L249:
	.cfi_restore_state
	xorl	%ecx, %ecx
	jmp	.L183
	.cfi_endproc
.LFE25:
	.size	main, .-main
	.ident	"GCC: (Debian 14.2.0-19) 14.2.0"
	.section	.note.GNU-stack,"",@progbits
