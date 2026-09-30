	.file	"expat_xml_scan.c"
	.text
	.p2align 4
	.type	expat_utf8_name_length, @function
expat_utf8_name_length:
.LFB13:
	.cfi_startproc
	movq	%rsi, %rcx
	cmpq	%rsi, %rdi
	jnb	.L21
	movq	%rdi, %rax
.L20:
	movzbl	(%rax), %edx
	cmpb	$-17, %dl
	ja	.L3
	cmpb	$-33, %dl
	ja	.L4
	testb	%dl, %dl
	js	.L33
	movl	%edx, %esi
	andl	$-33, %esi
	subl	$65, %esi
	cmpb	$25, %sil
	ja	.L34
.L9:
	addq	$1, %rax
.L12:
	cmpq	%rcx, %rax
	jb	.L20
.L31:
	subq	%rdi, %rax
	ret
	.p2align 4,,10
	.p2align 3
.L3:
	addl	$16, %edx
	cmpb	$4, %dl
	ja	.L31
	movq	%rcx, %rdx
	subq	%rax, %rdx
	cmpq	$3, %rdx
	jle	.L31
	movzbl	1(%rax), %edx
	andl	$-64, %edx
	cmpb	$-128, %dl
	jne	.L31
	movzbl	2(%rax), %edx
	andl	$-64, %edx
	cmpb	$-128, %dl
	jne	.L31
	movzbl	3(%rax), %edx
	andl	$-64, %edx
	cmpb	$-128, %dl
	jne	.L31
	movl	$4, %edx
	jmp	.L18
	.p2align 4,,10
	.p2align 3
.L33:
	addl	$62, %edx
	cmpb	$29, %dl
	ja	.L31
	movq	%rcx, %rdx
	subq	%rax, %rdx
	cmpq	$1, %rdx
	je	.L31
	movzbl	1(%rax), %edx
	andl	$-64, %edx
	cmpb	$-128, %dl
	jne	.L31
	movl	$2, %edx
.L18:
	addq	%rdx, %rax
	jmp	.L12
	.p2align 4,,10
	.p2align 3
.L4:
	movq	%rcx, %rdx
	subq	%rax, %rdx
	cmpq	$2, %rdx
	jle	.L31
	movzbl	1(%rax), %edx
	andl	$-64, %edx
	cmpb	$-128, %dl
	jne	.L31
	movzbl	2(%rax), %edx
	andl	$-64, %edx
	cmpb	$-128, %dl
	jne	.L31
	movl	$3, %edx
	jmp	.L18
	.p2align 4,,10
	.p2align 3
.L34:
	subl	$45, %edx
	cmpb	$50, %dl
	ja	.L31
	movabsq	$-1125899906859004, %rsi
	btq	%rdx, %rsi
	jc	.L31
	jmp	.L9
.L21:
	xorl	%eax, %eax
	ret
	.cfi_endproc
.LFE13:
	.size	expat_utf8_name_length, .-expat_utf8_name_length
	.section	.rodata.str1.1,"aMS",@progbits,1
.LC3:
	.string	"%lu\n"
	.section	.text.startup,"ax",@progbits
	.p2align 4
	.globl	main
	.type	main, @function
main:
.LFB17:
	.cfi_startproc
	pushq	%rbp
	.cfi_def_cfa_offset 16
	.cfi_offset 6, -16
	leaq	7+ascii_name.2(%rip), %rsi
	movl	$2, %r8d
	leaq	-7(%rsi), %rdi
	movq	%rsp, %rbp
	.cfi_def_cfa_register 6
	pushq	%r14
	pushq	%r13
	pushq	%r12
	pushq	%rbx
	.cfi_offset 14, -24
	.cfi_offset 13, -32
	.cfi_offset 12, -40
	.cfi_offset 3, -48
	call	expat_utf8_name_length
	cmpq	$7, %rax
	je	.L62
.L35:
	popq	%rbx
	movl	%r8d, %eax
	popq	%r12
	popq	%r13
	popq	%r14
	popq	%rbp
	.cfi_remember_state
	.cfi_def_cfa 7, 8
	ret
