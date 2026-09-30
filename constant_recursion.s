	.file	"constant_recursion.c"
	.text
	.p2align 4
	.type	ackermann, @function
ackermann:
.LFB11:
	.cfi_startproc
	pushq	%r15
	.cfi_def_cfa_offset 16
	.cfi_offset 15, -16
	movl	%esi, %eax
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
	movl	%edi, %ebx
	subq	$24, %rsp
	.cfi_def_cfa_offset 80
.L2:
	movl	%ebx, %ebp
	subl	$1, %ebx
	testl	%eax, %eax
	jne	.L42
	movl	$1, %eax
.L3:
	testl	%ebx, %ebx
	jne	.L2
	addq	$24, %rsp
	.cfi_remember_state
	.cfi_def_cfa_offset 56
	addl	$1, %eax
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
.L42:
	.cfi_restore_state
	subl	$1, %eax
.L4:
	movl	%ebp, %r12d
	subl	$1, %ebp
	testl	%eax, %eax
	jne	.L43
	movl	$1, %eax
.L5:
	testl	%ebp, %ebp
	jne	.L4
	addl	$1, %eax
	jmp	.L3
	.p2align 4,,10
	.p2align 3
.L43:
	subl	$1, %eax
.L6:
	movl	%r12d, %r13d
	subl	$1, %r12d
	testl	%eax, %eax
	jne	.L44
	movl	$1, %eax
.L7:
	testl	%r12d, %r12d
	jne	.L6
	addl	$1, %eax
	jmp	.L5
	.p2align 4,,10
	.p2align 3
.L44:
	subl	$1, %eax
.L8:
	movl	%r13d, %r14d
	subl	$1, %r13d
	testl	%eax, %eax
	jne	.L45
	movl	$1, %eax
.L9:
	testl	%r13d, %r13d
	jne	.L8
	addl	$1, %eax
	jmp	.L7
	.p2align 4,,10
	.p2align 3
.L45:
	subl	$1, %eax
.L10:
	movl	%r14d, %r15d
	subl	$1, %r14d
	testl	%eax, %eax
	jne	.L46
	movl	$1, %eax
.L11:
	testl	%r14d, %r14d
	jne	.L10
	addl	$1, %eax
	jmp	.L9
	.p2align 4,,10
	.p2align 3
.L46:
	subl	$1, %eax
.L12:
	movl	%r15d, %edx
	subl	$1, %r15d
	testl	%eax, %eax
	jne	.L47
	movl	$1, %eax
.L13:
	testl	%r15d, %r15d
	jne	.L12
	addl	$1, %eax
	jmp	.L11
	.p2align 4,,10
	.p2align 3
.L47:
	subl	$1, %eax
.L14:
	movl	%edx, %ecx
	subl	$1, %edx
	testl	%eax, %eax
	jne	.L48
	movl	$1, %eax
.L15:
	testl	%edx, %edx
	jne	.L14
	addl	$1, %eax
	jmp	.L13
	.p2align 4,,10
	.p2align 3
.L48:
	subl	$1, %eax
.L16:
	movl	%ecx, %r8d
	subl	$1, %ecx
	testl	%eax, %eax
	jne	.L49
	movl	$1, %eax
.L17:
	testl	%ecx, %ecx
	jne	.L16
	addl	$1, %eax
	jmp	.L15
	.p2align 4,,10
	.p2align 3
.L49:
	subl	$1, %eax
.L18:
	movl	%r8d, %edi
	subl	$1, %r8d
	testl	%eax, %eax
	jne	.L50
	movl	$1, %eax
.L19:
	testl	%r8d, %r8d
	jne	.L18
	addl	$1, %eax
	jmp	.L17
	.p2align 4,,10
	.p2align 3
.L50:
	leal	-1(%rax), %esi
	movl	%ecx, 12(%rsp)
	movl	%edx, 8(%rsp)
	movl	%r8d, 4(%rsp)
	call	ackermann
	movl	4(%rsp), %r8d
	movl	8(%rsp), %edx
	movl	12(%rsp), %ecx
	jmp	.L19
	.cfi_endproc
.LFE11:
	.size	ackermann, .-ackermann
	.section	.rodata.str1.1,"aMS",@progbits,1
.LC0:
	.string	"constant ackermann: %d\n"
	.section	.text.startup,"ax",@progbits
	.p2align 4
	.globl	main
	.type	main, @function
main:
.LFB12:
	.cfi_startproc
	movl	$10, %esi
	subq	$24, %rsp
	.cfi_def_cfa_offset 32
	movl	$3, %edi
	call	ackermann
	movl	$1, %esi
	testl	%eax, %eax
	jne	.L59
.L52:
	subl	$1, %esi
	movl	$1, %edi
	call	ackermann
.L53:
	addl	$1, %eax
	leaq	.LC0(%rip), %rdi
	movl	%eax, 12(%rsp)
	movl	12(%rsp), %esi
	xorl	%eax, %eax
	call	printf@PLT
	xorl	%eax, %eax
	addq	$24, %rsp
	.cfi_remember_state
	.cfi_def_cfa_offset 8
	ret
.L59:
	.cfi_restore_state
	leal	-1(%rax), %esi
	movl	$2, %edi
	call	ackermann
	movl	%eax, %esi
	testl	%eax, %eax
	jne	.L52
	movl	$1, %eax
	jmp	.L53
	.cfi_endproc
.LFE12:
	.size	main, .-main
	.ident	"GCC: (Debian 14.2.0-19) 14.2.0"
	.section	.note.GNU-stack,"",@progbits
