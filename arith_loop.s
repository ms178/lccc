	.file	"arith_loop.c"
	.text
	.p2align 4
	.globl	arith_loop
	.type	arith_loop, @function
arith_loop:
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
	pushq	%rbx
	.cfi_def_cfa_offset 56
	.cfi_offset 3, -56
	movl	%edi, -4(%rsp)
	testl	%edi, %edi
	jle	.L4
	movl	$32, %edx
	movl	$0, -48(%rsp)
	movl	$31, %ecx
	movl	$30, %esi
	movl	$29, -8(%rsp)
	movl	$28, %edi
	movl	$27, %r8d
	movl	$26, %r9d
	movl	$19, -52(%rsp)
	movl	$25, %r10d
	movl	$24, %r11d
	movl	$23, %ebx
	movl	$18, -12(%rsp)
	movl	$22, %ebp
	movl	$21, %r12d
	movl	$20, %r13d
	movl	$17, -56(%rsp)
	movl	$6, %r14d
	movl	$16, -16(%rsp)
	movl	$15, -60(%rsp)
	movl	$14, -20(%rsp)
	movl	$13, -84(%rsp)
	movl	$12, -64(%rsp)
	movl	$11, -68(%rsp)
	movl	$10, -24(%rsp)
	movl	$9, -72(%rsp)
	movl	$8, -28(%rsp)
	movl	$7, -32(%rsp)
	movl	$5, -36(%rsp)
	movl	$4, -40(%rsp)
	movl	$3, -88(%rsp)
	movl	$2, -76(%rsp)
	movl	$1, -44(%rsp)
	movl	%edx, -80(%rsp)
	.p2align 4
	.p2align 3
