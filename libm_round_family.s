	.file	"libm_round_family.c"
	.text
	.p2align 4
	.type	round_family_pass, @function
round_family_pass:
.LFB20:
	.cfi_startproc
	leaq	buf(%rip), %rax
	vmovq	.LC1(%rip), %xmm6
	vmovsd	.LC2(%rip), %xmm7
	vxorpd	%xmm5, %xmm5, %xmm5
	leaq	32768(%rax), %rdx
	.p2align 4
	.p2align 3
.L2:
	vmovsd	(%rax), %xmm1
	addq	$8, %rax
	vroundsd	$9, %xmm1, %xmm1, %xmm3
	vroundsd	$10, %xmm1, %xmm1, %xmm10
	vfmadd132sd	%xmm7, %xmm10, %xmm3
	vroundsd	$4, %xmm1, %xmm1, %xmm0
	vroundsd	$12, %xmm1, %xmm1, %xmm9
	vroundsd	$8, %xmm1, %xmm1, %xmm8
	vroundsd	$11, %xmm1, %xmm1, %xmm4
	vandpd	%xmm1, %xmm6, %xmm1
	vandnpd	%xmm4, %xmm6, %xmm2
	vorpd	%xmm1, %xmm2, %xmm1
	vaddsd	%xmm3, %xmm0, %xmm0
	vaddsd	%xmm9, %xmm0, %xmm0
	vaddsd	%xmm8, %xmm0, %xmm0
	vaddsd	%xmm1, %xmm0, %xmm0
	vsubsd	%xmm4, %xmm0, %xmm0
	vaddsd	%xmm0, %xmm5, %xmm5
	cmpq	%rax, %rdx
	jne	.L2
	vmovapd	%xmm5, %xmm0
	ret
	.cfi_endproc
.LFE20:
	.size	round_family_pass, .-round_family_pass
	.p2align 4
	.globl	rint
	.type	rint, @function
rint:
.LFB23:
	.cfi_startproc
	vxorpd	%xmm1, %xmm1, %xmm1
	vcomisd	%xmm1, %xmm0
	vmovsd	.LC3(%rip), %xmm1
	jb	.L10
	vaddsd	%xmm1, %xmm0, %xmm0
	vsubsd	%xmm1, %xmm0, %xmm0
	ret
	.p2align 4,,10
	.p2align 3
.L10:
	vsubsd	%xmm1, %xmm0, %xmm0
	vaddsd	%xmm1, %xmm0, %xmm0
	ret
	.cfi_endproc
.LFE23:
	.size	rint, .-rint
	.p2align 4
	.globl	nearbyint
	.type	nearbyint, @function
nearbyint:
.LFB13:
	.cfi_startproc
	vxorpd	%xmm1, %xmm1, %xmm1
	vcomisd	%xmm1, %xmm0
	vmovsd	.LC3(%rip), %xmm1
	jb	.L16
	vaddsd	%xmm1, %xmm0, %xmm0
	vsubsd	%xmm1, %xmm0, %xmm0
	ret
	.p2align 4,,10
	.p2align 3
.L16:
	vsubsd	%xmm1, %xmm0, %xmm0
	vaddsd	%xmm1, %xmm0, %xmm0
	ret
	.cfi_endproc
.LFE13:
	.size	nearbyint, .-nearbyint
	.p2align 4
	.globl	roundeven
	.type	roundeven, @function
roundeven:
.LFB25:
	.cfi_startproc
	vxorpd	%xmm1, %xmm1, %xmm1
	vcomisd	%xmm1, %xmm0
	vmovsd	.LC3(%rip), %xmm1
	jb	.L22
	vaddsd	%xmm1, %xmm0, %xmm0
	vsubsd	%xmm1, %xmm0, %xmm0
	ret
	.p2align 4,,10
	.p2align 3
.L22:
	vsubsd	%xmm1, %xmm0, %xmm0
	vaddsd	%xmm1, %xmm0, %xmm0
	ret
	.cfi_endproc
.LFE25:
	.size	roundeven, .-roundeven
	.p2align 4
	.globl	floor
	.type	floor, @function
