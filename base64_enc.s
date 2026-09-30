	.file	"base64_enc.c"
	.text
	.section	.rodata.str1.1,"aMS",@progbits,1
.LC10:
	.string	"%llx\n"
	.section	.text.startup,"ax",@progbits
	.p2align 4
	.globl	main
	.type	main, @function
main:
.LFB12:
	.cfi_startproc
	pushq	%rbp
	.cfi_def_cfa_offset 16
	.cfi_offset 6, -16
	leaq	a.1(%rip), %rcx
	leaq	d.0(%rip), %rdi
	movl	$300, %r8d
	leaq	300(%rcx), %r10
	movq	%rdi, %rsi
	leaq	tab(%rip), %r9
	movq	%rsp, %rbp
	.cfi_def_cfa_register 6
	pushq	%r15
	pushq	%r14
	pushq	%rbx
	subq	$8, %rsp
	.cfi_offset 15, -24
	.cfi_offset 14, -32
	.cfi_offset 3, -40
	vmovdqa	.LC1(%rip), %ymm1
	vmovdqa	.LC0(%rip), %ymm0
	movl	$1109260499, 296+a.1(%rip)
	movq	.LC8(%rip), %rax
	vmovdqa	%ymm1, 32+a.1(%rip)
	vmovdqa	.LC2(%rip), %ymm1
	movq	%rax, 288+a.1(%rip)
	vmovdqa	%ymm1, 64+a.1(%rip)
	vmovdqa	.LC3(%rip), %ymm1
	vmovdqa	%ymm0, a.1(%rip)
	vmovdqa	%ymm1, 96+a.1(%rip)
	vmovdqa	.LC4(%rip), %ymm1
	vmovdqa	%ymm0, 256+a.1(%rip)
	vmovdqa	%ymm1, 128+a.1(%rip)
	vmovdqa	.LC5(%rip), %ymm1
	vmovdqa	%ymm1, 160+a.1(%rip)
	vmovdqa	.LC6(%rip), %ymm1
	vmovdqa	%ymm1, 192+a.1(%rip)
	vmovdqa	.LC7(%rip), %ymm1
	vmovdqa	%ymm1, 224+a.1(%rip)
	jmp	.L4
	.p2align 4,,10
	.p2align 3
.L15:
	movzbl	1(%rcx), %eax
	sall	$8, %eax
	orl	%edx, %eax
	movl	%eax, %r14d
	cmpl	$2, %r8d
	je	.L3
	movzbl	2(%rcx), %ebx
	xorl	%edx, %edx
	orl	%eax, %ebx
	shrl	$18, %eax
	movl	%eax, %r11d
	movl	%r14d, %eax
	movl	%ebx, %r15d
	shrl	$12, %eax
	movb	(%r9,%r11), %dl
	movl	%eax, %r11d
	movq	%r9, %rax
	andl	$63, %r11d
	movq	%r11, %rbx
	movb	(%rax,%rbx), %dh
	movl	%r15d, %ebx
	shrl	$6, %ebx
	movw	%dx, (%rsi)
	movl	%ebx, %edx
	andl	$63, %edx
	movzbl	(%r9,%rdx), %eax
	movb	%al, 2(%rsi)
	movl	%r15d, %eax
	andl	$63, %eax
	movzbl	(%r9,%rax), %eax
.L6:
	addq	$3, %rcx
	movb	%al, 3(%rsi)
	subl	$3, %r8d
	addq	$4, %rsi
	cmpq	%r10, %rcx
	je	.L14
.L4:
	movzbl	(%rcx), %edx
	sall	$16, %edx
	cmpl	$1, %r8d
	jne	.L15
	movl	%edx, %r11d
	shrl	$12, %edx
	movq	%r9, %rbx
	xorl	%eax, %eax
	shrl	$18, %r11d
	andl	$48, %edx
	movb	$61, 2(%rsi)
	movb	(%r9,%r11), %al
	movb	(%rbx,%rdx), %ah
	movw	%ax, (%rsi)
.L7:
	movl	$61, %eax
	jmp	.L6
	.p2align 4,,10
	.p2align 3
.L3:
	shrl	$18, %eax
	xorl	%edx, %edx
	movq	%r9, %rbx
	movl	%eax, %r11d
	movl	%r14d, %eax
	shrl	$12, %eax
	movb	(%r9,%r11), %dl
	movl	%eax, %r11d
	andl	$63, %r11d
	movq	%r11, %rax
	movb	(%rbx,%rax), %dh
	movl	%r14d, %eax
	shrl	$6, %eax
	andl	$60, %eax
	movw	%dx, (%rsi)
	movzbl	(%r9,%rax), %eax
	movb	%al, 2(%rsi)
	jmp	.L7
.L14:
	leaq	400+d.0(%rip), %rdx
	movl	$400, %esi
	.p2align 5
	.p2align 4
	.p2align 3
.L5:
	movq	%rsi, %rax
	addq	$1, %rdi
	salq	$5, %rax
	addq	%rax, %rsi
	movzbl	-1(%rdi), %eax
	addq	%rax, %rsi
	cmpq	%rdx, %rdi
	jne	.L5
	xorl	%eax, %eax
	leaq	.LC10(%rip), %rdi
	vzeroupper
	call	printf@PLT
	addq	$8, %rsp
	xorl	%eax, %eax
	popq	%rbx
	popq	%r14
	popq	%r15
	popq	%rbp
	.cfi_def_cfa 7, 8
	ret
	.cfi_endproc
