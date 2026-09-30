	.file	"nbody.c"
	.text
	.p2align 4
	.type	energy, @function
energy:
.LFB12:
	.cfi_startproc
	vmovsd	32+bodies(%rip), %xmm1
	vmovsd	24+bodies(%rip), %xmm0
	vmovsd	.LC0(%rip), %xmm13
	vmovsd	48+bodies(%rip), %xmm10
	vmulsd	%xmm1, %xmm1, %xmm1
	vmovsd	8+bodies(%rip), %xmm14
	vmovsd	bodies(%rip), %xmm11
	vmulsd	%xmm13, %xmm10, %xmm2
	vmovsd	56+bodies(%rip), %xmm12
	vmovsd	16+bodies(%rip), %xmm3
	vsubsd	72+bodies(%rip), %xmm3, %xmm4
	vmovsd	104+bodies(%rip), %xmm9
	vsubsd	%xmm12, %xmm11, %xmm5
	vmovsd	120+bodies(%rip), %xmm8
	vmulsd	%xmm9, %xmm10, %xmm7
	vfmadd231sd	%xmm0, %xmm0, %xmm1
	vmovsd	40+bodies(%rip), %xmm0
	vfmadd132sd	%xmm0, %xmm1, %xmm0
	vxorpd	%xmm1, %xmm1, %xmm1
	vfmadd132sd	%xmm2, %xmm1, %xmm0
	vmovsd	64+bodies(%rip), %xmm2
	vsubsd	%xmm2, %xmm14, %xmm6
	vmulsd	%xmm6, %xmm6, %xmm6
	vfmadd132sd	%xmm5, %xmm6, %xmm5
	vmovsd	160+bodies(%rip), %xmm6
	vmulsd	%xmm6, %xmm10, %xmm1
	vfmadd132sd	%xmm4, %xmm5, %xmm4
	vsubsd	%xmm8, %xmm14, %xmm5
	vmulsd	%xmm5, %xmm5, %xmm5
	vsqrtsd	%xmm4, %xmm4, %xmm4
	vdivsd	%xmm4, %xmm7, %xmm4
	vmovsd	112+bodies(%rip), %xmm7
	vsubsd	%xmm7, %xmm11, %xmm15
	vfmadd231sd	%xmm15, %xmm15, %xmm5
	vsubsd	168+bodies(%rip), %xmm11, %xmm15
	vsubsd	224+bodies(%rip), %xmm11, %xmm11
	vsubsd	%xmm4, %xmm0, %xmm0
	vsubsd	128+bodies(%rip), %xmm3, %xmm4
	vfmadd132sd	%xmm4, %xmm5, %xmm4
	vsqrtsd	%xmm4, %xmm4, %xmm4
	vdivsd	%xmm4, %xmm1, %xmm4
	vsubsd	%xmm4, %xmm0, %xmm5
	vsubsd	176+bodies(%rip), %xmm14, %xmm0
	vsubsd	184+bodies(%rip), %xmm3, %xmm4
	vmulsd	%xmm0, %xmm0, %xmm0
	vmovq	%xmm5, %rax
	vmovsd	216+bodies(%rip), %xmm5
	vmulsd	%xmm5, %xmm10, %xmm1
	vfmadd132sd	%xmm15, %xmm0, %xmm15
	vfmadd132sd	%xmm4, %xmm15, %xmm4
	vmovsd	232+bodies(%rip), %xmm15
	vsubsd	%xmm15, %xmm14, %xmm0
	vmovsd	240+bodies(%rip), %xmm14
	vsubsd	%xmm14, %xmm3, %xmm3
	vmulsd	%xmm0, %xmm0, %xmm0
	vsqrtsd	%xmm4, %xmm4, %xmm4
	vdivsd	%xmm4, %xmm1, %xmm4
	vmovq	%rax, %xmm1
	vfmadd132sd	%xmm11, %xmm0, %xmm11
	vfmadd132sd	%xmm3, %xmm11, %xmm3
	vmovsd	80+bodies(%rip), %xmm11
	vsqrtsd	%xmm3, %xmm3, %xmm3
	vsubsd	%xmm4, %xmm1, %xmm4
	vmovsd	72+bodies(%rip), %xmm1
	vmovq	%xmm4, %rax
	vmovsd	272+bodies(%rip), %xmm4
	vmulsd	%xmm4, %xmm10, %xmm10
	vdivsd	%xmm3, %xmm10, %xmm10
	vmovq	%rax, %xmm3
	vsubsd	%xmm10, %xmm3, %xmm0
	vmovsd	88+bodies(%rip), %xmm10
	vmulsd	%xmm13, %xmm9, %xmm3
	vmulsd	%xmm10, %xmm10, %xmm10
	vfmadd231sd	%xmm11, %xmm11, %xmm10
	vmovsd	96+bodies(%rip), %xmm11
	vfmadd132sd	%xmm11, %xmm10, %xmm11
	vsubsd	%xmm7, %xmm12, %xmm10
	vfmadd132sd	%xmm11, %xmm0, %xmm3
	vsubsd	%xmm8, %xmm2, %xmm11
	vsubsd	128+bodies(%rip), %xmm1, %xmm0
	vmulsd	%xmm6, %xmm9, %xmm1
	vmulsd	%xmm11, %xmm11, %xmm11
	vfmadd132sd	%xmm10, %xmm11, %xmm10
	vsubsd	176+bodies(%rip), %xmm2, %xmm11
	vsubsd	%xmm15, %xmm2, %xmm2
	vmulsd	%xmm11, %xmm11, %xmm11
	vmulsd	%xmm2, %xmm2, %xmm2
	vfmadd132sd	%xmm0, %xmm10, %xmm0
	vsubsd	168+bodies(%rip), %xmm12, %xmm10
	vfmadd132sd	%xmm10, %xmm11, %xmm10
	vsqrtsd	%xmm0, %xmm0, %xmm0
	vdivsd	%xmm0, %xmm1, %xmm0
	vmovsd	72+bodies(%rip), %xmm1
	vsubsd	%xmm0, %xmm3, %xmm3
	vsubsd	184+bodies(%rip), %xmm1, %xmm0
	vmulsd	%xmm5, %xmm9, %xmm1
	vsubsd	224+bodies(%rip), %xmm12, %xmm12
	vmulsd	%xmm4, %xmm9, %xmm9
	vfmadd132sd	%xmm0, %xmm10, %xmm0
	vfmadd231sd	%xmm12, %xmm12, %xmm2
	vsqrtsd	%xmm0, %xmm0, %xmm0
	vdivsd	%xmm0, %xmm1, %xmm0
	vsubsd	%xmm0, %xmm3, %xmm0
	vmovsd	72+bodies(%rip), %xmm3
	vsubsd	%xmm14, %xmm3, %xmm1
	vmovsd	136+bodies(%rip), %xmm3
	vfmadd132sd	%xmm1, %xmm2, %xmm1
	vmovsd	144+bodies(%rip), %xmm2
	vmulsd	%xmm2, %xmm2, %xmm2
	vsqrtsd	%xmm1, %xmm1, %xmm1
	vfmadd231sd	%xmm3, %xmm3, %xmm2
	vmovsd	152+bodies(%rip), %xmm3
	vdivsd	%xmm1, %xmm9, %xmm9
	vmulsd	%xmm13, %xmm6, %xmm1
	vfmadd132sd	%xmm3, %xmm2, %xmm3
	vmovsd	128+bodies(%rip), %xmm2
	vsubsd	184+bodies(%rip), %xmm2, %xmm2
	vsubsd	%xmm9, %xmm0, %xmm0
	vsubsd	168+bodies(%rip), %xmm7, %xmm9
	vsubsd	224+bodies(%rip), %xmm7, %xmm7
	vfmadd231sd	%xmm3, %xmm1, %xmm0
	vsubsd	176+bodies(%rip), %xmm8, %xmm3
	vsubsd	%xmm15, %xmm8, %xmm8
	vmulsd	%xmm5, %xmm6, %xmm1
	vmulsd	%xmm4, %xmm6, %xmm6
	vmulsd	%xmm3, %xmm3, %xmm3
	vmulsd	%xmm8, %xmm8, %xmm8
	vfmadd231sd	%xmm9, %xmm9, %xmm3
	vfmadd132sd	%xmm7, %xmm8, %xmm7
	vfmadd132sd	%xmm2, %xmm3, %xmm2
	vmovsd	128+bodies(%rip), %xmm3
	vsqrtsd	%xmm2, %xmm2, %xmm2
	vdivsd	%xmm2, %xmm1, %xmm1
	vsubsd	%xmm14, %xmm3, %xmm2
	vmulsd	%xmm13, %xmm5, %xmm3
	vmulsd	%xmm4, %xmm5, %xmm5
	vmulsd	%xmm13, %xmm4, %xmm4
	vfmadd132sd	%xmm2, %xmm7, %xmm2
	vmovsd	168+bodies(%rip), %xmm7
	vsqrtsd	%xmm2, %xmm2, %xmm2
	vdivsd	%xmm2, %xmm6, %xmm6
	vmovsd	200+bodies(%rip), %xmm2
	vmulsd	%xmm2, %xmm2, %xmm2
	vsubsd	%xmm1, %xmm0, %xmm0
	vmovsd	192+bodies(%rip), %xmm1
	vfmadd231sd	%xmm1, %xmm1, %xmm2
	vmovsd	208+bodies(%rip), %xmm1
	vfmadd132sd	%xmm1, %xmm2, %xmm1
	vsubsd	%xmm6, %xmm0, %xmm0
	vmovsd	184+bodies(%rip), %xmm6
	vfmadd132sd	%xmm1, %xmm0, %xmm3
	vsubsd	224+bodies(%rip), %xmm7, %xmm1
	vsubsd	%xmm14, %xmm6, %xmm0
	vmovsd	176+bodies(%rip), %xmm7
	vsubsd	%xmm15, %xmm7, %xmm2
	vmulsd	%xmm2, %xmm2, %xmm2
	vfmadd132sd	%xmm1, %xmm2, %xmm1
	vmovsd	256+bodies(%rip), %xmm2
	vmulsd	%xmm2, %xmm2, %xmm2
	vfmadd132sd	%xmm0, %xmm1, %xmm0
	vmovsd	248+bodies(%rip), %xmm1
	vfmadd231sd	%xmm1, %xmm1, %xmm2
	vmovsd	264+bodies(%rip), %xmm1
	vsqrtsd	%xmm0, %xmm0, %xmm0
	vfmadd132sd	%xmm1, %xmm2, %xmm1
	vdivsd	%xmm0, %xmm5, %xmm5
	vsubsd	%xmm5, %xmm3, %xmm0
	vfmadd231sd	%xmm1, %xmm4, %xmm0
	ret
	.cfi_endproc
