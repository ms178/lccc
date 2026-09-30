	.file	"csv_field_sum.c"
	.text
	.p2align 4
	.type	sum_fields.constprop.0, @function
sum_fields.constprop.0:
.LFB13:
	.cfi_startproc
	movl	%edi, %r11d
	xorl	%r10d, %r10d
	xorl	%edi, %edi
	xorl	%ecx, %ecx
	xorl	%r8d, %r8d
	xorl	%r9d, %r9d
	leaq	buf.0(%rip), %rdx
	jmp	.L3
	.p2align 4,,10
	.p2align 3
.L7:
	cmpb	$10, %al
	je	.L17
	testb	%al, %al
	je	.L1
	movzbl	1(%rdx), %ecx
	addq	$1, %rdx
	leal	-48(%rcx), %esi
	cmpb	$9, %sil
	ja	.L15
	xorl	%r10d, %r10d
	cmpb	$45, %al
	movl	%ecx, %eax
	sete	%r10b
	xorl	%ecx, %ecx
	.p2align 6
	.p2align 4
	.p2align 3
.L8:
	subl	$48, %eax
	leaq	(%rcx,%rcx,4), %rcx
	addq	$1, %rdx
	movl	$1, %edi
	movsbq	%al, %rax
	leaq	(%rax,%rcx,2), %rcx
.L3:
	movzbl	(%rdx), %eax
	leal	-48(%rax), %esi
	cmpb	$9, %sil
	jbe	.L8
	cmpl	%r11d, %r8d
	jne	.L6
	testl	%edi, %edi
	je	.L6
	movq	%rcx, %rsi
	negq	%rsi
	testl	%r10d, %r10d
	cmovne	%rsi, %rcx
	addq	%rcx, %r9
.L6:
	cmpb	$44, %al
	jne	.L7
	movzbl	1(%rdx), %eax
	addq	$1, %rdx
	addl	$1, %r8d
	leal	-48(%rax), %ecx
	cmpb	$9, %cl
	ja	.L6
	xorl	%r10d, %r10d
	xorl	%ecx, %ecx
	jmp	.L8
	.p2align 4,,10
	.p2align 3
.L17:
	testb	%al, %al
	je	.L1
	movzbl	1(%rdx), %eax
	addq	$1, %rdx
	leal	-48(%rax), %ecx
	cmpb	$9, %cl
	jbe	.L16
	xorl	%r8d, %r8d
	jmp	.L6
	.p2align 4,,10
	.p2align 3
.L15:
	movl	%ecx, %eax
	jmp	.L6
	.p2align 4,,10
	.p2align 3
.L16:
	xorl	%r10d, %r10d
	xorl	%r8d, %r8d
	xorl	%ecx, %ecx
	jmp	.L8
	.p2align 4,,10
	.p2align 3
.L1:
	movq	%r9, %rax
	ret
	.cfi_endproc
.LFE13:
	.size	sum_fields.constprop.0, .-sum_fields.constprop.0
	.section	.rodata.str1.1,"aMS",@progbits,1
.LC0:
	.string	"%ld %ld %ld\n"
	.section	.text.startup,"ax",@progbits
	.p2align 4
	.globl	main
	.type	main, @function
main:
.LFB12:
	.cfi_startproc
	pushq	%rbx
	.cfi_def_cfa_offset 16
	.cfi_offset 3, -16
	movl	$102, %r8d
	xorl	%r11d, %r11d
	movl	$3435973837, %r9d
	leaq	buf.0(%rip), %rdi
	movl	$500, %r10d
	subq	$16, %rsp
	.cfi_def_cfa_offset 32
	.p2align 4
	.p2align 3
.L29:
	leal	-102(%r8), %eax
	movq	%rax, %rcx
	imulq	$274877907, %rax, %rax
	shrq	$38, %rax
	imull	$1000, %eax, %esi
	movl	%ecx, %eax
	subl	%esi, %eax
	movl	%eax, %ecx
	subl	$500, %ecx
	js	.L48
	jne	.L37
	movb	$48, %dl
	movslq	%r11d, %rax
	movl	$1, %ebx
	movb	%dl, (%rdi,%rax)
.L38:
	addl	%r11d, %ebx
	leal	-85(%r8), %r11d
.L35:
	movslq	%ebx, %rax
	leal	1(%rbx), %esi
	movb	$44, (%rdi,%rax)
	movl	%r11d, %eax
	imulq	$274877907, %rax, %rax
	shrq	$38, %rax
	imull	$1000, %eax, %ecx
	movl	%r11d, %eax
	subl	%ecx, %eax
	movl	%eax, %ecx
	subl	$500, %ecx
	js	.L49
	jne	.L31
	movslq	%esi, %rax
	movb	$48, %dl
	movb	%dl, (%rdi,%rax)
	movl	$1, %eax