.L62:
	.cfi_restore_state
	leaq	8+utf8_name.1(%rip), %rsi
	leaq	-8(%rsi), %rdi
	call	expat_utf8_name_length
	cmpq	$8, %rax
	jne	.L35
	leaq	expat_xml_data(%rip), %r10
	vmovdqa	.LC0(%rip), %ymm1
	vmovdqa	.LC1(%rip), %ymm0
	leaq	1048560(%r10), %rdx
	movq	%r10, %rax
	.p2align 5
	.p2align 4
	.p2align 3
.L37:
	vmovdqu	%ymm1, (%rax)
	addq	$60, %rax
	vmovdqu	%ymm0, -32(%rax)
	cmpq	%rdx, %rax
	jne	.L37
	vpbroadcastd	.LC5(%rip), %xmm0
	xorl	%r11d, %r11d
	leaq	524224+expat_xml_data(%rip), %r12
	movabsq	$1469598103934665603, %rbx
	leaq	524352(%r12), %r14
	movabsq	$1099511628211, %r9
	vmovdqa	%xmm0, 1048560+expat_xml_data(%rip)
	.p2align 4
	.p2align 3
.L38:
	movq	%rbx, %r13
	leaq	expat_xml_data(%rip), %rdi
	jmp	.L49
	.p2align 4,,10
	.p2align 3
.L63:
	cmpb	$39, %r8b
	je	.L53
	movl	%r8d, %eax
	andl	$-33, %eax
	subl	$65, %eax
	cmpb	$25, %al
	jbe	.L45
	cmpb	$95, %r8b
	sete	%al
	cmpb	$58, %r8b
	sete	%dl
	orb	%dl, %al
	jne	.L45
	cmpb	$-63, %r8b
	jbe	.L46
.L45:
	movq	%r14, %rsi
	call	expat_utf8_name_length
	testq	%rax, %rax
	je	.L42
	addq	%rax, %r8
	addq	%rax, %rdi
	xorq	%r13, %r8
	imulq	%r9, %r8
	movq	%r8, %r13
.L44:
	cmpq	%r14, %rdi
	jnb	.L42
.L49:
	movzbl	(%rdi), %r8d
	cmpb	$34, %r8b
	jne	.L63
.L53:
	leaq	1(%rdi), %rax
	cmpq	%r14, %rax
	jb	.L41
	jmp	.L42
	.p2align 4
	.p2align 4,,10
	.p2align 3
.L43:
	addq	$1, %rax
	cmpq	%r14, %rax
	je	.L42
.L41:
	cmpb	(%rax), %r8b
	jne	.L43
	cmpq	%r14, %rax
	jnb	.L42
	leaq	1(%rax), %rdi
	jmp	.L44
	.p2align 4,,10
	.p2align 3
.L42:
	xorb	$1, (%r10)
	addq	$8191, %r10
	xorq	%r13, %r11
	cmpq	%r10, %r12
	jne	.L38
	xorl	%eax, %eax
	movq	%r11, %rsi
	leaq	.LC3(%rip), %rdi
	vzeroupper
	call	printf@PLT
	xorl	%r8d, %r8d
	jmp	.L35
	.p2align 4,,10
	.p2align 3
.L46:
	addq	$1, %rdi
	jmp	.L44
	.cfi_endproc
.LFE17:
	.size	main, .-main
	.section	.rodata
	.align 8
	.type	utf8_name.1, @object
	.size	utf8_name.1, 9
utf8_name.1:
	.string	"caf\303\251Tag"
	.align 8
	.type	ascii_name.2, @object
	.size	ascii_name.2, 8
ascii_name.2:
	.string	"alpha-9"
	.local	expat_xml_data
	.comm	expat_xml_data,1048576,32
	.section	.rodata.cst32,"aM",@progbits,32
	.align 32
.LC0:
	.quad	7214953259748909884
	.quad	2467238573764605025
	.quad	7308338831271540529
	.quad	6879655750179037757
	.align 32
.LC1:
	.quad	4477196180081177204
	.quad	7881622743709934964
	.quad	4352010437821217648
	.quad	738214045636715311
	.section	.rodata.cst4,"aM",@progbits,4
	.align 4
.LC5:
	.long	538976288
	.ident	"GCC: (Debian 14.2.0-19) 14.2.0"
	.section	.note.GNU-stack,"",@progbits