.LFE12:
	.size	energy, .-energy
	.section	.rodata.str1.1,"aMS",@progbits,1
.LC3:
	.string	"%.9f\n"
	.section	.text.startup,"ax",@progbits
	.p2align 4
	.globl	main
	.type	main, @function
main:
.LFB13:
	.cfi_startproc
	vmovsd	48+bodies(%rip), %xmm2
	vxorpd	%xmm3, %xmm3, %xmm3
	vmovsd	.LC2(%rip), %xmm4
	pushq	%rbx
	.cfi_def_cfa_offset 16
	.cfi_offset 3, -16
	leaq	.LC3(%rip), %rbx
	vmovapd	%xmm2, %xmm1
	vmovapd	%xmm2, %xmm0
	movq	%rbx, %rdi
	vfmadd132sd	24+bodies(%rip), %xmm3, %xmm1
	vfmadd132sd	32+bodies(%rip), %xmm3, %xmm0
	vfmadd231sd	40+bodies(%rip), %xmm2, %xmm3
	vmovsd	104+bodies(%rip), %xmm2
	vfmadd231sd	80+bodies(%rip), %xmm2, %xmm1
	vfmadd231sd	88+bodies(%rip), %xmm2, %xmm0
	vfmadd231sd	96+bodies(%rip), %xmm2, %xmm3
	vmovsd	160+bodies(%rip), %xmm2
	vfmadd231sd	136+bodies(%rip), %xmm2, %xmm1
	vfmadd231sd	144+bodies(%rip), %xmm2, %xmm0
	vfmadd231sd	152+bodies(%rip), %xmm2, %xmm3
	vmovsd	216+bodies(%rip), %xmm2
	vfmadd231sd	208+bodies(%rip), %xmm2, %xmm3
	vfmadd231sd	192+bodies(%rip), %xmm2, %xmm1
	vfmadd231sd	200+bodies(%rip), %xmm2, %xmm0
	vmovsd	272+bodies(%rip), %xmm2
	vfnmsub231sd	248+bodies(%rip), %xmm2, %xmm1
	vfnmsub231sd	256+bodies(%rip), %xmm2, %xmm0
	vfnmsub132sd	264+bodies(%rip), %xmm3, %xmm2
	vdivsd	%xmm4, %xmm1, %xmm1
	vdivsd	%xmm4, %xmm0, %xmm0
	vmovsd	%xmm1, 24+bodies(%rip)
	vdivsd	%xmm4, %xmm2, %xmm2
	vmovsd	%xmm0, 32+bodies(%rip)
	vmovsd	%xmm2, 40+bodies(%rip)
	call	energy
	movl	$1, %eax
	call	printf@PLT
	vmovapd	bodies(%rip), %xmm8
	movq	16+bodies(%rip), %rsi
	movl	$5000000, %ecx
	vmovddup	.LC5(%rip), %xmm7
	vmovsd	.LC5(%rip), %xmm6
	leaq	bodies(%rip), %rdi
	.p2align 4
	.p2align 3
