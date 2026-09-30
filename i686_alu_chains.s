	.file	"i686_alu_chains.c"
	.text
	.p2align 4
	.globl	k_urem7
	.type	k_urem7, @function
k_urem7:
.LFB23:
	.cfi_startproc
	movl	%edi, %eax
	testl	%esi, %esi
	je	.L2
	xorl	%edi, %edi
	.p2align 6
	.p2align 4
	.p2align 3
.L3:
	movl	%eax, %ecx
	movl	%eax, %edx
	imulq	$613566757, %rcx, %rcx
	shrq	$32, %rcx
	subl	%ecx, %edx
	shrl	%edx
	addl	%ecx, %edx
	shrl	$2, %edx
	leal	0(,%rdx,8), %ecx
	subl	%edx, %ecx
	subl	%ecx, %eax
	leal	(%rax,%rdi), %eax
	addl	$1, %edi
	cmpl	%edi, %esi
	jne	.L3
.L2:
	ret
	.cfi_endproc
.LFE23:
	.size	k_urem7, .-k_urem7
	.p2align 4
	.globl	k_urem10
	.type	k_urem10, @function
k_urem10:
.LFB24:
	.cfi_startproc
	movl	%edi, %eax
	testl	%esi, %esi
	je	.L10
	xorl	%ecx, %ecx
	movl	$3435973837, %r8d
	.p2align 5
	.p2align 4
	.p2align 3
.L11:
	movl	%eax, %edx
	imulq	%r8, %rdx
	shrq	$35, %rdx
	leal	(%rdx,%rdx,4), %edi
	movl	%eax, %edx
	xorl	%ecx, %eax
	addl	$1, %ecx
	addl	%edi, %edi
	subl	%edi, %edx
	addl	%edx, %eax
	cmpl	%ecx, %esi
	jne	.L11
.L10:
	ret
	.cfi_endproc
.LFE24:
	.size	k_urem10, .-k_urem10
	.p2align 4
	.globl	k_udiv7
	.type	k_udiv7, @function
k_udiv7:
.LFB25:
	.cfi_startproc
	movl	%edi, %eax
	testl	%esi, %esi
	je	.L16
	leal	(%rsi,%rsi,2), %esi
	xorl	%ecx, %ecx
	.p2align 5
	.p2align 4
	.p2align 3
.L18:
	movl	%eax, %edx
	imulq	$613566757, %rdx, %rdx
	shrq	$32, %rdx
	subl	%edx, %eax
	shrl	%eax
	addl	%edx, %eax
	shrl	$2, %eax
	addl	%ecx, %eax
	addl	$3, %ecx
	cmpl	%ecx, %esi
	jne	.L18
.L16:
	ret
	.cfi_endproc
.LFE25:
	.size	k_udiv7, .-k_udiv7
	.p2align 4
	.globl	k_udr10
	.type	k_udr10, @function
k_udr10:
.LFB26:
	.cfi_startproc
	movl	%edi, %eax
	testl	%esi, %esi
	je	.L22
	xorl	%ecx, %ecx
	movl	$3435973837, %r8d
	.p2align 5
	.p2align 4
	.p2align 3
.L23:
	movl	%eax, %edx
	imulq	%r8, %rdx
	shrq	$35, %rdx
	leal	(%rdx,%rdx,4), %edi
	addl	%ecx, %edx
	addl	$1, %ecx
	addl	%edi, %edi
	subl	%edi, %eax
	leal	(%rax,%rax,2), %eax
	addl	%edx, %eax
	cmpl	%ecx, %esi
	jne	.L23
.L22:
	ret
	.cfi_endproc
.LFE26:
	.size	k_udr10, .-k_udr10
	.p2align 4
	.globl	k_sdr7
	.type	k_sdr7, @function
k_sdr7:
.LFB27:
	.cfi_startproc
	movl	%edi, %eax
	testl	%esi, %esi
	je	.L29
	xorl	%ecx, %ecx
	.p2align 6
	.p2align 4
	.p2align 3
.L30:
	movslq	%eax, %rdx
	movl	%eax, %edi
	imulq	$-1840700269, %rdx, %rdx
	sarl	$31, %edi
	shrq	$32, %rdx
	addl	%eax, %edx
	sarl	$2, %edx
	subl	%edi, %edx
	leal	0(,%rdx,8), %edi
	subl	%edx, %edi
	subl	%edi, %eax
	leal	(%rax,%rax,4), %eax
	addl	%edx, %eax
	subl	%ecx, %eax
	addl	$1, %ecx
	cmpl	%ecx, %esi
	jne	.L30
