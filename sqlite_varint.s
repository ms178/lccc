	.file	"sqlite_varint.c"
	.text
	.p2align 4
	.type	sqlite_put_varint, @function
sqlite_put_varint:
.LFB11:
	.cfi_startproc
	movq	%rdi, %rdx
	movq	%rsi, %rax
	cmpq	$127, %rsi
	jbe	.L28
	cmpq	$16383, %rsi
	jbe	.L29
	movq	%rsi, %rdi
	shrq	$56, %rdi
	jne	.L5
	movq	%rsi, %rcx
	shrq	$7, %rcx
	orl	$-128, %ecx
	movb	%cl, -9(%rsp)
	movq	%rsi, %rcx
	shrq	$14, %rcx
	orl	$-128, %ecx
	movb	%cl, -8(%rsp)
	movq	%rsi, %rcx
	shrq	$21, %rcx
	je	.L30
	orl	$-128, %ecx
	movb	%cl, -7(%rsp)
	movq	%rsi, %rcx
	shrq	$28, %rcx
	je	.L31
	orl	$-128, %ecx
	movb	%cl, -6(%rsp)
	movq	%rsi, %rcx
	shrq	$35, %rcx
	je	.L32
	orl	$-128, %ecx
	movb	%cl, -5(%rsp)
	movq	%rsi, %rcx
	shrq	$42, %rcx
	je	.L33
	orl	$-128, %ecx
	movb	%cl, -4(%rsp)
	movq	%rsi, %rcx
	shrq	$49, %rcx
	je	.L34
	orl	$-128, %ecx
	andl	$127, %eax
	movb	%cl, -3(%rsp)
	movl	$8, %ecx
	movb	%al, -10(%rsp)
	vmovq	-10(%rsp), %xmm0
	vpshufb	.LC1(%rip), %xmm0, %xmm0
	vmovq	%xmm0, (%rdx)
.L1:
	movl	%ecx, %eax
	ret
	.p2align 4,,10
	.p2align 3
.L28:
	movl	$1, %ecx
	movb	%sil, (%rdi)
	movl	%ecx, %eax
	ret
	.p2align 4,,10
	.p2align 3
.L5:
	movq	%rax, %r11
	movq	%rax, %rcx
	movq	%rax, %r10
	movq	%rax, %r9
	shrq	$15, %r11
	andl	$65280, %ecx
	movq	%rax, %r8
	movq	%rax, %rdi
	shrq	$22, %r10
	movzbl	%r11b, %r11d
	shrq	$29, %r9
	movb	%sil, 8(%rdx)
	orq	%r11, %rcx
	movzbl	%r10b, %r10d
	movzbl	%r9b, %r9d
	shrq	$36, %r8
	salq	$8, %rcx
	movzbl	%r8b, %r8d
	shrq	$43, %rdi
	vmovq	.LC0(%rip), %xmm0
	shrq	$50, %rsi
	orq	%r10, %rcx
	movzbl	%dil, %edi
	shrq	$57, %rax
	salq	$8, %rcx
	movzbl	%sil, %esi
	orq	%r9, %rcx
	salq	$8, %rcx
	orq	%r8, %rcx
	salq	$8, %rcx
	orq	%rdi, %rcx
	salq	$8, %rcx
	orq	%rsi, %rcx
	salq	$8, %rcx
	orq	%rax, %rcx
	vmovq	%rcx, %xmm1
	movl	$9, %ecx
	vpor	%xmm1, %xmm0, %xmm0
	movl	%ecx, %eax
	vmovq	%xmm0, (%rdx)
	ret
	.p2align 4,,10
	.p2align 3
.L29:
	movq	%rsi, %rcx
	andl	$127, %eax
	shrq	$7, %rcx
	movb	%al, 1(%rdi)
	orl	$-128, %ecx
	movb	%cl, (%rdi)
	movl	$2, %ecx
	movl	%ecx, %eax
	ret
.L34:
	andl	$127, %eax
	movl	$7, %r8d
	movl	$6, %esi
	movl	$6, %edi
	movl	$7, %ecx