.L4:
	leaq	bodies(%rip), %rax
	movl	$1, %edx
.L10:
	vmovupd	(%rax), %xmm3
	vsubpd	56(%rax), %xmm3, %xmm13
	leal	1(%rdx), %r8d
	vmovsd	16(%rax), %xmm10
	vsubsd	72(%rax), %xmm10, %xmm0
	vmovsd	48(%rax), %xmm4
	vunpckhpd	%xmm13, %xmm13, %xmm1
	vmovapd	%xmm13, %xmm2
	vmulsd	%xmm1, %xmm1, %xmm1
	vmovddup	%xmm4, %xmm5
	vmulpd	%xmm13, %xmm5, %xmm12
	vfmadd132sd	%xmm13, %xmm1, %xmm2
	vfmadd231sd	%xmm0, %xmm0, %xmm2
	vsqrtsd	%xmm2, %xmm2, %xmm1
	vmulsd	%xmm2, %xmm1, %xmm1
	vmovsd	104(%rax), %xmm2
	vmovddup	%xmm2, %xmm9
	vmulpd	%xmm13, %xmm9, %xmm9
	vmulsd	%xmm0, %xmm2, %xmm2
	vdivsd	%xmm1, %xmm6, %xmm1
	vmulsd	%xmm0, %xmm4, %xmm0
	vfnmadd213sd	40(%rax), %xmm1, %xmm2
	vfmadd213sd	96(%rax), %xmm1, %xmm0
	vmovddup	%xmm1, %xmm11
	vfnmadd213pd	24(%rax), %xmm11, %xmm9
	vfmadd213pd	80(%rax), %xmm12, %xmm11
	vmovsd	%xmm2, 40(%rax)
	vmovsd	%xmm0, 96(%rax)
	vmovupd	%xmm9, 24(%rax)
	vmovupd	%xmm11, 80(%rax)
	cmpl	$4, %edx
	je	.L7
	vsubpd	112(%rax), %xmm3, %xmm13
	vsubsd	128(%rax), %xmm10, %xmm0
	vmovsd	160(%rax), %xmm14
	vunpckhpd	%xmm13, %xmm13, %xmm1
	vmovapd	%xmm13, %xmm11
	vmovddup	%xmm14, %xmm15
	vmulsd	%xmm1, %xmm1, %xmm1
	vmulsd	%xmm0, %xmm14, %xmm14
	vmulpd	%xmm13, %xmm15, %xmm15
	vmulpd	%xmm13, %xmm5, %xmm12
	vfmadd132sd	%xmm13, %xmm1, %xmm11
	vfmadd231sd	%xmm0, %xmm0, %xmm11
	vmulsd	%xmm0, %xmm4, %xmm0
	vsqrtsd	%xmm11, %xmm11, %xmm1
	vmulsd	%xmm11, %xmm1, %xmm1
	vdivsd	%xmm1, %xmm6, %xmm1
	vfmadd213sd	152(%rax), %xmm1, %xmm0
	vmovddup	%xmm1, %xmm11
	vfnmadd231sd	%xmm1, %xmm14, %xmm2
	vfnmadd231pd	%xmm11, %xmm15, %xmm9
	vmovsd	%xmm0, 152(%rax)
	vfmadd213pd	136(%rax), %xmm11, %xmm12
	vmovsd	%xmm2, 40(%rax)
	vmovupd	%xmm9, 24(%rax)
	vmovupd	%xmm12, 136(%rax)
	cmpl	$3, %edx
	je	.L9
	vsubpd	168(%rax), %xmm3, %xmm14
	vsubsd	184(%rax), %xmm10, %xmm0
	vmulpd	%xmm5, %xmm14, %xmm13
	vunpckhpd	%xmm14, %xmm14, %xmm1
	vmovapd	%xmm14, %xmm11
	vmulsd	%xmm1, %xmm1, %xmm1
	vfmadd132sd	%xmm14, %xmm1, %xmm11
	vfmadd231sd	%xmm0, %xmm0, %xmm11
	vsqrtsd	%xmm11, %xmm11, %xmm1
	vmulsd	%xmm11, %xmm1, %xmm1
	vmovsd	216(%rax), %xmm11
	vmovddup	%xmm11, %xmm15
	vmulpd	%xmm14, %xmm15, %xmm15
	vmulsd	%xmm0, %xmm11, %xmm11
	vdivsd	%xmm1, %xmm6, %xmm1
	vmulsd	%xmm0, %xmm4, %xmm0
	vfmadd213sd	208(%rax), %xmm1, %xmm0
	vmovddup	%xmm1, %xmm12
	vfnmadd132sd	%xmm1, %xmm2, %xmm11
	vfnmadd231pd	%xmm12, %xmm15, %xmm9
	vmovsd	%xmm0, 208(%rax)
	vfmadd213pd	192(%rax), %xmm13, %xmm12
	vmovsd	%xmm11, 40(%rax)
	vmovupd	%xmm9, 24(%rax)
	vmovupd	%xmm12, 192(%rax)
	cmpl	$2, %edx
	je	.L9
	vsubpd	224(%rax), %xmm3, %xmm3
	vsubsd	240(%rax), %xmm10, %xmm10
	vmulpd	%xmm3, %xmm5, %xmm5
	vunpckhpd	%xmm3, %xmm3, %xmm1
	vmovapd	%xmm3, %xmm0
	vmulsd	%xmm1, %xmm1, %xmm1
	vfmadd132sd	%xmm3, %xmm1, %xmm0
	vfmadd231sd	%xmm10, %xmm10, %xmm0
	vsqrtsd	%xmm0, %xmm0, %xmm1
	vmulsd	%xmm1, %xmm0, %xmm0
	vmovsd	272(%rax), %xmm1
	vmovddup	%xmm1, %xmm2
	vmulpd	%xmm3, %xmm2, %xmm2
	vmulsd	%xmm1, %xmm10, %xmm1
	vdivsd	%xmm0, %xmm6, %xmm0
	vmulsd	%xmm4, %xmm10, %xmm10
	vmovddup	%xmm0, %xmm12
	vfnmadd132sd	%xmm0, %xmm11, %xmm1
	vfmadd213sd	264(%rax), %xmm10, %xmm0
	vfnmadd132pd	%xmm12, %xmm9, %xmm2
	vfmadd213pd	248(%rax), %xmm5, %xmm12
	vmovsd	%xmm1, 40(%rax)
	vmovupd	%xmm2, 24(%rax)
	vmovupd	%xmm12, 248(%rax)
	vmovsd	%xmm0, 264(%rax)