.L29:
	ret
	.cfi_endproc
.LFE27:
	.size	k_sdr7, .-k_sdr7
	.p2align 4
	.globl	k_srem7
	.type	k_srem7, @function
k_srem7:
.LFB28:
	.cfi_startproc
	movl	%edi, %eax
	testl	%esi, %esi
	je	.L36
	xorl	%ecx, %ecx
	.p2align 6
	.p2align 4
	.p2align 3
.L37:
	movslq	%eax, %rdx
	movl	%eax, %edi
	imulq	$-1840700269, %rdx, %rdx
	sarl	$31, %edi
	shrq	$32, %rdx
	addl	%eax, %edx
	sarl	$2, %edx
	subl	%edi, %edx
	leal	0(,%rdx,8), %edi
	subl	%edx, %edi
	movzwl	%cx, %edx
	addl	$1, %ecx
	subl	%edi, %eax
	leal	-30000(%rax,%rdx), %eax
	cmpl	%ecx, %esi
	jne	.L37
.L36:
	ret
	.cfi_endproc
.LFE28:
	.size	k_srem7, .-k_srem7
	.p2align 4
	.globl	k_srem16
	.type	k_srem16, @function
k_srem16:
.LFB29:
	.cfi_startproc
	movl	%edi, %eax
	testl	%esi, %esi
	je	.L43
	xorl	%ecx, %ecx
	.p2align 5
	.p2align 4
	.p2align 3
.L44:
	cltd
	shrl	$28, %edx
	addl	%edx, %eax
	andl	$15, %eax
	subl	%edx, %eax
	leal	(%rax,%rax,2), %edx
	movzbl	%cl, %eax
	addl	$1, %ecx
	leal	-100(%rdx,%rax), %eax
	cmpl	%ecx, %esi
	jne	.L44
.L43:
	ret
	.cfi_endproc
.LFE29:
	.size	k_srem16, .-k_srem16
	.p2align 4
	.globl	k_sdr16
	.type	k_sdr16, @function
k_sdr16:
.LFB30:
	.cfi_startproc
	movl	%edi, %eax
	testl	%esi, %esi
	je	.L50
	xorl	%edi, %edi
	.p2align 6
	.p2align 4
	.p2align 3
.L51:
	movl	%eax, %ecx
	sarl	$31, %ecx
	shrl	$28, %ecx
	leal	(%rax,%rcx), %edx
	andl	$15, %edx
	subl	%ecx, %edx
	leal	15(%rax), %ecx
	sall	$3, %edx
	testl	%eax, %eax
	cmovs	%ecx, %eax
	sarl	$4, %eax
	xorl	%edi, %eax
	addl	$1, %edi
	xorl	%edx, %eax
	cmpl	%edi, %esi
	jne	.L51
.L50:
	ret
	.cfi_endproc
.LFE30:
	.size	k_sdr16, .-k_sdr16
	.p2align 4
	.globl	k_mul7
	.type	k_mul7, @function
k_mul7:
.LFB31:
	.cfi_startproc
	movl	%edi, %eax
	testl	%esi, %esi
	je	.L57
	xorl	%edx, %edx
	.p2align 5
	.p2align 4
	.p2align 3
.L58:
	leal	0(,%rax,8), %ecx
	subl	%eax, %ecx
	movl	%ecx, %eax
	xorl	%edx, %eax
	addl	$1, %edx
	cmpl	%edx, %esi
	jne	.L58
.L57:
	ret
	.cfi_endproc
.LFE31:
	.size	k_mul7, .-k_mul7
	.p2align 4
	.globl	k_mul11
	.type	k_mul11, @function
k_mul11:
.LFB32:
	.cfi_startproc
	movl	%edi, %eax
	testl	%esi, %esi
	je	.L64
	xorl	%edx, %edx
	.p2align 4
	.p2align 4
	.p2align 3
.L65:
	leal	(%rax,%rax,4), %ecx
	leal	(%rax,%rcx,2), %eax
	xorl	%edx, %eax
	addl	$1, %edx
	cmpl	%edx, %esi
	jne	.L65
.L64:
	ret
	.cfi_endproc
.LFE32:
	.size	k_mul11, .-k_mul11
	.p2align 4
	.globl	k_mul17
	.type	k_mul17, @function