.L10:
	movb	%al, -10(%rsp)
	movslq	%edi, %rax
	vmovd	-13(%rsp,%rax), %xmm0
	leal	-4(%rsi), %eax
	vpshufb	.LC2(%rip), %xmm0, %xmm0
	vmovd	%xmm0, (%rdx)
	cmpl	$4, %r8d
	je	.L1
	cltq
	movzbl	-10(%rsp,%rax), %eax
	movb	%al, 4(%rdx)
	leal	-5(%rsi), %eax
	cmpl	$4, %esi
	je	.L1
	cltq
	movzbl	-10(%rsp,%rax), %eax
	movb	%al, 5(%rdx)
	cmpl	$5, %esi
	je	.L1
	movl	$6, %eax
.L8:
	movzbl	-10(%rsp), %esi
	movb	%sil, (%rdx,%rax)
	movl	%ecx, %eax
	ret
	.p2align 4,,10
	.p2align 3
.L30:
	andl	$127, %eax
	movl	$3, %ecx
	movb	%al, -10(%rsp)
	movbew	-9(%rsp), %ax
	movw	%ax, (%rdx)
	movl	$2, %eax
	jmp	.L8
.L31:
	andl	$127, %eax
	movl	$4, %r8d
	movl	$3, %esi
	movl	$3, %edi
	movl	$4, %ecx
	jmp	.L10
.L32:
	andl	$127, %eax
	movl	$5, %r8d
	movl	$4, %esi
	movl	$4, %edi
	movl	$5, %ecx
	jmp	.L10
.L33:
	andl	$127, %eax
	movl	$6, %r8d
	movl	$5, %esi
	movl	$5, %edi
	movl	$6, %ecx
	jmp	.L10
	.cfi_endproc
.LFE11:
	.size	sqlite_put_varint, .-sqlite_put_varint
	.section	.rodata.str1.1,"aMS",@progbits,1
.LC3:
	.string	"%llx\n"
	.section	.text.startup,"ax",@progbits
	.p2align 4
	.globl	main
	.type	main, @function
main:
.LFB16:
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
	leaq	values.0(%rip), %r12
	pushq	%rbp
	.cfi_def_cfa_offset 48
	.cfi_offset 6, -48
	leaq	88(%r12), %rbp
	pushq	%rbx
	.cfi_def_cfa_offset 56
	.cfi_offset 3, -56
	subq	$24, %rsp
	.cfi_def_cfa_offset 80
	leaq	7(%rsp), %rbx
.L46:
	movq	(%r12), %r13
	movq	%rbx, %rdi
	movq	%r13, %rsi
	call	sqlite_put_varint
	movzbl	7(%rsp), %ecx
	movl	%eax, %edx
	testb	%cl, %cl
	jns	.L79
	movsbl	8(%rsp), %eax
	movzbl	%cl, %ecx
	testb	%al, %al
	jns	.L80
	movzbl	9(%rsp), %r8d
	sall	$14, %ecx
	movzbl	%al, %edi
	orl	%r8d, %ecx
	andl	$128, %r8d
	movl	%ecx, %esi
	je	.L81
	movzbl	10(%rsp), %eax
	sall	$14, %edi
	orl	%edi, %eax
	movl	%eax, %ecx
	andl	$2080895, %ecx
	testb	$-128, %al
	je	.L82
	movl	%esi, %r8d
	movzbl	11(%rsp), %edi
	sall	$14, %r8d
	andl	$-266354688, %r8d
	movl	%edi, %eax
	orl	%edi, %r8d
	andl	$128, %edi
	je	.L83
	sall	$7, %esi
	movl	%ecx, %edi
	andl	$266354560, %esi
	sall	$14, %edi
	orl	%ecx, %esi
	movzbl	12(%rsp), %ecx
	orl	%ecx, %edi
	andl	$128, %ecx
	movl	%r8d, %ecx
	je	.L84
	movzbl	13(%rsp), %r8d
	sall	$14, %ecx
	orl	%r8d, %ecx
	andl	$128, %r8d
	je	.L85
	movzbl	14(%rsp), %r8d
	sall	$14, %edi
	andl	$2080895, %ecx
	orl	%r8d, %edi
	andl	$128, %r8d
	je	.L86
	sall	$4, %esi
	andl	$127, %eax
	sall	$8, %edi
	shrl	$3, %eax
	sall	$15, %ecx
	andl	$532709120, %edi
	orl	%esi, %eax
	movzbl	15(%rsp), %esi
	orl	%edi, %ecx
	salq	$32, %rax
	orl	%esi, %ecx
	orq	%rcx, %rax
	movl	$9, %ecx
	jmp	.L37
