	.file	"prefix_scan.c"
	.text
	.section	.rodata.str1.1,"aMS",@progbits,1
.LC5:
	.string	"%llx\n"
	.section	.text.startup,"ax",@progbits
	.p2align 4
	.globl	main
	.type	main, @function
main:
.LFB12:
	.cfi_startproc
	movl	$8, %edx
	pushq	%rbp
	.cfi_def_cfa_offset 16
	.cfi_offset 6, -16
	leaq	a.1(%rip), %rsi
	vmovdqa	.LC0(%rip), %ymm2
	vmovd	%edx, %xmm6
	movl	$16, %edx
	movq	%rsi, %rax
	vmovd	%edx, %xmm5
	movl	$65535, %edx
	leaq	2048(%rsi), %rcx
	vmovd	%edx, %xmm3
	movl	$458759, %edx
	movq	%rsp, %rbp
	.cfi_def_cfa_register 6
	andq	$-32, %rsp
	vmovd	%edx, %xmm4
	vpbroadcastd	%xmm6, %ymm6
	vpbroadcastd	%xmm5, %ymm5
	vpbroadcastd	%xmm3, %ymm3
	vpbroadcastd	%xmm4, %ymm4
	.p2align 6
	.p2align 4
	.p2align 3
.L2:
	vpaddd	%ymm6, %ymm2, %ymm1
	vmovdqa	%ymm2, %ymm0
	vpaddd	%ymm5, %ymm2, %ymm2
	addq	$32, %rax
	vpand	%ymm1, %ymm3, %ymm1
	vpand	%ymm0, %ymm3, %ymm0
	vpackusdw	%ymm1, %ymm0, %ymm0
	vpermq	$216, %ymm0, %ymm0
	vpsllw	$5, %ymm0, %ymm1
	vpsubw	%ymm0, %ymm1, %ymm0
	vpaddw	%ymm4, %ymm0, %ymm0
	vmovdqa	%ymm0, -32(%rax)
	cmpq	%rcx, %rax
	jne	.L2
	xorl	%eax, %eax
	xorl	%edx, %edx
	leaq	d.0(%rip), %rdi
	.p2align 5
	.p2align 4
	.p2align 3
.L3:
	movzwl	(%rsi,%rax,2), %ecx
	addl	%ecx, %edx
	movl	%edx, (%rdi,%rax,4)
	addq	$1, %rax
	cmpq	$1024, %rax
	jne	.L3
	leaq	d.0(%rip), %rax
	movl	2044+d.0(%rip), %esi
	addl	d.0(%rip), %esi
	leaq	4116(%rax), %rcx
	addl	4092+d.0(%rip), %esi
	.p2align 5
	.p2align 4
	.p2align 3
.L4:
	movq	%rsi, %rdx
	addq	$28, %rax
	salq	$5, %rdx
	addq	%rdx, %rsi
	movl	-28(%rax), %edx
	addq	%rdx, %rsi
	cmpq	%rcx, %rax
	jne	.L4
	xorl	%eax, %eax
	leaq	.LC5(%rip), %rdi
	vzeroupper
	call	printf@PLT
	xorl	%eax, %eax
	leave
	.cfi_def_cfa 7, 8
	ret
	.cfi_endproc
.LFE12:
	.size	main, .-main
	.local	d.0
	.comm	d.0,4096,32
	.local	a.1
	.comm	a.1,2048,32
	.section	.rodata.cst32,"aM",@progbits,32
	.align 32
.LC0:
	.long	0
	.long	1
	.long	2
	.long	3
	.long	4
	.long	5
	.long	6
	.long	7
	.ident	"GCC: (Debian 14.2.0-19) 14.2.0"
	.section	.note.GNU-stack,"",@progbits