k_mul17:
.LFB33:
	.cfi_startproc
	movl	%edi, %eax
	testl	%esi, %esi
	je	.L71
	xorl	%edx, %edx
	.p2align 4
	.p2align 4
	.p2align 3
.L72:
	movl	%eax, %ecx
	sall	$4, %ecx
	addl	%ecx, %eax
	xorl	%edx, %eax
	addl	$1, %edx
	cmpl	%edx, %esi
	jne	.L72
.L71:
	ret
	.cfi_endproc
.LFE33:
	.size	k_mul17, .-k_mul17
	.p2align 4
	.globl	k_mul24
	.type	k_mul24, @function
k_mul24:
.LFB34:
	.cfi_startproc
	movl	%edi, %eax
	testl	%esi, %esi
	je	.L78
	xorl	%edx, %edx
	.p2align 4
	.p2align 4
	.p2align 3
.L79:
	leal	(%rax,%rax,2), %eax
	sall	$3, %eax
	xorl	%edx, %eax
	addl	$1, %edx
	cmpl	%edx, %esi
	jne	.L79
.L78:
	ret
	.cfi_endproc
.LFE34:
	.size	k_mul24, .-k_mul24
	.p2align 4
	.globl	k_mul45
	.type	k_mul45, @function
k_mul45:
.LFB35:
	.cfi_startproc
	movl	%edi, %eax
	testl	%esi, %esi
	je	.L85
	xorl	%edx, %edx
	.p2align 4
	.p2align 4
	.p2align 3
.L86:
	imull	$45, %eax, %eax
	xorl	%edx, %eax
	addl	$1, %edx
	cmpl	%edx, %esi
	jne	.L86
.L85:
	ret
	.cfi_endproc
.LFE35:
	.size	k_mul45, .-k_mul45
	.p2align 4
	.globl	k_mulm3
	.type	k_mulm3, @function
k_mulm3:
.LFB36:
	.cfi_startproc
	movl	%edi, %eax
	testl	%esi, %esi
	je	.L92
	xorl	%edx, %edx
	.p2align 5
	.p2align 4
	.p2align 3
.L93:
	leal	0(,%rax,4), %ecx
	subl	%ecx, %eax
	xorl	%edx, %eax
	addl	$1, %edx
	cmpl	%edx, %esi
	jne	.L93
.L92:
	ret
	.cfi_endproc
.LFE36:
	.size	k_mulm3, .-k_mulm3
	.p2align 4
	.globl	k_mul1000
	.type	k_mul1000, @function
k_mul1000:
.LFB37:
	.cfi_startproc
	movl	%edi, %eax
	testl	%esi, %esi
	je	.L99
	xorl	%edx, %edx
	.p2align 4
	.p2align 4
	.p2align 3
.L100:
	imull	$1000, %eax, %eax
	xorl	%edx, %eax
	addl	$1, %edx
	cmpl	%edx, %esi
	jne	.L100
.L99:
	ret
	.cfi_endproc
.LFE37:
	.size	k_mul1000, .-k_mul1000
	.p2align 4
	.globl	k_fnv
	.type	k_fnv, @function
k_fnv:
.LFB38:
	.cfi_startproc
	movl	%edi, %eax
	testl	%esi, %esi
	je	.L106
	xorl	%edx, %edx
	.p2align 5
	.p2align 4
	.p2align 3
.L107:
	movzbl	%dl, %ecx
	addl	$1, %edx
	xorl	%ecx, %eax
	imull	$16777619, %eax, %eax
	cmpl	%edx, %esi
	jne	.L107
.L106:
	ret
	.cfi_endproc
.LFE38:
	.size	k_fnv, .-k_fnv
	.p2align 4
	.globl	k_digits
	.type	k_digits, @function
k_digits:
.LFB39:
	.cfi_startproc
	testl	%esi, %esi
	je	.L117
	leal	(%rsi,%rdi), %r10d
	movl	$3435973837, %r9d
	xorl	%esi, %esi
	.p2align 4
	.p2align 3
.L116:
	testl	%edi, %edi
	je	.L114
	movl	%edi, %edx
	.p2align 6
	.p2align 4
	.p2align 3
.L115:
	movl	%edx, %eax
	movl	%edx, %r8d
	imulq	%r9, %rax
	shrq	$35, %rax
	leal	(%rax,%rax,4), %ecx
	addl	%ecx, %ecx
	subl	%ecx, %r8d
	movl	%edx, %ecx
	movl	%eax, %edx
	addl	%r8d, %esi
	cmpl	$9, %ecx
	ja	.L115
