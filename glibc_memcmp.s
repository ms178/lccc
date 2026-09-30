	.file	"glibc_memcmp.c"
	.text
	.p2align 4
	.type	glibc_memcmp_common_alignment, @function
glibc_memcmp_common_alignment:
.LFB12:
	.cfi_startproc
	shrq	$2, %rdx
	pushq	%rbp
	.cfi_def_cfa_offset 16
	.cfi_offset 6, -16
	movq	%rdi, %rax
	movq	%rdx, %rbp
	pushq	%rbx
	.cfi_def_cfa_offset 24
	.cfi_offset 3, -24
	salq	$5, %rbp
	addq	%rdi, %rbp
	.p2align 4
	.p2align 3
.L15:
	movq	(%rax), %rcx
	movq	(%rsi), %rdx
	movq	8(%rax), %rdi
	movq	16(%rax), %r8
	movq	24(%rax), %r11
	movq	8(%rsi), %rbx
	movq	16(%rsi), %r9
	movq	24(%rsi), %r10
	cmpq	%rdx, %rcx
	jne	.L25
	cmpq	%rbx, %rdi
	jne	.L26
	cmpq	%r9, %r8
	jne	.L27
	cmpq	%r10, %r11
	jne	.L28
	addq	$32, %rax
	addq	$32, %rsi
	cmpq	%rbp, %rax
	jne	.L15
	xorl	%eax, %eax
.L1:
	popq	%rbx
	.cfi_remember_state
	.cfi_def_cfa_offset 16
	popq	%rbp
	.cfi_def_cfa_offset 8
	ret
	.p2align 4,,10
	.p2align 3
.L25:
	.cfi_restore_state
	movl	%ecx, %edi
	movl	%edx, %esi
	cmpb	%dl, %cl
	jne	.L3
	movzbl	%ch, %edi
	movzbl	%dh, %esi
	cmpb	%sil, %dil
	jne	.L3
	movq	%rcx, %r8
	movq	%rdx, %rax
	shrq	$16, %r8
	shrq	$16, %rax
	movl	%r8d, %edi
	movl	%eax, %esi
	cmpb	%al, %r8b
	jne	.L3
	movq	%rcx, %r8
	movq	%rdx, %rax
	shrq	$24, %r8
	shrq	$24, %rax
	movl	%r8d, %edi
	movl	%eax, %esi
	cmpb	%al, %r8b
	jne	.L3
	movq	%rcx, %r8
	movq	%rdx, %rax
	shrq	$32, %r8
	shrq	$32, %rax
	movl	%r8d, %edi
	movl	%eax, %esi
	cmpb	%al, %r8b
	jne	.L3
	movq	%rcx, %r8
	movq	%rdx, %rax
	shrq	$40, %r8
	shrq	$40, %rax
	movl	%r8d, %edi
	movl	%eax, %esi
	cmpb	%al, %r8b
	jne	.L3
	movq	%rcx, %r8
	movq	%rdx, %rax
	shrq	$48, %r8
	shrq	$48, %rax
	movl	%r8d, %edi
	movl	%eax, %esi
	cmpb	%al, %r8b
	jne	.L3
	shrq	$56, %rcx
	shrq	$56, %rdx
	xorl	%eax, %eax
	movl	%ecx, %edi
	movl	%edx, %esi
	cmpb	%dl, %cl
	je	.L1
.L3:
	movzbl	%dil, %edi
	movzbl	%sil, %esi
	movl	%edi, %eax
	subl	%esi, %eax
	jmp	.L1
	.p2align 4,,10
	.p2align 3
.L26:
	movl	%edi, %ecx
	movl	%ebx, %edx
	cmpb	%bl, %dil
	jne	.L13
	movq	%rdi, %rax
	movzbl	%bh, %edx
	movzbl	%ah, %ecx
	cmpb	%dl, %cl
	jne	.L13
	movq	%rdi, %rsi
	movq	%rbx, %rax
	shrq	$16, %rsi
	shrq	$16, %rax
	movl	%esi, %ecx
	movl	%eax, %edx
	cmpb	%al, %sil
	jne	.L13
	movq	%rdi, %rsi
	movq	%rbx, %rax
	shrq	$24, %rsi
	shrq	$24, %rax
	movl	%esi, %ecx
	movl	%eax, %edx
	cmpb	%al, %sil
	jne	.L13
	movq	%rdi, %rsi
	movq	%rbx, %rax
	shrq	$32, %rsi
	shrq	$32, %rax
	movl	%esi, %ecx
	movl	%eax, %edx
	cmpb	%al, %sil
	jne	.L13
	movq	%rdi, %rsi
	movq	%rbx, %rax
	shrq	$40, %rsi
	shrq	$40, %rax
	movl	%esi, %ecx
	movl	%eax, %edx
	cmpb	%al, %sil
	jne	.L13
	movq	%rdi, %rsi
	movq	%rbx, %rax
	shrq	$48, %rsi
	shrq	$48, %rax
	movl	%esi, %ecx
	movl	%eax, %edx
	cmpb	%al, %sil
	jne	.L13
	shrq	$56, %rdi
	shrq	$56, %rbx
	xorl	%eax, %eax
	movl	%edi, %ecx
	movl	%ebx, %edx
	cmpb	%bl, %dil
	je	.L1