.L9:
	addq	$56, %rax
	movl	%r8d, %edx
	jmp	.L10
	.p2align 4,,10
	.p2align 3
.L7:
	vmovapd	80+bodies(%rip), %xmm0
	vmovq	%rsi, %xmm5
	vfmadd213pd	56+bodies(%rip), %xmm7, %xmm0
	vfmadd231sd	40+bodies(%rip), %xmm6, %xmm5
	vfmadd231pd	24+bodies(%rip), %xmm7, %xmm8
	vmovupd	%xmm0, 56+bodies(%rip)
	vmovsd	96+bodies(%rip), %xmm0
	vfmadd213sd	72+bodies(%rip), %xmm6, %xmm0
	vmovapd	%xmm8, (%rdi)
	vmovq	%xmm5, %rsi
	vmovsd	%xmm5, 16+bodies(%rip)
	vmovsd	%xmm0, 72+bodies(%rip)
	vmovupd	136+bodies(%rip), %xmm0
	vfmadd213pd	112+bodies(%rip), %xmm7, %xmm0
	vmovapd	%xmm0, 112+bodies(%rip)
	vmovsd	152+bodies(%rip), %xmm0
	vfmadd213sd	128+bodies(%rip), %xmm6, %xmm0
	vmovsd	%xmm0, 128+bodies(%rip)
	vmovapd	192+bodies(%rip), %xmm0
	vfmadd213pd	168+bodies(%rip), %xmm7, %xmm0
	vmovupd	%xmm0, 168+bodies(%rip)
	vmovsd	208+bodies(%rip), %xmm0
	vfmadd213sd	184+bodies(%rip), %xmm6, %xmm0
	vmovsd	%xmm0, 184+bodies(%rip)
	vmovupd	248+bodies(%rip), %xmm0
	vfmadd213pd	224+bodies(%rip), %xmm7, %xmm0
	vmovapd	%xmm0, 224+bodies(%rip)
	vmovsd	264+bodies(%rip), %xmm0
	vfmadd213sd	240+bodies(%rip), %xmm6, %xmm0
	vmovsd	%xmm0, 240+bodies(%rip)
	subl	$1, %ecx
	jne	.L4
	call	energy
	movq	%rbx, %rdi
	movl	$1, %eax
	call	printf@PLT
	xorl	%eax, %eax
	popq	%rbx
	.cfi_def_cfa_offset 8
	ret
	.cfi_endproc