.L32:
	addl	%eax, %esi
	addl	$17, %r11d
	movl	%esi, %ebx
	cmpl	%r11d, %r8d
	jne	.L35
	leal	1(%rsi), %r11d
	addl	$131, %r8d
	movslq	%esi, %rsi
	movb	$10, (%rdi,%rsi)
	cmpl	$8486, %r8d
	jne	.L29
	movslq	%r11d, %r11
	movb	$0, (%rdi,%r11)
	movl	$5, %edi
	call	sum_fields.constprop.0
	movl	$3, %edi
	movq	%rax, 8(%rsp)
	call	sum_fields.constprop.0
	xorl	%edi, %edi
	movq	%rax, %rbx
	call	sum_fields.constprop.0
	movq	8(%rsp), %rcx
	movq	%rbx, %rdx
	leaq	.LC0(%rip), %rdi
	movq	%rax, %rsi
	xorl	%eax, %eax
	call	printf@PLT
	addq	$16, %rsp
	.cfi_remember_state
	.cfi_def_cfa_offset 16
	xorl	%eax, %eax
	popq	%rbx
	.cfi_def_cfa_offset 8
	ret
	.p2align 4,,10
	.p2align 3
.L49:
	.cfi_restore_state
	movslq	%esi, %rsi
	movl	%r10d, %ecx
	movb	$45, (%rdi,%rsi)
	subl	%eax, %ecx
	leal	2(%rbx), %esi
.L31:
	movl	%ecx, %eax
	imulq	%r9, %rax
	shrq	$35, %rax
	leal	(%rax,%rax,4), %ebx
	addl	%ebx, %ebx
	subl	%ebx, %ecx
	movb	%cl, %dl
	addb	$48, %dl
	testl	%eax, %eax
	je	.L33
	movl	%eax, %ecx
	imulq	%r9, %rcx
	shrq	$35, %rcx
	leal	(%rcx,%rcx,4), %ebx
	addl	%ebx, %ebx
	subl	%ebx, %eax
	addl	$48, %eax
	movb	%al, %dh
	testl	%ecx, %ecx
	je	.L34
	addl	$48, %ecx
	andq	$-16711681, %rdx
	movslq	%esi, %rax
	movzbl	%cl, %ecx
	salq	$16, %rcx
	orq	%rcx, %rdx
	movl	%edx, %ecx
	sall	$8, %ecx
	sarl	$24, %ecx
	movb	%cl, (%rdi,%rax)
	leal	1(%rsi), %eax
	cltq
	movb	%dh, (%rdi,%rax)
	leal	2(%rsi), %eax
	cltq
	movb	%dl, (%rdi,%rax)
	movl	$3, %eax
	jmp	.L32
.L48:
	movslq	%r11d, %rcx
	addl	$1, %r11d
	movb	$45, (%rdi,%rcx)
	movl	%r10d, %ecx
	subl	%eax, %ecx
.L37:
	movl	%ecx, %eax
	imulq	%r9, %rax
	shrq	$35, %rax
	leal	(%rax,%rax,4), %esi
	addl	%esi, %esi
	subl	%esi, %ecx
	movb	%cl, %dl
	addb	$48, %dl
	testl	%eax, %eax
	je	.L50
	movl	%eax, %ecx
	imulq	%r9, %rcx
	shrq	$35, %rcx
	leal	(%rcx,%rcx,4), %esi
	addl	%esi, %esi
	subl	%esi, %eax
	addl	$48, %eax
	movb	%al, %dh
	testl	%ecx, %ecx
	je	.L51
	leal	48(%rcx), %eax
	andq	$-16711681, %rdx
	movl	$3, %ebx
	movzbl	%al, %eax
	salq	$16, %rax
	orq	%rax, %rdx
	movslq	%r11d, %rax
	movl	%edx, %ecx
	sall	$8, %ecx
	sarl	$24, %ecx
	movb	%cl, (%rdi,%rax)
	leal	1(%r11), %eax
	cltq
	movb	%dh, (%rdi,%rax)
	leal	2(%r11), %eax
	cltq
	movb	%dl, (%rdi,%rax)
	jmp	.L38
.L33:
	movslq	%esi, %rax
	movb	%dl, (%rdi,%rax)
	movl	$1, %eax
	jmp	.L32
.L34:
	movslq	%esi, %rax
	movb	%dh, (%rdi,%rax)
	leal	1(%rsi), %eax
	cltq
	movb	%dl, (%rdi,%rax)
	movl	$2, %eax
	jmp	.L32
.L50:
	movslq	%r11d, %rax
	movl	$1, %ebx
	movb	%dl, (%rdi,%rax)
	jmp	.L38
.L51:
	movslq	%r11d, %rax
	movl	$2, %ebx
	movb	%dh, (%rdi,%rax)
	leal	1(%r11), %eax
	cltq
	movb	%dl, (%rdi,%rax)
	jmp	.L38
	.cfi_endproc
.LFE12:
	.size	main, .-main
	.local	buf.0
	.comm	buf.0,4096,32
	.ident	"GCC: (Debian 14.2.0-19) 14.2.0"
	.section	.note.GNU-stack,"",@progbits
