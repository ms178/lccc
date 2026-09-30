	.file	"binary_search.c"
	.text
	.section	.text.startup,"ax",@progbits
	.p2align 4
	.globl	main
	.type	main, @function
main:
.LFB2:
	.cfi_startproc
	movl	$8, %edx
	leaq	table(%rip), %r8
	vmovdqa	.LC0(%rip), %ymm1
	vmovd	%edx, %xmm4
	movl	$7, %edx
	movq	%r8, %rax
	vmovd	%edx, %xmm3
	leaq	16384(%r8), %rcx
	vpbroadcastd	%xmm4, %ymm4
	vpbroadcastd	%xmm3, %ymm3
	.p2align 6
	.p2align 4
	.p2align 3
.L2:
	vmovdqa	%ymm1, %ymm2
	addq	$32, %rax
	vpaddd	%ymm4, %ymm1, %ymm1
	vpslld	$1, %ymm2, %ymm0
	vpaddd	%ymm2, %ymm0, %ymm0
	vpaddd	%ymm3, %ymm0, %ymm0
	vmovdqa	%ymm0, -32(%rax)
	cmpq	%rcx, %rax
	jne	.L2
	movl	$-1000, %edi
	xorl	%r9d, %r9d
	.p2align 4
	.p2align 3
.L3:
	xorl	%edx, %edx
	movl	$4095, %ecx
	.p2align 4
	.p2align 3
.L9:
	movl	%ecx, %eax
	subl	%edx, %eax
	sarl	%eax
	addl	%edx, %eax
	movslq	%eax, %rsi
	cmpl	%edi, (%r8,%rsi,4)
	je	.L4
	jge	.L5
	leal	1(%rax), %edx
	cmpl	%ecx, %edx
	jle	.L9
.L7:
	addl	$7, %edi
	cmpl	$13294, %edi
	jne	.L3
.L8:
	xorl	%eax, %eax
	cmpl	$1198665, %r9d
	setne	%al
	addl	%eax, %eax
	vzeroupper
	ret
	.p2align 4,,10
	.p2align 3
.L5:
	leal	-1(%rax), %ecx
	cmpl	%edx, %ecx
	jge	.L9
	jmp	.L7
	.p2align 4,,10
	.p2align 3
.L4:
	testl	%eax, %eax
	js	.L7
	addl	$7, %edi
	addl	%eax, %r9d
	cmpl	$13294, %edi
	jne	.L3
	jmp	.L8
	.cfi_endproc
.LFE2:
	.size	main, .-main
	.local	table
	.comm	table,16384,32
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