.L79:
	movsbq	%cl, %rax
	movl	$1, %ecx
.L37:
	cmpl	%ecx, %edx
	jne	.L45
.L94:
	cmpq	%rax, %r13
	jne	.L45
	addq	$8, %r12
	cmpq	%rbp, %r12
	jne	.L46
	xorl	%r15d, %r15d
	xorl	%r12d, %r12d
	movl	$1831565813, %r14d
	leaq	sqlite_varint_offsets(%rip), %rbp
	leaq	sqlite_varint_bytes(%rip), %rbx
	leaq	.L50(%rip), %r13
	.p2align 4
	.p2align 3
.L59:
	movl	%r15d, %eax
	imull	$1664525, %r14d, %ecx
	imulq	$954437177, %rax, %rax
	leal	1013904223(%rcx), %r14d
	shrq	$33, %rax
	leal	(%rax,%rax,8), %esi
	movl	%r15d, %eax
	subl	%esi, %eax
	cmpl	$7, %eax
	ja	.L48
	movslq	0(%r13,%rax,4), %rax
	addq	%r13, %rax
	jmp	*%rax
	.section	.rodata
	.align 4
	.align 4
.L50:
	.long	.L57-.L50
	.long	.L56-.L50
	.long	.L55-.L50
	.long	.L54-.L50
	.long	.L53-.L50
	.long	.L52-.L50
	.long	.L51-.L50
	.long	.L49-.L50
	.section	.text.startup
.L51:
	movabsq	$4398046511104, %rax
	movl	%r14d, %esi
	salq	$17, %rsi
	orq	%rax, %rsi
	.p2align 4
	.p2align 3
.L58:
	movl	%r12d, %edi
	movl	%r12d, 0(%rbp,%r15,4)
	addq	$1, %r15
	addq	%rbx, %rdi
	call	sqlite_put_varint
	addl	%eax, %r12d
	cmpq	$262144, %r15
	jne	.L59
	movl	%r12d, sqlite_varint_used(%rip)
	xorl	%r8d, %r8d
	xorl	%edi, %edi
.L60:
	xorl	%ecx, %ecx
	jmp	.L70
	.p2align 4,,10
	.p2align 3
.L63:
	movzbl	2(%r9), %r11d
	sall	$14, %eax
	movzbl	%dl, %r10d
	orl	%r11d, %eax
	andl	$128, %r11d
	je	.L87
	movzbl	3(%r9), %edx
	sall	$14, %r10d
	orl	%r10d, %edx
	movl	%edx, %r10d
	andl	$2080895, %r10d
	andl	$128, %edx
	je	.L88
	movl	%eax, %r11d
	movzbl	4(%r9), %r13d
	sall	$14, %r11d
	andl	$-266354688, %r11d
	movl	%r13d, %edx
	orl	%r13d, %r11d
	andl	$128, %r13d
	je	.L89
	sall	$7, %eax
	movzbl	5(%r9), %r13d
	andl	$266354560, %eax
	orl	%r10d, %eax
	sall	$14, %r10d
	orl	%r13d, %r10d
	andl	$128, %r13d
	je	.L90
	movzbl	6(%r9), %r13d
	sall	$14, %r11d
	orl	%r13d, %r11d
	andl	$128, %r13d
	je	.L91
	movzbl	7(%r9), %r13d
	sall	$14, %r10d
	andl	$2080895, %r11d
	orl	%r13d, %r10d
	andl	$128, %r13d
	je	.L92
	sall	$8, %r10d
	sall	$4, %eax
	andl	$532709120, %r10d
	sall	$15, %r11d
	orl	%r10d, %r11d
	movl	%eax, %r10d
	movl	%edx, %eax
	movzbl	8(%r9), %edx
	andl	$127, %eax
	shrl	$3, %eax
	orl	%r11d, %edx
	orl	%r10d, %eax
	salq	$32, %rax
	orq	%rdx, %rax
	movl	$9, %edx
	.p2align 6
	.p2align 4
	.p2align 3