.L114:
	addl	$1, %edi
	cmpl	%edi, %r10d
	jne	.L116
	movl	%esi, %eax
	ret
.L117:
	xorl	%esi, %esi
	movl	%esi, %eax
	ret
	.cfi_endproc
.LFE39:
	.size	k_digits, .-k_digits
	.section	.rodata.str1.1,"aMS",@progbits,1
.LC0:
	.string	"urem7"
.LC1:
	.string	"urem10"
.LC2:
	.string	"udiv7"
.LC3:
	.string	"udr10"
.LC4:
	.string	"sdr7"
.LC5:
	.string	"srem7"
.LC6:
	.string	"srem16"
.LC7:
	.string	"sdr16"
.LC8:
	.string	"mul7"
.LC9:
	.string	"mul11"
.LC10:
	.string	"mul17"
.LC11:
	.string	"mul24"
.LC12:
	.string	"mul45"
.LC13:
	.string	"mulm3"
.LC14:
	.string	"mul1000"
.LC15:
	.string	"fnv"
.LC16:
	.string	"digits"
.LC18:
	.string	"%-8s %11.3f\n"
.LC19:
	.string	"%-8s %08x\n"
.LC20:
	.string	"TOTAL %08x\n"
	.section	.text.startup,"ax",@progbits
	.p2align 4
	.globl	main
	.type	main, @function
main:
.LFB40:
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
	movl	$20000000, %r13d
	pushq	%r12
	.cfi_def_cfa_offset 40
	.cfi_offset 12, -40
	pushq	%rbp
	.cfi_def_cfa_offset 48
	.cfi_offset 6, -48
	pushq	%rbx
	.cfi_def_cfa_offset 56
	.cfi_offset 3, -56
	subq	$472, %rsp
	.cfi_def_cfa_offset 528
	cmpl	$1, %edi
	jle	.L124
	movq	8(%rsi), %rdi
	xorl	%edx, %edx
	xorl	%esi, %esi
	call	strtoul@PLT
	movl	%eax, %r13d
.L124:
	leaq	.LC0(%rip), %rax
	leaq	48(%rsp), %r15
	xorl	%r12d, %r12d
	movl	$123456789, 64(%rsp)
	movq	%rax, 48(%rsp)
	leaq	k_urem7(%rip), %rax
	movq	%rax, 56(%rsp)
	leaq	.LC1(%rip), %rax
	movq	%rax, 72(%rsp)
	leaq	k_urem10(%rip), %rax
	movq	%rax, 80(%rsp)
	leaq	.LC2(%rip), %rax
	movq	%rax, 96(%rsp)
	leaq	k_udiv7(%rip), %rax
	movq	%rax, 104(%rsp)
	leaq	.LC3(%rip), %rax
	movq	%rax, 120(%rsp)
	leaq	k_udr10(%rip), %rax
	movq	%rax, 128(%rsp)
	leaq	.LC4(%rip), %rax
	movq	%rax, 144(%rsp)
	leaq	k_sdr7(%rip), %rax
	movq	%rax, 152(%rsp)
	leaq	.LC5(%rip), %rax
	movq	%rax, 168(%rsp)
	leaq	k_srem7(%rip), %rax
	movq	%rax, 176(%rsp)
	leaq	.LC6(%rip), %rax
	movq	%rax, 192(%rsp)
	leaq	k_srem16(%rip), %rax
	movq	%rax, 200(%rsp)
	leaq	.LC7(%rip), %rax
	movq	%rax, 216(%rsp)
	leaq	k_sdr16(%rip), %rax
	movq	%rax, 224(%rsp)
	leaq	.LC8(%rip), %rax
	movq	%rax, 240(%rsp)
	leaq	k_mul7(%rip), %rax
	movq	%rax, 248(%rsp)
	leaq	.LC9(%rip), %rax
	movq	%rax, 264(%rsp)
	leaq	k_mul11(%rip), %rax
	movq	%rax, 272(%rsp)
	leaq	.LC10(%rip), %rax
	movq	%rax, 288(%rsp)
	leaq	k_mul17(%rip), %rax
	movq	%rax, 296(%rsp)
	leaq	.LC11(%rip), %rax
	movl	$987654321, 88(%rsp)
	movl	$-559038737, 112(%rsp)
	movl	$305419896, 136(%rsp)
	movl	$2147422772, 160(%rsp)
	movl	$-2147478988, 184(%rsp)
	movl	$-16, 208(%rsp)
	movl	$267242409, 232(%rsp)
	movl	$1, 256(%rsp)
	movl	$3, 280(%rsp)
	movl	$5, 304(%rsp)
	movq	%rax, 312(%rsp)
	leaq	k_mul24(%rip), %rax
	movq	%rax, 320(%rsp)
	leaq	.LC12(%rip), %rax
	movq	%rax, 336(%rsp)
	leaq	k_mul45(%rip), %rax
	movq	%rax, 344(%rsp)
	leaq	.LC13(%rip), %rax
	movq	%rax, 360(%rsp)
	leaq	k_mulm3(%rip), %rax
	movq	%rax, 368(%rsp)
	leaq	.LC14(%rip), %rax
	movq	%rax, 384(%rsp)
	leaq	k_mul1000(%rip), %rax
	movq	%rax, 392(%rsp)
	leaq	.LC15(%rip), %rax
	movq	%rax, 408(%rsp)
	leaq	k_fnv(%rip), %rax
	movq	%rax, 416(%rsp)
	leaq	.LC16(%rip), %rax
	movq	%rax, 432(%rsp)
	leaq	k_digits(%rip), %rax
	movq	%rax, 440(%rsp)
	leaq	456(%rsp), %rax
	movq	%rax, 16(%rsp)
	leaq	32(%rsp), %rax
	movl	$7, 328(%rsp)
	movl	$9, 352(%rsp)
	movl	$11, 376(%rsp)
	movl	$13, 400(%rsp)
	movl	$-2128831035, 424(%rsp)
	movl	$1234567, 448(%rsp)
	movq	%rax, 24(%rsp)
	.p2align 4
	.p2align 3