.LFE13:
	.size	main, .-main
	.data
	.align 32
	.type	bodies, @object
	.size	bodies, 280
bodies:
	.long	0
	.long	0
	.long	0
	.long	0
	.long	0
	.long	0
	.long	0
	.long	0
	.long	0
	.long	0
	.long	0
	.long	0
	.long	-910277154
	.long	1078181180
	.long	876402988
	.long	1075010976
	.long	-1071654020
	.long	-1074622293
	.long	1814424560
	.long	-1078294791
	.long	-1684812612
	.long	1071867654
	.long	-176319333
	.long	1074167538
	.long	-1705375979
	.long	-1080438057
	.long	-643091496
	.long	1067666581
	.long	-1020081561
	.long	1075883981
	.long	836633008
	.long	1074823115
	.long	-504674692
	.long	-1076243629
	.long	-1199074238
	.long	-1074779103
	.long	-1088450797
	.long	1073559017
	.long	1594958772
	.long	1065434184
	.long	218613303
	.long	1065819465
	.long	-827860529
	.long	1076480490
	.long	-702126466
	.long	-1070712600
	.long	-1104839264
	.long	-1077111465
	.long	-1450107921
	.long	1072780060
	.long	1045740485
	.long	1072417919
	.long	-84787588
	.long	-1081725077
	.long	-1661722957
	.long	1063009746
	.long	-1459267798
	.long	1076806247
	.long	868786720
	.long	-1069946024
	.long	-1817451200
	.long	1070002675
	.long	374979658
	.long	1072649398
	.long	834993059
	.long	1071843270
	.long	1484154358
	.long	-1079915640
	.long	1394055596
	.long	1063299315
	.section	.rodata.cst8,"aM",@progbits,8
	.align 8
.LC0:
	.long	0
	.long	1071644672
	.align 8
.LC2:
	.long	-910277154
	.long	1078181180
	.align 8
.LC5:
	.long	1202590843
	.long	1065646817
	.ident	"GCC: (Debian 14.2.0-19) 14.2.0"
	.section	.note.GNU-stack,"",@progbits