.L13:
	movzbl	%cl, %ecx
	movzbl	%dl, %edx
	popq	%rbx
	.cfi_remember_state
	.cfi_def_cfa_offset 16
	popq	%rbp
	.cfi_def_cfa_offset 8
	movl	%ecx, %eax
	subl	%edx, %eax
	ret
	.p2align 4,,10
	.p2align 3
.L27:
	.cfi_restore_state
	movl	%r8d, %ecx
	movl	%r9d, %edx
	cmpb	%r8b, %r9b
	jne	.L13
	movq	%r8, %rax
	movzbl	%ah, %ecx
	movq	%r9, %rax
	movzbl	%ah, %edx
	cmpb	%cl, %dl
	jne	.L13
	movq	%r8, %rax
	movq	%r9, %rsi
	shrq	$16, %rax
	shrq	$16, %rsi
	movl	%eax, %ecx
	movl	%esi, %edx
	cmpb	%al, %sil
	jne	.L13
	movq	%r8, %rax
	movq	%r9, %rsi
	shrq	$24, %rax
	shrq	$24, %rsi
	movl	%eax, %ecx
	movl	%esi, %edx
	cmpb	%al, %sil
	jne	.L13
	movq	%r8, %rax
	movq	%r9, %rsi
	shrq	$32, %rax
	shrq	$32, %rsi
	movl	%eax, %ecx
	movl	%esi, %edx
	cmpb	%al, %sil
	jne	.L13
	movq	%r8, %rsi
	movq	%r9, %rax
	shrq	$40, %rsi
	shrq	$40, %rax
	movl	%esi, %ecx
	movl	%eax, %edx
	cmpb	%al, %sil
	jne	.L13
	movq	%r8, %rax
	movq	%r9, %rsi
	shrq	$48, %rax
	shrq	$48, %rsi
	movl	%eax, %ecx
	movl	%esi, %edx
	cmpb	%al, %sil
	jne	.L13
	shrq	$56, %r8
	shrq	$56, %r9
	xorl	%eax, %eax
	movl	%r8d, %ecx
	movl	%r9d, %edx
	cmpb	%r8b, %r9b
	jne	.L13
	popq	%rbx
	.cfi_remember_state
	.cfi_def_cfa_offset 16
	popq	%rbp
	.cfi_def_cfa_offset 8
	ret
	.p2align 4,,10
	.p2align 3
.L28:
	.cfi_restore_state
	movl	%r11d, %ecx
	movl	%r10d, %edx
	cmpb	%r10b, %r11b
	jne	.L13
	movq	%r11, %rax
	movzbl	%ah, %ecx
	movq	%r10, %rax
	movzbl	%ah, %edx
	cmpb	%cl, %dl
	jne	.L13
	movq	%r11, %rax
	movq	%r10, %rsi
	shrq	$16, %rax
	shrq	$16, %rsi
	movl	%eax, %ecx
	movl	%esi, %edx
	cmpb	%al, %sil
	jne	.L13
	movq	%r11, %rax
	movq	%r10, %rsi
	shrq	$24, %rax
	shrq	$24, %rsi
	movl	%eax, %ecx
	movl	%esi, %edx
	cmpb	%al, %sil
	jne	.L13
	movq	%r11, %rax
	movq	%r10, %rsi
	shrq	$32, %rax
	shrq	$32, %rsi
	movl	%eax, %ecx
	movl	%esi, %edx
	cmpb	%al, %sil
	jne	.L13
	movq	%r11, %rax
	movq	%r10, %rsi
	shrq	$40, %rax
	shrq	$40, %rsi
	movl	%eax, %ecx
	movl	%esi, %edx
	cmpb	%al, %sil
	jne	.L13
	movq	%r11, %rax
	movq	%r10, %rsi
	shrq	$48, %rax
	shrq	$48, %rsi
	movl	%eax, %ecx
	movl	%esi, %edx
	cmpb	%al, %sil
	jne	.L13
	shrq	$56, %r11
	shrq	$56, %r10
	xorl	%eax, %eax
	movl	%r11d, %ecx
	movl	%r10d, %edx
	cmpb	%r10b, %r11b
	jne	.L13
	popq	%rbx
	.cfi_def_cfa_offset 16
	popq	%rbp
	.cfi_def_cfa_offset 8
	ret
	.cfi_endproc