.LFE12:
	.size	main, .-main
	.local	d.0
	.comm	d.0,404,32
	.local	a.1
	.comm	a.1,300,32
	.section	.rodata
	.align 32
	.type	tab, @object
	.size	tab, 64
tab:
	.ascii	"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz01234567"
	.ascii	"89+/"
	.section	.rodata.cst32,"aM",@progbits,32
	.align 32
.LC0:
	.byte	11
	.byte	48
	.byte	85
	.byte	122
	.byte	-97
	.byte	-60
	.byte	-23
	.byte	14
	.byte	51
	.byte	88
	.byte	125
	.byte	-94
	.byte	-57
	.byte	-20
	.byte	17
	.byte	54
	.byte	91
	.byte	-128
	.byte	-91
	.byte	-54
	.byte	-17
	.byte	20
	.byte	57
	.byte	94
	.byte	-125
	.byte	-88
	.byte	-51
	.byte	-14
	.byte	23
	.byte	60
	.byte	97
	.byte	-122
	.align 32
.LC1:
	.byte	-85
	.byte	-48
	.byte	-11
	.byte	26
	.byte	63
	.byte	100
	.byte	-119
	.byte	-82
	.byte	-45
	.byte	-8
	.byte	29
	.byte	66
	.byte	103
	.byte	-116
	.byte	-79
	.byte	-42
	.byte	-5
	.byte	32
	.byte	69
	.byte	106
	.byte	-113
	.byte	-76
	.byte	-39
	.byte	-2
	.byte	35
	.byte	72
	.byte	109
	.byte	-110
	.byte	-73
	.byte	-36
	.byte	1
	.byte	38
	.align 32
.LC2:
	.byte	75
	.byte	112
	.byte	-107
	.byte	-70
	.byte	-33
	.byte	4
	.byte	41
	.byte	78
	.byte	115
	.byte	-104
	.byte	-67
	.byte	-30
	.byte	7
	.byte	44
	.byte	81
	.byte	118
	.byte	-101
	.byte	-64
	.byte	-27
	.byte	10
	.byte	47
	.byte	84
	.byte	121
	.byte	-98
	.byte	-61
	.byte	-24
	.byte	13
	.byte	50
	.byte	87
	.byte	124
	.byte	-95
	.byte	-58
	.align 32
.LC3:
	.byte	-21
	.byte	16
	.byte	53
	.byte	90
	.byte	127
	.byte	-92
	.byte	-55
	.byte	-18
	.byte	19
	.byte	56
	.byte	93
	.byte	-126
	.byte	-89
	.byte	-52
	.byte	-15
	.byte	22
	.byte	59
	.byte	96
	.byte	-123
	.byte	-86
	.byte	-49
	.byte	-12
	.byte	25
	.byte	62
	.byte	99
	.byte	-120
	.byte	-83
	.byte	-46
	.byte	-9
	.byte	28
	.byte	65
	.byte	102
	.align 32
.LC4:
	.byte	-117
	.byte	-80
	.byte	-43
	.byte	-6
	.byte	31
	.byte	68
	.byte	105
	.byte	-114
	.byte	-77
	.byte	-40
	.byte	-3
	.byte	34
	.byte	71
	.byte	108
	.byte	-111
	.byte	-74
	.byte	-37
	.byte	0
	.byte	37
	.byte	74
	.byte	111
	.byte	-108
	.byte	-71
	.byte	-34
	.byte	3
	.byte	40
	.byte	77
	.byte	114
	.byte	-105
	.byte	-68
	.byte	-31
	.byte	6
	.align 32
.LC5:
	.byte	43
	.byte	80
	.byte	117
	.byte	-102
	.byte	-65
	.byte	-28
	.byte	9
	.byte	46
	.byte	83
	.byte	120
	.byte	-99
	.byte	-62
	.byte	-25
	.byte	12
	.byte	49
	.byte	86
	.byte	123
	.byte	-96
	.byte	-59
	.byte	-22
	.byte	15
	.byte	52
	.byte	89
	.byte	126
	.byte	-93
	.byte	-56
	.byte	-19
	.byte	18
	.byte	55
	.byte	92
	.byte	-127
	.byte	-90
	.align 32
.LC6:
	.byte	-53
	.byte	-16
	.byte	21
	.byte	58
	.byte	95
	.byte	-124
	.byte	-87
	.byte	-50
	.byte	-13
	.byte	24
	.byte	61
	.byte	98
	.byte	-121
	.byte	-84
	.byte	-47
	.byte	-10
	.byte	27
	.byte	64
	.byte	101
	.byte	-118
	.byte	-81
	.byte	-44
	.byte	-7
	.byte	30
	.byte	67
	.byte	104
	.byte	-115
	.byte	-78
	.byte	-41
	.byte	-4
	.byte	33
	.byte	70
	.align 32
.LC7:
	.byte	107
	.byte	-112
	.byte	-75
	.byte	-38
	.byte	-1
	.byte	36
	.byte	73
	.byte	110
	.byte	-109
	.byte	-72
	.byte	-35
	.byte	2
	.byte	39
	.byte	76
	.byte	113
	.byte	-106
	.byte	-69
	.byte	-32
	.byte	5
	.byte	42
	.byte	79
	.byte	116
	.byte	-103
	.byte	-66
	.byte	-29
	.byte	8
	.byte	45
	.byte	82
	.byte	119
	.byte	-100
	.byte	-63
	.byte	-26
	.set	.LC8,.LC1
	.ident	"GCC: (Debian 14.2.0-19) 14.2.0"
	.section	.note.GNU-stack,"",@progbits