.L62:
	andl	$31, %esi
	addq	$1, %rcx
	shlx	%rsi, %rdx, %rsi
	xorq	%rax, %rsi
	addq	%rsi, %rdi
	cmpq	$262144, %rcx
	je	.L93
.L70:
	movl	0(%rbp,%rcx,4), %r9d
	movl	%ecx, %esi
	movl	$1, %edx
	addq	%rbx, %r9
	movsbq	(%r9), %rax
	testb	%al, %al
	jns	.L62
	movsbl	1(%r9), %edx
	movzbl	%al, %eax
	testb	%dl, %dl
	js	.L63
	sall	$7, %eax
	andl	$16256, %eax
	orl	%edx, %eax
	movl	$2, %edx
	jmp	.L62
.L52:
	movl	%r14d, %esi
	salq	$10, %rsi
	btsq	$35, %rsi
	jmp	.L58
.L53:
	movl	%r14d, %esi
	salq	$3, %rsi
	orq	$268435456, %rsi
	jmp	.L58
.L54:
	movl	%r14d, %esi
	andl	$266338303, %esi
	orl	$2097152, %esi
	jmp	.L58
.L55:
	movl	%r14d, %esi
	andl	$2080767, %esi
	orl	$16384, %esi
	jmp	.L58
.L56:
	movl	%r14d, %esi
	andl	$16255, %esi
	orl	$128, %esi
	jmp	.L58
.L57:
	movl	%r14d, %esi
	andl	$127, %esi
	jmp	.L58
.L49:
	movabsq	$562949953421312, %rax
	movl	%r14d, %esi
	salq	$24, %rsi
	orq	%rax, %rsi
	jmp	.L58
	.p2align 4,,10
	.p2align 3
.L87:
	andl	$127, %edx
	andl	$2080895, %eax
	sall	$7, %edx
	orl	%eax, %edx
	movl	%edx, %eax
	movl	$3, %edx
	jmp	.L62
	.p2align 4,,10
	.p2align 3
.L88:
	sall	$7, %eax
	movl	$4, %edx
	andl	$266354560, %eax
	orl	%r10d, %eax
	jmp	.L62
	.p2align 4,,10
	.p2align 3
.L89:
	shrl	$18, %eax
	sall	$7, %r10d
	movl	$5, %edx
	andl	$7, %eax
	orl	%r11d, %r10d
	salq	$32, %rax
	orq	%r10, %rax
	jmp	.L62
	.p2align 4,,10
	.p2align 3
.L90:
	sall	$7, %r11d
	shrl	$18, %eax
	movl	$6, %edx
	andl	$266354560, %r11d
	salq	$32, %rax
	orl	%r10d, %r11d
	orq	%r11, %rax
	jmp	.L62
	.p2align 4,,10
	.p2align 3
.L92:
	sall	$7, %r11d
	shrl	$4, %eax
	movl	$8, %edx
	andl	$-266354561, %r10d
	salq	$32, %rax
	orl	%r11d, %r10d
	orq	%r10, %rax
	jmp	.L62
	.p2align 4,,10
	.p2align 3
.L91:
	sall	$7, %r10d
	andl	$-266354561, %r11d
	shrl	$11, %eax
	movl	$7, %edx
	andl	$266354560, %r10d
	salq	$32, %rax
	orl	%r11d, %r10d
	orq	%r10, %rax
	jmp	.L62
