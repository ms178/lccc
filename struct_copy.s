	.file	"struct_copy.c"
	.text
	.section	.rodata.str1.1,"aMS",@progbits,1
.LC3:
	.string	"struct_copy total: %.2f\n"
	.section	.text.startup,"ax",@progbits
	.p2align 4
	.globl	main
	.type	main, @function
main:
.LFB14:
	.cfi_startproc
	subq	$40, %rsp
	.cfi_def_cfa_offset 48
	vmovsd	.LC2(%rip), %xmm11
	xorl	%edx, %edx
	vxorpd	%xmm12, %xmm12, %xmm12
	movq	$0x000000000, 16(%rsp)
	.p2align 4
	.p2align 3
.L2:
	vxorpd	%xmm5, %xmm5, %xmm5
	vmovapd	%xmm12, %xmm1
	vmovddup	%xmm12, %xmm15
	movl	%edx, %ecx
	addl	$1, %edx
	leal	2(%rcx), %edi
	vmovapd	%xmm1, %xmm9
	addl	$3, %ecx
	vcvtsi2sdl	%edx, %xmm5, %xmm12
	vmulpd	.LC1(%rip), %xmm15, %xmm15
	vcvtsi2sdl	%ecx, %xmm5, %xmm8
	vcvtsi2sdl	%edi, %xmm5, %xmm0
	vmulsd	%xmm11, %xmm12, %xmm6
	vmovddup	%xmm12, %xmm3
	vmulpd	.LC1(%rip), %xmm3, %xmm3
	vunpckhpd	%xmm15, %xmm15, %xmm13
	vmovddup	%xmm8, %xmm5
	vmulsd	%xmm11, %xmm8, %xmm14
	vmovddup	%xmm0, %xmm2
	vmulpd	.LC1(%rip), %xmm2, %xmm2
	vmulpd	.LC1(%rip), %xmm5, %xmm5
	vmulsd	%xmm11, %xmm0, %xmm0
	vfmsub132sd	%xmm11, %xmm6, %xmm9
	vunpckhpd	%xmm3, %xmm3, %xmm7
	vsubsd	%xmm3, %xmm15, %xmm8
	vsubsd	%xmm7, %xmm13, %xmm10
	vmovsd	%xmm14, 24(%rsp)
	vunpckhpd	%xmm2, %xmm2, %xmm4
	vsubsd	%xmm4, %xmm13, %xmm14
	vmovhpd	%xmm5, 8(%rsp)
	vsubsd	8(%rsp), %xmm13, %xmm13
	vmulsd	%xmm10, %xmm10, %xmm10
	vmulsd	%xmm14, %xmm14, %xmm14
	vmulsd	%xmm13, %xmm13, %xmm13
	vfmadd132sd	%xmm8, %xmm10, %xmm8
	vxorpd	%xmm10, %xmm10, %xmm10
	vfmadd132sd	%xmm9, %xmm8, %xmm9
	vmovapd	%xmm1, %xmm8
	vfmsub132sd	%xmm11, %xmm0, %xmm8
	vaddsd	%xmm10, %xmm9, %xmm9
	vsubsd	%xmm2, %xmm15, %xmm10
	vsubsd	%xmm5, %xmm15, %xmm15
	vfmadd132sd	%xmm10, %xmm14, %xmm10
	vmovsd	24(%rsp), %xmm14
	vfmadd132sd	%xmm15, %xmm13, %xmm15
	vmovsd	8(%rsp), %xmm13
	vfmsub132sd	%xmm11, %xmm14, %xmm1
	vfmadd132sd	%xmm8, %xmm10, %xmm8
	vsubsd	%xmm4, %xmm7, %xmm10
	vsubsd	%xmm13, %xmm7, %xmm7
	vsubsd	%xmm13, %xmm4, %xmm4
	vfmadd132sd	%xmm1, %xmm15, %xmm1
	vmulsd	%xmm10, %xmm10, %xmm10
	vmulsd	%xmm7, %xmm7, %xmm7
	vaddsd	%xmm9, %xmm8, %xmm8
	vsubsd	%xmm2, %xmm3, %xmm9
	vmulsd	%xmm4, %xmm4, %xmm4
	vsubsd	%xmm5, %xmm3, %xmm3
	vsubsd	%xmm5, %xmm2, %xmm2
	vfmadd132sd	%xmm9, %xmm10, %xmm9
	vaddsd	%xmm1, %xmm8, %xmm1
	vsubsd	%xmm0, %xmm6, %xmm8
	vfmadd132sd	%xmm3, %xmm7, %xmm3
	vsubsd	%xmm14, %xmm6, %xmm6
	vsubsd	%xmm14, %xmm0, %xmm0
	vfmadd132sd	%xmm2, %xmm4, %xmm2
	vfmadd132sd	%xmm8, %xmm9, %xmm8
	vfmadd132sd	%xmm6, %xmm3, %xmm6
	vfmadd132sd	%xmm0, %xmm2, %xmm0
	vaddsd	%xmm8, %xmm1, %xmm1
	vaddsd	%xmm6, %xmm1, %xmm1
	vaddsd	%xmm1, %xmm0, %xmm0
	vaddsd	16(%rsp), %xmm0, %xmm4
	vmovsd	%xmm4, 16(%rsp)
	cmpl	$2000000, %edx
	jne	.L2
	vmovapd	%xmm4, %xmm0
	leaq	.LC3(%rip), %rdi
	movl	$1, %eax
	call	printf@PLT
	xorl	%eax, %eax
	addq	$40, %rsp
	.cfi_def_cfa_offset 8
	ret
	.cfi_endproc
.LFE14:
	.size	main, .-main
	.section	.rodata.cst16,"aM",@progbits,16
	.align 16
.LC1:
	.long	-1717986918
	.long	1069128089
	.long	-1717986918
	.long	1070176665
	.section	.rodata.cst8,"aM",@progbits,8
	.align 8
.LC2:
	.long	858993459
	.long	1070805811
	.ident	"GCC: (Debian 14.2.0-19) 14.2.0"
	.section	.note.GNU-stack,"",@progbits
