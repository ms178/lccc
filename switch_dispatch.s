	.file	"switch_dispatch.c"
	.text
	.section	.rodata.str1.1,"aMS",@progbits,1
.LC0:
	.string	"switch_dispatch sum: %ld\n"
	.section	.text.startup,"ax",@progbits
	.p2align 4
	.globl	main
	.type	main, @function
main:
.LFB12:
	.cfi_startproc
	subq	$8, %rsp
	.cfi_def_cfa_offset 16
	xorl	%ecx, %ecx
	movl	$777, %eax
	xorl	%edi, %edi
	leaq	.L4(%rip), %r8
	.p2align 6
	.p2align 4
	.p2align 3
.L20:
	imull	$1664525, %eax, %eax
	addl	$1013904223, %eax
	movl	%eax, %edx
	movzwl	%ax, %esi
	shrl	$16, %edx
	andl	$15, %edx
	movslq	(%r8,%rdx,4), %rdx
	addq	%r8, %rdx
	jmp	*%rdx
	.section	.rodata
	.align 4
	.align 4
.L4:
	.long	.L2-.L4
	.long	.L23-.L4
	.long	.L17-.L4
	.long	.L16-.L4
	.long	.L15-.L4
	.long	.L14-.L4
	.long	.L13-.L4
	.long	.L12-.L4
	.long	.L11-.L4
	.long	.L10-.L4
	.long	.L9-.L4
	.long	.L8-.L4
	.long	.L7-.L4
	.long	.L6-.L4
	.long	.L5-.L4
	.long	.L3-.L4
	.section	.text.startup
	.p2align 4,,10
	.p2align 3
.L10:
	notl	%esi
.L23:
	movl	%ecx, %edx
	subl	%esi, %edx
	.p2align 4
	.p2align 3
.L19:
	movslq	%edx, %rdx
	addl	$1, %ecx
	addq	%rdx, %rdi
	cmpl	$50000000, %ecx
	jne	.L20
	movq	%rdi, %rsi
	xorl	%eax, %eax
	leaq	.LC0(%rip), %rdi
	call	printf@PLT
	xorl	%eax, %eax
	addq	$8, %rsp
	.cfi_remember_state
	.cfi_def_cfa_offset 8
	ret
	.p2align 4,,10
	.p2align 3
.L3:
	.cfi_restore_state
	leal	1(%rsi,%rcx), %edx
	jmp	.L19
	.p2align 4,,10
	.p2align 3
.L5:
	andl	%ecx, %esi
	leal	2(%rsi), %edx
	jmp	.L19
	.p2align 4,,10
	.p2align 3
.L6:
	orl	%ecx, %esi
	leal	-1(%rsi), %edx
	jmp	.L19
	.p2align 4,,10
	.p2align 3
.L7:
	xorl	%ecx, %esi
	leal	1(%rsi), %edx
	jmp	.L19
	.p2align 4,,10
	.p2align 3
.L8:
	movl	%ecx, %edx
	subl	%esi, %edx
	leal	(%rdx,%rdx,4), %edx
	jmp	.L19
	.p2align 4,,10
	.p2align 3
.L9:
	addl	%ecx, %esi
	leal	(%rsi,%rsi,2), %edx
	jmp	.L19
	.p2align 4,,10
	.p2align 3
.L11:
	movl	%ecx, %edx
	notl	%edx
	addl	%esi, %edx
	jmp	.L19
	.p2align 4,,10
	.p2align 3
.L12:
	movl	%eax, %edx
	andl	$7, %edx
	sarx	%edx, %ecx, %edx
	jmp	.L19
	.p2align 4,,10
	.p2align 3
.L13:
	movl	%eax, %edx
	andl	$7, %edx
	shlx	%edx, %ecx, %edx
	jmp	.L19
	.p2align 4,,10
	.p2align 3
.L14:
	andl	%ecx, %esi
	movl	%esi, %edx
	jmp	.L19
	.p2align 4,,10
	.p2align 3
.L15:
	orl	%ecx, %esi
	movl	%esi, %edx
	jmp	.L19
	.p2align 4,,10
	.p2align 3
.L16:
	xorl	%ecx, %esi
	movl	%esi, %edx
	jmp	.L19
	.p2align 4,,10
	.p2align 3
.L17:
	imull	%ecx, %esi
	movl	%esi, %edx
	jmp	.L19
.L2:
	leal	(%rsi,%rcx), %edx
	jmp	.L19
	.cfi_endproc
.LFE12:
	.size	main, .-main
	.ident	"GCC: (Debian 14.2.0-19) 14.2.0"
	.section	.note.GNU-stack,"",@progbits
