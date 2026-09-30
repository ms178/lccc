	.file	"spectral_norm.c"
	.text
	.section	.rodata.str1.1,"aMS",@progbits,1
.LC3:
	.string	"%.9f\n"
	.section	.text.startup,"ax",@progbits
	.p2align 4
	.globl	main
	.type	main, @function
main:
.LFB15:
	.cfi_startproc
	leaq	8(%rsp), %r10
	.cfi_def_cfa 10, 0
	andq	$-32, %rsp
	pushq	-8(%r10)
	pushq	%rbp
	movq	%rsp, %rbp
	.cfi_escape 0x10,0x6,0x2,0x76,0
	pushq	%r10
	.cfi_escape 0xf,0x3,0x76,0x78,0x6
	pushq	%rbx
	leaq	-48016(%rbp), %rdi
	leaq	-32016(%rbp), %rsi
	movq	%rdi, %rax
	subq	$48000, %rsp
	.cfi_escape 0x10,0x3,0x2,0x76,0x70
	vbroadcastsd	.LC2(%rip), %ymm0
	.p2align 5
	.p2align 4
	.p2align 3
.L2:
	vmovapd	%ymm0, (%rax)
	addq	$64, %rax
	vmovapd	%ymm0, -32(%rax)
	cmpq	%rax, %rsi
	jne	.L2
	vmovsd	.LC2(%rip), %xmm1
	vxorps	%xmm2, %xmm2, %xmm2
	movl	$10, %r8d
	leaq	-16016(%rbp), %rcx
	.p2align 4
	.p2align 3
.L3:
	movl	$2000, %r10d
	xorl	%ebx, %ebx
	.p2align 4
	.p2align 3
.L14:
	movl	%ebx, %edx
	movl	%ebx, %r11d
	vxorpd	%xmm3, %xmm3, %xmm3
	movq	%rdi, %r9
	.p2align 6
	.p2align 4
	.p2align 3
.L4:
	movl	%edx, %eax
	addl	$1, %edx
	addq	$8, %r9
	imull	%edx, %eax
	sarl	%eax
	leal	1(%r11,%rax), %eax
	vcvtsi2sdl	%eax, %xmm2, %xmm0
	vdivsd	%xmm0, %xmm1, %xmm0
	vmulsd	-8(%r9), %xmm0, %xmm0
	vaddsd	%xmm0, %xmm3, %xmm3
	cmpl	%r10d, %edx
	jne	.L4
	vmovsd	%xmm3, (%rcx,%rbx,8)
	addq	$1, %rbx
	leal	1(%rdx), %r10d
	cmpq	$2000, %rbx
	jne	.L14
	xorl	%r10d, %r10d
	.p2align 4
	.p2align 3
.L5:
	xorl	%eax, %eax
	vxorpd	%xmm3, %xmm3, %xmm3
	leal	1(%r10), %r11d
	.p2align 6
	.p2align 4
	.p2align 3
.L6:
	leal	(%r11,%rax), %edx
	leal	(%r10,%rax), %r9d
	imull	%r9d, %edx
	sarl	%edx
	leal	1(%rdx,%rax), %edx
	vcvtsi2sdl	%edx, %xmm2, %xmm0
	vdivsd	%xmm0, %xmm1, %xmm0
	vmulsd	(%rcx,%rax,8), %xmm0, %xmm0
	addq	$1, %rax
	vaddsd	%xmm0, %xmm3, %xmm3
	cmpq	$2000, %rax
	jne	.L6
	vmovsd	%xmm3, (%rsi,%r10,8)
	addq	$1, %r10
	cmpq	$2000, %r10
	jne	.L5
	movl	$2000, %r10d
	xorl	%ebx, %ebx
	.p2align 4
	.p2align 3
.L8:
	movl	%ebx, %edx
	movl	%ebx, %r11d
	vxorpd	%xmm3, %xmm3, %xmm3
	movq	%rsi, %r9
	.p2align 6
	.p2align 4
	.p2align 3
