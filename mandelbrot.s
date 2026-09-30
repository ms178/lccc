	.file	"mandelbrot.c"
	.text
	.section	.rodata.str1.8,"aMS",@progbits,1
	.align 8
.LC5:
	.string	"mandelbrot total iterations: %d\n"
	.section	.text.startup,"ax",@progbits
	.p2align 4
	.globl	main
	.type	main, @function
main:
.LFB11:
	.cfi_startproc
	subq	$8, %rsp
	.cfi_def_cfa_offset 16
	vmovsd	.LC1(%rip), %xmm7
	xorl	%ecx, %ecx
	xorl	%esi, %esi
	vmovsd	.LC3(%rip), %xmm9
	vmovsd	.LC4(%rip), %xmm6
	vxorps	%xmm8, %xmm8, %xmm8
.L5:
	vcvtsi2sdl	%ecx, %xmm8, %xmm0
	xorl	%edx, %edx
	vaddsd	%xmm0, %xmm0, %xmm0
	vdivsd	%xmm7, %xmm0, %xmm5
	vsubsd	.LC2(%rip), %xmm5, %xmm5
	.p2align 4
	.p2align 3
.L4:
	vcvtsi2sdl	%edx, %xmm8, %xmm4
	vxorpd	%xmm3, %xmm3, %xmm3
	xorl	%eax, %eax
	vmovapd	%xmm3, %xmm0
	vmovapd	%xmm3, %xmm1
	vaddsd	%xmm4, %xmm4, %xmm4
	vdivsd	%xmm7, %xmm4, %xmm4
	vsubsd	%xmm9, %xmm4, %xmm4
	jmp	.L3
	.p2align 6
	.p2align 4,,10
	.p2align 3
.L11:
	addl	$1, %eax
	cmpl	$50, %eax
	je	.L2
.L3:
	vmovapd	%xmm0, %xmm2
	vfmsub231sd	%xmm0, %xmm0, %xmm1
	vaddsd	%xmm2, %xmm2, %xmm2
	vfmadd132sd	%xmm2, %xmm5, %xmm3
	vaddsd	%xmm4, %xmm1, %xmm0
	vmovapd	%xmm0, %xmm2
	vmulsd	%xmm3, %xmm3, %xmm1
	vfmadd132sd	%xmm0, %xmm1, %xmm2
	vcomisd	%xmm6, %xmm2
	jbe	.L11
.L2:
	addl	$1, %edx
	addl	%eax, %esi
	cmpl	$4000, %edx
	jne	.L4
	addl	$1, %ecx
	cmpl	$4000, %ecx
	jne	.L5
	leaq	.LC5(%rip), %rdi
	xorl	%eax, %eax
	call	printf@PLT
	xorl	%eax, %eax
	addq	$8, %rsp
	.cfi_def_cfa_offset 8
	ret
	.cfi_endproc
.LFE11:
	.size	main, .-main
	.section	.rodata.cst8,"aM",@progbits,8
	.align 8
.LC1:
	.long	0
	.long	1085227008
	.align 8
.LC2:
	.long	0
	.long	1072693248
	.align 8
.LC3:
	.long	0
	.long	1073217536
	.align 8
.LC4:
	.long	0
	.long	1074790400
	.ident	"GCC: (Debian 14.2.0-19) 14.2.0"
	.section	.note.GNU-stack,"",@progbits
