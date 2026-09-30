	.file	"ring_fifo.c"
	.text
	.section	.text.startup,"ax",@progbits
	.p2align 4
	.globl	main
	.type	main, @function
main:
.LFB0:
	.cfi_startproc
	movl	$20000, %edx
	movl	$1, %eax
	xorl	%ecx, %ecx
	.p2align 5
	.p2align 4
	.p2align 3
.L2:
	imull	$1664525, %eax, %eax
	addl	$1013904223, %eax
	addl	%eax, %ecx
	subl	$1, %edx
	jne	.L2
	xorl	%eax, %eax
	cmpl	$1920339856, %ecx
	setne	%al
	addl	%eax, %eax
	ret
	.cfi_endproc
.LFE0:
	.size	main, .-main
	.ident	"GCC: (Debian 14.2.0-19) 14.2.0"
	.section	.note.GNU-stack,"",@progbits