floor:
.LFB15:
	.cfi_startproc
	vmovapd	%xmm0, %xmm2
	vxorpd	%xmm0, %xmm0, %xmm0
	vcomisd	%xmm0, %xmm2
	jb	.L31
	vmovsd	.LC3(%rip), %xmm1
	vaddsd	%xmm1, %xmm2, %xmm0
	vsubsd	%xmm1, %xmm0, %xmm0
.L26:
	vcomisd	%xmm2, %xmm0
	jbe	.L23
	vsubsd	.LC4(%rip), %xmm0, %xmm0
.L23:
	ret
	.p2align 4,,10
	.p2align 3
.L31:
	vmovsd	.LC3(%rip), %xmm0
	vsubsd	%xmm0, %xmm2, %xmm1
	vaddsd	%xmm0, %xmm1, %xmm0
	jmp	.L26
	.cfi_endproc
.LFE15:
	.size	floor, .-floor
	.p2align 4
	.globl	ceil
	.type	ceil, @function
ceil:
.LFB16:
	.cfi_startproc
	vmovapd	%xmm0, %xmm2
	vxorpd	%xmm0, %xmm0, %xmm0
	vcomisd	%xmm0, %xmm2
	jb	.L40
	vmovsd	.LC3(%rip), %xmm1
	vaddsd	%xmm1, %xmm2, %xmm0
	vsubsd	%xmm1, %xmm0, %xmm0
.L35:
	vcomisd	%xmm0, %xmm2
	jbe	.L32
	vaddsd	.LC4(%rip), %xmm0, %xmm0
.L32:
	ret
	.p2align 4,,10
	.p2align 3
.L40:
	vmovsd	.LC3(%rip), %xmm0
	vsubsd	%xmm0, %xmm2, %xmm1
	vaddsd	%xmm0, %xmm1, %xmm0
	jmp	.L35
	.cfi_endproc
.LFE16:
	.size	ceil, .-ceil
	.p2align 4
	.globl	trunc
	.type	trunc, @function
trunc:
.LFB17:
	.cfi_startproc
	vmovapd	%xmm0, %xmm2
	vxorpd	%xmm0, %xmm0, %xmm0
	vcomisd	%xmm0, %xmm2
	jb	.L48
	vmovsd	.LC3(%rip), %xmm1
	vaddsd	%xmm1, %xmm2, %xmm0
	vsubsd	%xmm1, %xmm0, %xmm0
	vcomisd	%xmm2, %xmm0
	jbe	.L49
	vsubsd	.LC4(%rip), %xmm0, %xmm0
	ret
	.p2align 4,,10
	.p2align 3
.L48:
	vmovsd	.LC3(%rip), %xmm0
	vsubsd	%xmm0, %xmm2, %xmm1
	vaddsd	%xmm0, %xmm1, %xmm0
	vcomisd	%xmm0, %xmm2
	jbe	.L50
	vaddsd	.LC4(%rip), %xmm0, %xmm0
	ret
	.p2align 4,,10
	.p2align 3
.L49:
	ret
	.p2align 4,,10
	.p2align 3
.L50:
	ret
	.cfi_endproc
.LFE17:
	.size	trunc, .-trunc
	.p2align 4
	.globl	copysign
	.type	copysign, @function
copysign:
.LFB18:
	.cfi_startproc
	vmovq	%xmm0, %rax
	vmovq	%xmm1, %rcx
	movabsq	$-9223372036854775808, %rdx
	btrq	$63, %rax
	andq	%rcx, %rdx
	orq	%rdx, %rax
	vmovq	%rax, %xmm0
	ret
	.cfi_endproc
.LFE18:
	.size	copysign, .-copysign
	.p2align 4
	.globl	fma
	.type	fma, @function
fma:
.LFB19:
	.cfi_startproc
	vfmadd132sd	%xmm1, %xmm2, %xmm0
	ret
	.cfi_endproc
.LFE19:
	.size	fma, .-fma
	.section	.rodata.str1.1,"aMS",@progbits,1
.LC13:
	.string	"libm-round-family: %.6f\n"
	.section	.text.startup,"ax",@progbits
	.p2align 4
	.globl	main
	.type	main, @function