.L9:
	movl	%edx, %eax
	addl	$1, %edx
	addq	$8, %r9
	imull	%edx, %eax
	sarl	%eax
	leal	1(%r11,%rax), %eax
	vcvtsi2sdl	%eax, %xmm2, %xmm0
	vdivsd	%xmm0, %xmm1, %xmm0
	vmulsd	-8(%r9), %xmm0, %xmm0
	vaddsd	%xmm0, %xmm3, %xmm3
	cmpl	%r10d, %edx
	jne	.L9
	vmovsd	%xmm3, (%rcx,%rbx,8)
	addq	$1, %rbx
	leal	1(%rdx), %r10d
	cmpq	$2000, %rbx
	jne	.L8
	xorl	%r10d, %r10d
	.p2align 4
	.p2align 3
.L10:
	xorl	%eax, %eax
	vxorpd	%xmm3, %xmm3, %xmm3
	leal	1(%r10), %r11d
	.p2align 6
	.p2align 4
	.p2align 3
.L11:
	leal	(%r11,%rax), %edx
	leal	(%rax,%r10), %r9d
	imull	%r9d, %edx
	sarl	%edx
	leal	1(%rdx,%rax), %edx
	vcvtsi2sdl	%edx, %xmm2, %xmm0
	vdivsd	%xmm0, %xmm1, %xmm0
	vmulsd	(%rcx,%rax,8), %xmm0, %xmm0
	addq	$1, %rax
	vaddsd	%xmm0, %xmm3, %xmm3
	cmpq	$2000, %rax
	jne	.L11
	vmovsd	%xmm3, (%rdi,%r10,8)
	addq	$1, %r10
	cmpq	$2000, %r10
	jne	.L10
	subl	$1, %r8d
	jne	.L3
	vxorpd	%xmm2, %xmm2, %xmm2
	xorl	%eax, %eax
	vmovapd	%xmm2, %xmm1
	.p2align 4
	.p2align 3
.L13:
	vmovapd	(%rsi,%rax), %ymm0
	vmulpd	(%rdi,%rax), %ymm0, %ymm3
	addq	$32, %rax
	vmulpd	%ymm0, %ymm0, %ymm0
	vaddsd	%xmm3, %xmm1, %xmm1
	vunpckhpd	%xmm3, %xmm3, %xmm4
	vextractf128	$0x1, %ymm3, %xmm3
	vaddsd	%xmm0, %xmm2, %xmm2
	vaddsd	%xmm1, %xmm4, %xmm4
	vaddsd	%xmm4, %xmm3, %xmm1
	vunpckhpd	%xmm3, %xmm3, %xmm3
	vaddsd	%xmm3, %xmm1, %xmm1
	vunpckhpd	%xmm0, %xmm0, %xmm3
	vextractf128	$0x1, %ymm0, %xmm0
	vaddsd	%xmm2, %xmm3, %xmm3
	vaddsd	%xmm3, %xmm0, %xmm2
	vunpckhpd	%xmm0, %xmm0, %xmm0
	vaddsd	%xmm0, %xmm2, %xmm2
	cmpq	$16000, %rax
	jne	.L13
	vdivsd	%xmm2, %xmm1, %xmm0
	vxorpd	%xmm1, %xmm1, %xmm1
	vucomisd	%xmm0, %xmm1
	ja	.L29
	vsqrtsd	%xmm0, %xmm0, %xmm0
	vzeroupper
.L17:
	leaq	.LC3(%rip), %rdi
	movl	$1, %eax
	call	printf@PLT
	addq	$48000, %rsp
	xorl	%eax, %eax
	popq	%rbx
	popq	%r10
	.cfi_remember_state
	.cfi_def_cfa 10, 0
	popq	%rbp
	leaq	-8(%r10), %rsp
	.cfi_def_cfa 7, 8
	ret
.L29:
	.cfi_restore_state
	vzeroupper
	call	sqrt@PLT
	jmp	.L17
	.cfi_endproc
.LFE15:
	.size	main, .-main
	.section	.rodata.cst8,"aM",@progbits,8
	.align 8
.LC2:
	.long	0
	.long	1072693248
	.ident	"GCC: (Debian 14.2.0-19) 14.2.0"
	.section	.note.GNU-stack,"",@progbits