.LFE12:
	.size	glibc_memcmp_common_alignment, .-glibc_memcmp_common_alignment
	.section	.rodata.str1.1,"aMS",@progbits,1
.LC1:
	.string	"%lu\n"
	.section	.text.startup,"ax",@progbits
	.p2align 4
	.globl	main
	.type	main, @function
main:
.LFB15:
	.cfi_startproc
	pushq	%rbp
	.cfi_def_cfa_offset 16
	.cfi_offset 6, -16
	movl	$4, %edx
	movq	%rsp, %rbp
	.cfi_def_cfa_register 6
	pushq	%r15
	pushq	%r14
	pushq	%r13
	pushq	%r12
	pushq	%rbx
	andq	$-32, %rsp
	addq	$-128, %rsp
	.cfi_offset 15, -24
	.cfi_offset 14, -32
	.cfi_offset 13, -40
	.cfi_offset 12, -48
	.cfi_offset 3, -56
	vmovdqa	.LC0(%rip), %ymm0
	leaq	96(%rsp), %r13
	leaq	64(%rsp), %r12
	movq	%r13, %rsi
	movq	%r12, %rdi
	vmovdqa	%ymm0, 64(%rsp)
	vmovdqa	%ymm0, 96(%rsp)
	call	glibc_memcmp_common_alignment
	testl	%eax, %eax
	jne	.L32
	movl	%eax, %ebx
	movl	$4, %edx
	movq	%r13, %rsi
	movq	%r12, %rdi
	movabsq	$72623859790382857, %rax
	movq	%rax, 104(%rsp)
	call	glibc_memcmp_common_alignment
	testl	%eax, %eax
	jns	.L32
	leaq	glibc_left(%rip), %rsi
	xorl	%ecx, %ecx
	leaq	glibc_right(%rip), %r15
	movabsq	$-7046029254386353131, %rax
	movq	%rsi, 40(%rsp)
	movq	%rax, %rdx
	.p2align 6
	.p2align 4
	.p2align 3
.L33:
	movq	%rdx, %rax
	salq	$7, %rax
	xorq	%rdx, %rax
	movq	%rax, %rdx
	shrq	$9, %rdx
	xorq	%rdx, %rax
	movq	%rax, %rdx
	salq	$8, %rdx
	xorq	%rax, %rdx
	movq	%rdx, (%rsi,%rcx,8)
	movq	%rdx, (%r15,%rcx,8)
	addq	$1, %rcx
	cmpq	$8192, %rcx
	jne	.L33
	movl	%ebx, 56(%rsp)
	xorl	%r13d, %r13d
	leaq	65536+glibc_left(%rip), %r12
	movq	$1, 48(%rsp)
	movl	$0, 60(%rsp)
	movq	$0, 24(%rsp)
	movq	%r15, 32(%rsp)
	.p2align 4
	.p2align 3
.L44:
	movq	40(%rsp), %rax
	movq	%r13, %r14
	movzbl	60(%rsp), %edi
	andl	$8191, %r14d
	movq	32(%rsp), %rsi
	movq	(%rax,%r14,8), %r15
	movl	$1, %eax
	shlx	%rdi, %rax, %rax
	xorq	%r15, %rax
	movq	%rax, (%rsi,%r14,8)
	leaq	glibc_right(%rip), %rax
	movq	%rax, 32(%rsp)
	movq	%rax, %rdx
	leaq	glibc_left(%rip), %rax
	movq	%rax, 40(%rsp)
	.p2align 4
	.p2align 3
.L43:
	movq	(%rax), %rbx
	movq	(%rdx), %rcx
	movq	8(%rax), %rdi
	movq	16(%rax), %r9
	movq	24(%rax), %r11
	movq	8(%rdx), %rsi
	movq	16(%rdx), %r8
	movq	24(%rdx), %r10
	cmpq	%rcx, %rbx
	jne	.L53
	cmpq	%rsi, %rdi
	jne	.L54
	cmpq	%r8, %r9
	jne	.L55
	cmpq	%r10, %r11
	jne	.L56
	addq	$32, %rax
	addq	$32, %rdx
	cmpq	%r12, %rax
	jne	.L43
	movl	$257, %eax