main:
.LFB21:
	.cfi_startproc
	leaq	8(%rsp), %r10
	.cfi_def_cfa 10, 0
	andq	$-32, %rsp
	movl	$8, %ecx
	pushq	-8(%r10)
	vmovd	%ecx, %xmm9
	movl	$-2048, %ecx
	leaq	buf(%rip), %rsi
	pushq	%rbp
	vmovd	%ecx, %xmm8
	movl	$3, %ecx
	movq	%rsi, %rax
	vmovd	%ecx, %xmm7
	vpbroadcastd	%xmm9, %ymm9
	vpbroadcastd	%xmm8, %ymm8
	leaq	32768(%rsi), %rdx
	vpbroadcastd	%xmm7, %ymm7
	movq	%rsp, %rbp
	.cfi_escape 0x10,0x6,0x2,0x76,0
	pushq	%r10
	.cfi_escape 0xf,0x3,0x76,0x78,0x6
	subq	$8, %rsp
	vmovdqa	.LC5(%rip), %ymm4
	vbroadcastsd	.LC10(%rip), %ymm6
	vbroadcastsd	.LC12(%rip), %ymm5
	.p2align 4
	.p2align 3
.L54:
	vmovdqa	%ymm4, %ymm0
	addq	$64, %rax
	vpaddd	%ymm9, %ymm4, %ymm4
	vpaddd	%ymm8, %ymm0, %ymm1
	vpand	%ymm7, %ymm0, %ymm0
	vextracti128	$0x1, %ymm0, %xmm3
	vcvtdq2pd	%xmm0, %ymm0
	vextracti128	$0x1, %ymm1, %xmm2
	vcvtdq2pd	%xmm1, %ymm1
	vmulpd	%ymm6, %ymm0, %ymm0
	vcvtdq2pd	%xmm3, %ymm3
	vcvtdq2pd	%xmm2, %ymm2
	vmulpd	%ymm6, %ymm3, %ymm3
	vfmadd132pd	%ymm5, %ymm0, %ymm1
	vfmadd132pd	%ymm5, %ymm3, %ymm2
	vmovapd	%ymm1, -64(%rax)
	vmovapd	%ymm2, -32(%rax)
	cmpq	%rax, %rdx
	jne	.L54
	vmovq	.LC1(%rip), %xmm12
	xorl	%ecx, %ecx
	vxorpd	%xmm11, %xmm11, %xmm11
	.p2align 4
	.p2align 3
.L55:
	call	round_family_pass
	movl	%ecx, %eax
	addl	$1, %ecx
	andl	$4095, %eax
	vaddsd	%xmm0, %xmm11, %xmm11
	vmovsd	(%rsi,%rax,8), %xmm0
	vxorpd	%xmm12, %xmm0, %xmm0
	vmovsd	%xmm0, (%rsi,%rax,8)
	cmpl	$20000, %ecx
	jne	.L55
	vmovapd	%xmm11, %xmm0
	leaq	.LC13(%rip), %rdi
	movl	$1, %eax
	vzeroupper
	call	printf@PLT
	addq	$8, %rsp
	xorl	%eax, %eax
	popq	%r10
	.cfi_def_cfa 10, 0
	popq	%rbp
	leaq	-8(%r10), %rsp
	.cfi_def_cfa 7, 8
	ret
	.cfi_endproc
.LFE21:
	.size	main, .-main
	.local	buf
	.comm	buf,32768,32
	.section	.rodata.cst16,"aM",@progbits,16
	.align 16
.LC1:
	.long	0
	.long	-2147483648
	.long	0
	.long	0
	.section	.rodata.cst8,"aM",@progbits,8
	.align 8
.LC2:
	.long	0
	.long	1071644672
	.align 8
.LC3:
	.long	0
	.long	1127219200
	.align 8
.LC4:
	.long	0
	.long	1072693248
	.section	.rodata.cst32,"aM",@progbits,32
	.align 32
.LC5:
	.long	0
	.long	1
	.long	2
	.long	3
	.long	4
	.long	5
	.long	6
	.long	7
	.section	.rodata.cst8
	.align 8
.LC10:
	.long	0
	.long	1070596096
	.align 8
.LC12:
	.long	-353218110
	.long	1071121180
	.ident	"GCC: (Debian 14.2.0-19) 14.2.0"
	.section	.note.GNU-stack,"",@progbits