.L93:
	movl	%r8d, %eax
	addl	$104729, %r8d
	andl	$262143, %eax
	movl	0(%rbp,%rax,4), %eax
	xorb	$1, (%rbx,%rax)
	cmpl	$2513496, %r8d
	jne	.L60
	testl	%r12d, %r12d
	je	.L73
	movq	%rdi, %rsi
	xorl	%eax, %eax
	leaq	.LC3(%rip), %rdi
	call	printf@PLT
	xorl	%eax, %eax
.L35:
	addq	$24, %rsp
	.cfi_remember_state
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
.L48:
	.cfi_restore_state
	movabsq	$-9223372036854775808, %rax
	movl	%r14d, %esi
	salq	$25, %rsi
	orq	%r15, %rsi
	orq	%rax, %rsi
	jmp	.L58
.L80:
	sall	$7, %ecx
	andl	$16256, %ecx
	orl	%ecx, %eax
	movl	$2, %ecx
	cmpl	%ecx, %edx
	je	.L94
.L45:
	movl	$2, %eax
	jmp	.L35
.L81:
	andl	$127, %eax
	andl	$2080895, %esi
	movl	$3, %ecx
	sall	$7, %eax
	orl	%esi, %eax
	jmp	.L37
.L86:
	movl	%esi, %eax
	sall	$7, %ecx
	andl	$-266354561, %edi
	shrl	$4, %eax
	orl	%edi, %ecx
	salq	$32, %rax
	orq	%rcx, %rax
	movl	$8, %ecx
	jmp	.L37
.L73:
	movl	$3, %eax
	jmp	.L35
.L82:
	movl	%esi, %eax
	sall	$7, %eax
	andl	$266354560, %eax
	orl	%ecx, %eax
	movl	$4, %ecx
	jmp	.L37
.L83:
	movl	%esi, %eax
	sall	$7, %ecx
	shrl	$18, %eax
	orl	%r8d, %ecx
	andl	$7, %eax
	salq	$32, %rax
	orq	%rcx, %rax
	movl	$5, %ecx
	jmp	.L37
.L84:
	sall	$7, %ecx
	movl	%esi, %eax
	andl	$266354560, %ecx
	shrl	$18, %eax
	orl	%edi, %ecx
	salq	$32, %rax
	orq	%rcx, %rax
	movl	$6, %ecx
	jmp	.L37
.L85:
	sall	$7, %edi
	movl	%esi, %eax
	andl	$-266354561, %ecx
	shrl	$11, %eax
	andl	$266354560, %edi
	orl	%edi, %ecx
	salq	$32, %rax
	orq	%rcx, %rax
	movl	$7, %ecx
	jmp	.L37
	.cfi_endproc
.LFE16:
	.size	main, .-main
	.section	.rodata
	.align 32
	.type	values.0, @object
	.size	values.0, 88
values.0:
	.quad	0
	.quad	127
	.quad	128
	.quad	16383
	.quad	16384
	.quad	2097151
	.quad	2097152
	.quad	268435455
	.quad	268435456
	.quad	9223372036854775807
	.quad	-1
	.local	sqlite_varint_used
	.comm	sqlite_varint_used,4,4
	.local	sqlite_varint_offsets
	.comm	sqlite_varint_offsets,1048576,32
	.local	sqlite_varint_bytes
	.comm	sqlite_varint_bytes,2359296,32
	.set	.LC0,.LC1+8
	.section	.rodata.cst16,"aM",@progbits,16
	.align 16
.LC1:
	.byte	7
	.byte	6
	.byte	5
	.byte	4
	.byte	3
	.byte	2
	.byte	1
	.byte	0
	.byte	-128
	.byte	-128
	.byte	-128
	.byte	-128
	.byte	-128
	.byte	-128
	.byte	-128
	.byte	-128
	.align 16
.LC2:
	.byte	3
	.byte	2
	.byte	1
	.byte	0
	.byte	-128
	.byte	-128
	.byte	-128
	.byte	-128
	.byte	-128
	.byte	-128
	.byte	-128
	.byte	-128
	.byte	-128
	.byte	-128
	.byte	-128
	.byte	-128
	.ident	"GCC: (Debian 14.2.0-19) 14.2.0"
	.section	.note.GNU-stack,"",@progbits