.L36:
	movq	48(%rsp), %rbx
	addq	$4051, %r13
	addl	$11, 60(%rsp)
	imulq	%rbx, %rax
	addq	$1, %rbx
	addq	%rax, 24(%rsp)
	leaq	glibc_right(%rip), %rax
	movq	%rbx, 48(%rsp)
	movq	%r15, (%rax,%r14,8)
	cmpq	$16592896, %r13
	jne	.L44
	movq	24(%rsp), %rsi
	leaq	.LC1(%rip), %rdi
	xorl	%eax, %eax
	movl	56(%rsp), %ebx
	vzeroupper
	call	printf@PLT
	jmp	.L29
.L32:
	movl	$2, %ebx
	vzeroupper
.L29:
	leaq	-40(%rbp), %rsp
	movl	%ebx, %eax
	popq	%rbx
	popq	%r12
	popq	%r13
	popq	%r14
	popq	%r15
	popq	%rbp
	.cfi_remember_state
	.cfi_def_cfa 7, 8
	ret
	.p2align 4,,10
	.p2align 3
.L53:
	.cfi_restore_state
	movl	%ebx, %edx
	movl	%ecx, %esi
	cmpb	%cl, %bl
	jne	.L35
	movzbl	%bh, %edx
	movzbl	%ch, %esi
	cmpb	%sil, %dl
	jne	.L35
	movq	%rbx, %rdi
	movq	%rcx, %rax
	shrq	$16, %rdi
	shrq	$16, %rax
	movl	%edi, %edx
	movl	%eax, %esi
	cmpb	%al, %dil
	jne	.L35
	movq	%rbx, %rdi
	movq	%rcx, %rax
	shrq	$24, %rdi
	shrq	$24, %rax
	movl	%edi, %edx
	movl	%eax, %esi
	cmpb	%al, %dil
	jne	.L35
	movq	%rbx, %rdi
	movq	%rcx, %rax
	shrq	$32, %rdi
	shrq	$32, %rax
	movl	%edi, %edx
	movl	%eax, %esi
	cmpb	%al, %dil
	jne	.L35
	movq	%rbx, %rdi
	movq	%rcx, %rax
	shrq	$40, %rdi
	shrq	$40, %rax
	movl	%edi, %edx
	movl	%eax, %esi
	cmpb	%al, %dil
	jne	.L35
	movq	%rbx, %rdi
	movq	%rcx, %rax
	shrq	$48, %rdi
	shrq	$48, %rax
	movl	%edi, %edx
	movl	%eax, %esi
	cmpb	%al, %dil
	jne	.L35
	shrq	$56, %rbx
	shrq	$56, %rcx
	movl	$257, %eax
	movl	%ebx, %edx
	movl	%ecx, %esi
	cmpb	%cl, %bl
	je	.L36
.L35:
	movzbl	%dl, %edx
	movzbl	%sil, %esi
	subl	%esi, %edx
	leal	257(%rdx), %eax
	cltq
	jmp	.L36
	.p2align 4,,10
	.p2align 3
.L54:
	movl	%edi, %edx
	movl	%esi, %ecx
	cmpb	%sil, %dil
	jne	.L42
	movq	%rdi, %rax
	movzbl	%ah, %edx
	movq	%rsi, %rax
	movzbl	%ah, %ecx
	cmpb	%cl, %dl
	jne	.L42
	movq	%rdi, %r8
	shrq	$16, %rax
	shrq	$16, %r8
	movl	%eax, %ecx
	movl	%r8d, %edx
	cmpb	%al, %r8b
	jne	.L42
	movq	%rdi, %r8
	movq	%rsi, %rax
	shrq	$24, %r8
	shrq	$24, %rax
	movl	%r8d, %edx
	movl	%eax, %ecx
	cmpb	%al, %r8b
	jne	.L42
	movq	%rdi, %r8
	movq	%rsi, %rax
	shrq	$32, %r8
	shrq	$32, %rax
	movl	%r8d, %edx
	movl	%eax, %ecx
	cmpb	%al, %r8b
	jne	.L42
	movq	%rdi, %r8
	movq	%rsi, %rax
	shrq	$40, %r8
	shrq	$40, %rax
	movl	%r8d, %edx
	movl	%eax, %ecx
	cmpb	%al, %r8b
	jne	.L42
	movq	%rdi, %r8
	movq	%rsi, %rax
	shrq	$48, %r8
	shrq	$48, %rax
	movl	%r8d, %edx
	movl	%eax, %ecx
	cmpb	%al, %r8b
	jne	.L42
	shrq	$56, %rdi
	shrq	$56, %rsi
	movl	$257, %eax
	movl	%edi, %edx
	movl	%esi, %ecx
	cmpb	%sil, %dil
	je	.L36
	.p2align 4
	.p2align 3