.L128:
	movq	8(%r15), %rbp
	movq	24(%rsp), %r14
	movl	%r13d, %ebx
	movl	$1, %edi
	leaq	k_digits(%rip), %rax
	shrl	$4, %ebx
	movq	%r14, %rsi
	cmpq	%rax, %rbp
	cmovne	%r13, %rbx
	addq	$24, %r15
	call	clock_gettime@PLT
	vxorpd	%xmm2, %xmm2, %xmm2
	movl	-8(%r15), %edi
	vcvtsi2sdq	40(%rsp), %xmm2, %xmm0
	movl	%ebx, %esi
	vcvtsi2sdq	32(%rsp), %xmm2, %xmm1
	vfmadd132sd	.LC17(%rip), %xmm0, %xmm1
	vmovsd	%xmm1, 8(%rsp)
	call	*%rbp
	movq	%r14, %rsi
	movl	$1, %edi
	movl	%eax, %ebp
	call	clock_gettime@PLT
	vxorpd	%xmm2, %xmm2, %xmm2
	movl	%r12d, %eax
	movq	-24(%r15), %r14
	vcvtsi2sdq	40(%rsp), %xmm2, %xmm1
	sall	$5, %eax
	movq	stderr(%rip), %rdi
	leaq	.LC18(%rip), %rsi
	vcvtsi2sdq	32(%rsp), %xmm2, %xmm0
	subl	%r12d, %eax
	movq	%r14, %rdx
	vfmadd132sd	.LC17(%rip), %xmm1, %xmm0
	vcvtsi2sdq	%rbx, %xmm2, %xmm1
	leal	(%rax,%rbp), %r12d
	movl	$1, %eax
	vsubsd	8(%rsp), %xmm0, %xmm0
	vdivsd	%xmm1, %xmm0, %xmm0
	call	fprintf@PLT
	movl	%ebp, %edx
	movq	%r14, %rsi
	xorl	%eax, %eax
	leaq	.LC19(%rip), %rdi
	call	printf@PLT
	movq	16(%rsp), %rax
	cmpq	%rax, %r15
	jne	.L128
	movl	%r12d, %esi
	leaq	.LC20(%rip), %rdi
	xorl	%eax, %eax
	call	printf@PLT
	addq	$472, %rsp
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
	.cfi_endproc
.LFE40:
	.size	main, .-main
	.section	.rodata.cst8,"aM",@progbits,8
	.align 8
.LC17:
	.long	0
	.long	1104006501
	.ident	"GCC: (Debian 14.2.0-19) 14.2.0"
	.section	.note.GNU-stack,"",@progbits