.L3:
	movl	-88(%rsp), %edx
	movl	-40(%rsp), %eax
	movl	-76(%rsp), %r15d
	imull	%edx, %r15d
	addl	%r15d, -44(%rsp)
	movl	%eax, %r15d
	imull	%edx, %r15d
	movl	-36(%rsp), %edx
	addl	%r15d, -76(%rsp)
	movl	%eax, %r15d
	imull	%edx, %r15d
	addl	%r15d, -88(%rsp)
	movl	%edx, %r15d
	imull	%r14d, %r15d
	addl	%r15d, %eax
	movl	%eax, -40(%rsp)
	movl	-32(%rsp), %eax
	movl	%eax, %r15d
	imull	%r14d, %r15d
	addl	%r15d, %edx
	movl	%eax, %r15d
	movl	%edx, -36(%rsp)
	movl	-28(%rsp), %edx
	imull	%edx, %r15d
	addl	%r15d, %r14d
	movl	-72(%rsp), %r15d
	imull	%edx, %r15d
	addl	%r15d, %eax
	movl	-72(%rsp), %r15d
	movl	%eax, -32(%rsp)
	movl	-24(%rsp), %eax
	imull	%eax, %r15d
	addl	%r15d, %edx
	movl	-68(%rsp), %r15d
	movl	%edx, -28(%rsp)
	movl	%eax, %edx
	imull	%eax, %r15d
	movl	-64(%rsp), %eax
	addl	%r15d, -72(%rsp)
	movl	-68(%rsp), %r15d
	imull	%eax, %r15d
	movl	%edx, %eax
	movl	-84(%rsp), %edx
	addl	%r15d, %eax
	movl	%edx, %r15d
	movl	%eax, -24(%rsp)
	movl	-64(%rsp), %eax
	imull	%eax, %r15d
	addl	%r15d, -68(%rsp)
	movl	%edx, %r15d
	movl	-20(%rsp), %edx
	imull	%edx, %r15d
	addl	%r15d, %eax
	movl	-60(%rsp), %r15d
	movl	%eax, -64(%rsp)
	movl	%edx, %eax
	imull	%edx, %r15d
	addl	%r15d, -84(%rsp)
	movl	-16(%rsp), %edx
	movl	-60(%rsp), %r15d
	imull	%edx, %r15d
	addl	%r15d, %eax
	movl	-56(%rsp), %r15d
	movl	%eax, -20(%rsp)
	movl	%edx, %eax
	imull	%edx, %r15d
	movl	-12(%rsp), %edx
	addl	%r15d, -60(%rsp)
	movl	-56(%rsp), %r15d
	imull	%edx, %r15d
	addl	%r15d, %eax
	movl	-52(%rsp), %r15d
	movl	%eax, -16(%rsp)
	movl	%edx, %eax
	imull	%edx, %r15d
	addl	%r15d, -56(%rsp)
	movl	-52(%rsp), %r15d
	movl	-8(%rsp), %edx
	imull	%r13d, %r15d
	addl	%r15d, %eax
	movl	%eax, -12(%rsp)
	movl	%r13d, %eax
	imull	%r12d, %eax
	addl	%eax, -52(%rsp)
	movl	%r12d, %eax
	imull	%ebp, %eax
	addl	%eax, %r13d
	movl	%ebp, %eax
	imull	%ebx, %eax
	addl	%eax, %r12d
	movl	%ebx, %eax
	imull	%r11d, %eax
	addl	%eax, %ebp
	movl	%r11d, %eax
	imull	%r10d, %eax
	addl	%eax, %ebx
	movl	%r10d, %eax
	imull	%r9d, %eax
	addl	%eax, %r11d
	movl	%r9d, %eax
	imull	%r8d, %eax
	addl	%eax, %r10d
	movl	%r8d, %eax
	imull	%edi, %eax
	addl	%eax, %r9d
	movl	%edi, %eax
	imull	%edx, %eax
	addl	%eax, %r8d
	movl	%edx, %eax
	imull	%esi, %eax
	addl	%eax, %edi
	movl	%esi, %eax
	imull	%ecx, %eax
	addl	%eax, %edx
	movl	%ecx, %eax
	movl	%edx, -8(%rsp)
	movl	-80(%rsp), %edx
	imull	%edx, %eax
	addl	%eax, %esi
	movl	-44(%rsp), %eax
	movl	%eax, %r15d
	imull	%edx, %r15d
	movl	-76(%rsp), %edx
	addl	%r15d, %ecx
	movl	%eax, %r15d
	imull	%edx, %r15d
	addl	%r15d, -80(%rsp)
	addl	$1, -48(%rsp)
	movl	-48(%rsp), %eax
	cmpl	%eax, -4(%rsp)
	jne	.L3
	movl	-44(%rsp), %r15d
	movl	%edx, %eax
	movl	-80(%rsp), %edx
	xorl	%eax, %r15d
	movl	-88(%rsp), %eax
	xorl	%eax, %r15d
	movl	-40(%rsp), %eax
	xorl	%eax, %r15d
	movl	-36(%rsp), %eax
	xorl	%eax, %r15d
	movl	-32(%rsp), %eax
	xorl	%r15d, %r14d
	xorl	%eax, %r14d
	movl	-28(%rsp), %eax
	xorl	%eax, %r14d
	movl	-72(%rsp), %eax
	xorl	%eax, %r14d
	movl	-24(%rsp), %eax
	xorl	%eax, %r14d
	movl	-68(%rsp), %eax
	xorl	%eax, %r14d
	movl	-64(%rsp), %eax
	xorl	%eax, %r14d
	movl	-84(%rsp), %eax
	xorl	%eax, %r14d
	movl	-20(%rsp), %eax
	xorl	%eax, %r14d
	movl	-60(%rsp), %eax
	xorl	%eax, %r14d
	movl	-16(%rsp), %eax
	xorl	%eax, %r14d
	movl	-56(%rsp), %eax
	xorl	%eax, %r14d
	movl	-12(%rsp), %eax
	xorl	%eax, %r14d
	movl	-52(%rsp), %eax
	xorl	%eax, %r14d
	movl	-8(%rsp), %eax
	xorl	%r13d, %r14d
	xorl	%r12d, %r14d
	xorl	%r14d, %ebp
	xorl	%ebp, %ebx
	xorl	%ebx, %r11d
	popq	%rbx
	.cfi_remember_state
	.cfi_def_cfa_offset 48
	popq	%rbp
	.cfi_def_cfa_offset 40
	xorl	%r11d, %r10d
	popq	%r12
	.cfi_def_cfa_offset 32
	popq	%r13
	.cfi_def_cfa_offset 24
	xorl	%r10d, %r9d
	popq	%r14
	.cfi_def_cfa_offset 16
	popq	%r15
	.cfi_def_cfa_offset 8
	xorl	%r9d, %r8d
	xorl	%r8d, %edi
	xorl	%edi, %eax
	xorl	%esi, %eax
	xorl	%ecx, %eax
	xorl	%edx, %eax
	ret
	.p2align 4,,10
	.p2align 3
.L4:
	.cfi_restore_state
	popq	%rbx
	.cfi_def_cfa_offset 48
	movl	$32, %eax
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
.LFE11:
	.size	arith_loop, .-arith_loop
	.section	.rodata.str1.1,"aMS",@progbits,1
.LC0:
	.string	"arith_loop result: %d\n"
	.section	.text.startup,"ax",@progbits
	.p2align 4
	.globl	main
	.type	main, @function
main:
.LFB12:
	.cfi_startproc
	subq	$24, %rsp
	.cfi_def_cfa_offset 32
	movl	$10000000, %edi
	call	arith_loop
	leaq	.LC0(%rip), %rdi
	movl	%eax, 12(%rsp)
	movl	12(%rsp), %esi
	xorl	%eax, %eax
	call	printf@PLT
	xorl	%eax, %eax
	addq	$24, %rsp
	.cfi_def_cfa_offset 8
	ret
	.cfi_endproc
.LFE12:
	.size	main, .-main
	.ident	"GCC: (Debian 14.2.0-19) 14.2.0"
	.section	.note.GNU-stack,"",@progbits