.L42:
	movzbl	%dl, %edx
	movzbl	%cl, %ecx
	subl	%ecx, %edx
	leal	257(%rdx), %eax
	cltq
	jmp	.L36
	.p2align 4,,10
	.p2align 3
.L55:
	movl	%r9d, %edx
	movl	%r8d, %ecx
	cmpb	%r8b, %r9b
	jne	.L42
	movq	%r9, %rax
	movzbl	%ah, %edx
	movq	%r8, %rax
	movzbl	%ah, %ecx
	cmpb	%cl, %dl
	jne	.L42
	movq	%r9, %rsi
	shrq	$16, %rax
	shrq	$16, %rsi
	movl	%eax, %ecx
	movl	%esi, %edx
	cmpb	%al, %sil
	jne	.L42
	movq	%r9, %rsi
	movq	%r8, %rax
	shrq	$24, %rsi
	shrq	$24, %rax
	movl	%esi, %edx
	movl	%eax, %ecx
	cmpb	%al, %sil
	jne	.L42
	movq	%r9, %rsi
	movq	%r8, %rax
	shrq	$32, %rsi
	shrq	$32, %rax
	movl	%esi, %edx
	movl	%eax, %ecx
	cmpb	%al, %sil
	jne	.L42
	movq	%r9, %rsi
	movq	%r8, %rax
	shrq	$40, %rsi
	shrq	$40, %rax
	movl	%esi, %edx
	movl	%eax, %ecx
	cmpb	%al, %sil
	jne	.L42
	movq	%r9, %rsi
	movq	%r8, %rax
	shrq	$48, %rsi
	shrq	$48, %rax
	movl	%esi, %edx
	movl	%eax, %ecx
	cmpb	%al, %sil
	jne	.L42
	shrq	$56, %r9
	shrq	$56, %r8
	movl	$257, %eax
	movl	%r9d, %edx
	movl	%r8d, %ecx
	cmpb	%r8b, %r9b
	je	.L36
	jmp	.L42
	.p2align 4,,10
	.p2align 3
.L56:
	movl	%r11d, %edx
	movl	%r10d, %ecx
	cmpb	%r10b, %r11b
	jne	.L42
	movq	%r11, %rax
	movzbl	%ah, %edx
	movq	%r10, %rax
	movzbl	%ah, %ecx
	cmpb	%cl, %dl
	jne	.L42
	movq	%r11, %rsi
	shrq	$16, %rax
	shrq	$16, %rsi
	movl	%eax, %ecx
	movl	%esi, %edx
	cmpb	%al, %sil
	jne	.L42
	movq	%r11, %rsi
	movq	%r10, %rax
	shrq	$24, %rsi
	shrq	$24, %rax
	movl	%esi, %edx
	movl	%eax, %ecx
	cmpb	%al, %sil
	jne	.L42
	movq	%r11, %rsi
	movq	%r10, %rax
	shrq	$32, %rsi
	shrq	$32, %rax
	movl	%esi, %edx
	movl	%eax, %ecx
	cmpb	%al, %sil
	jne	.L42
	movq	%r11, %rsi
	movq	%r10, %rax
	shrq	$40, %rsi
	shrq	$40, %rax
	movl	%esi, %edx
	movl	%eax, %ecx
	cmpb	%al, %sil
	jne	.L42
	movq	%r11, %rsi
	movq	%r10, %rax
	shrq	$48, %rsi
	shrq	$48, %rax
	movl	%esi, %edx
	movl	%eax, %ecx
	cmpb	%al, %sil
	jne	.L42
	shrq	$56, %r11
	shrq	$56, %r10
	movl	$257, %eax
	movl	%r11d, %edx
	movl	%r10d, %ecx
	cmpb	%r10b, %r11b
	je	.L36
	jmp	.L42
	.cfi_endproc
.LFE15:
	.size	main, .-main
	.local	glibc_right
	.comm	glibc_right,65536,32
	.local	glibc_left
	.comm	glibc_left,65536,32
	.section	.rodata.cst32,"aM",@progbits,32
	.align 32
.LC0:
	.quad	0
	.quad	72623859790382856
	.quad	-1
	.quad	7
	.ident	"GCC: (Debian 14.2.0-19) 14.2.0"
	.section	.note.GNU-stack,"",@progbits
