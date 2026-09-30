	.file	"k_matchlen.c"
	.text
	.p2align 4
	.globl	bench_setup
	.type	bench_setup, @function
bench_setup:
.LFB0:
	.cfi_startproc
	vmovdqa	.LC0(%rip), %ymm7
	vmovdqa	.LC1(%rip), %ymm6
	vmovdqa	.LC2(%rip), %ymm5
	vmovdqa	.LC3(%rip), %ymm4
	vmovdqa	.LC5(%rip), %ymm2
	vmovdqa	.LC6(%rip), %ymm1
	vmovdqa	%ymm7, a(%rip)
	vmovdqa	.LC7(%rip), %ymm0
	vmovdqa	.LC4(%rip), %ymm3
	vmovdqa	%ymm7, b(%rip)
	vmovdqa	%ymm6, 32+a(%rip)
	vmovdqa	%ymm6, 32+b(%rip)
	vmovdqa	%ymm5, 64+a(%rip)
	vmovdqa	%ymm5, 64+b(%rip)
	vmovdqa	%ymm4, 96+a(%rip)
	vmovdqa	%ymm4, 96+b(%rip)
	vmovdqa	%ymm3, 128+a(%rip)
	vmovdqa	%ymm3, 128+b(%rip)
	vmovdqa	%ymm2, 160+a(%rip)
	vmovdqa	%ymm2, 160+b(%rip)
	vmovdqa	%ymm1, 192+a(%rip)
	vmovdqa	%ymm1, 192+b(%rip)
	vmovdqa	%ymm0, 224+a(%rip)
	vmovdqa	%ymm0, 224+b(%rip)
	vmovdqa	%ymm7, 256+a(%rip)
	vmovdqa	%ymm7, 256+b(%rip)
	vmovdqa	%ymm6, 288+a(%rip)
	vmovdqa	%ymm6, 288+b(%rip)
	vmovdqa	%ymm5, 320+a(%rip)
	vmovdqa	%ymm5, 320+b(%rip)
	vmovdqa	%ymm4, 352+a(%rip)
	vmovdqa	%ymm4, 352+b(%rip)
	vmovdqa	%ymm3, 384+a(%rip)
	vmovdqa	%ymm0, 480+b(%rip)
	vmovdqa	%ymm3, 384+b(%rip)
	vmovdqa	%ymm2, 416+a(%rip)
	vmovdqa	%ymm2, 416+b(%rip)
	vmovdqa	%ymm1, 448+a(%rip)
	vmovdqa	%ymm1, 448+b(%rip)
	vmovdqa	%ymm0, 480+a(%rip)
	movb	$50, 509+b(%rip)
	vzeroupper
	ret
	.cfi_endproc
.LFE0:
	.size	bench_setup, .-bench_setup
	.p2align 4
	.globl	bench_run
	.type	bench_run, @function
bench_run:
.LFB2:
	.cfi_startproc
	leaq	a(%rip), %rdi
	leaq	b(%rip), %rsi
	movl	$512, %ecx
	xorl	%r8d, %r8d
	.p2align 4
	.p2align 3
.L7:
	xorl	%eax, %eax
	jmp	.L4
	.p2align 5
	.p2align 4,,10
	.p2align 3
.L8:
	movq	%rdx, %rax
.L4:
	movzbl	(%rsi,%rax), %edx
	cmpb	%dl, (%rdi,%rax)
	jne	.L5
	leaq	1(%rax), %rdx
	cmpq	%rcx, %rdx
	jne	.L8
	addl	$1, %eax
	cltq
.L5:
	subq	$1, %rcx
	addq	%rax, %r8
	addq	$1, %rdi
	addq	$1, %rsi
	cmpq	$448, %rcx
	jne	.L7
	movq	%r8, %rax
	ret
	.cfi_endproc
.LFE2:
	.size	bench_run, .-bench_run
	.local	b
	.comm	b,512,32
	.local	a
	.comm	a,512,32
	.section	.rodata.cst32,"aM",@progbits,32
	.align 32
.LC0:
	.byte	0
	.byte	17
	.byte	34
	.byte	51
	.byte	68
	.byte	85
	.byte	102
	.byte	119
	.byte	-120
	.byte	-103
	.byte	-86
	.byte	-69
	.byte	-52
	.byte	-35
	.byte	-18
	.byte	-1
	.byte	16
	.byte	33
	.byte	50
	.byte	67
	.byte	84
	.byte	101
	.byte	118
	.byte	-121
	.byte	-104
	.byte	-87
	.byte	-70
	.byte	-53
	.byte	-36
	.byte	-19
	.byte	-2
	.byte	15
	.align 32
.LC1:
	.byte	32
	.byte	49
	.byte	66
	.byte	83
	.byte	100
	.byte	117
	.byte	-122
	.byte	-105
	.byte	-88
	.byte	-71
	.byte	-54
	.byte	-37
	.byte	-20
	.byte	-3
	.byte	14
	.byte	31
	.byte	48
	.byte	65
	.byte	82
	.byte	99
	.byte	116
	.byte	-123
	.byte	-106
	.byte	-89
	.byte	-72
	.byte	-55
	.byte	-38
	.byte	-21
	.byte	-4
	.byte	13
	.byte	30
	.byte	47
	.align 32
.LC2:
	.byte	64
	.byte	81
	.byte	98
	.byte	115
	.byte	-124
	.byte	-107
	.byte	-90
	.byte	-73
	.byte	-56
	.byte	-39
	.byte	-22
	.byte	-5
	.byte	12
	.byte	29
	.byte	46
	.byte	63
	.byte	80
	.byte	97
	.byte	114
	.byte	-125
	.byte	-108
	.byte	-91
	.byte	-74
	.byte	-57
	.byte	-40
	.byte	-23
	.byte	-6
	.byte	11
	.byte	28
	.byte	45
	.byte	62
	.byte	79
	.align 32
.LC3:
	.byte	96
	.byte	113
	.byte	-126
	.byte	-109
	.byte	-92
	.byte	-75
	.byte	-58
	.byte	-41
	.byte	-24
	.byte	-7
	.byte	10
	.byte	27
	.byte	44
	.byte	61
	.byte	78
	.byte	95
	.byte	112
	.byte	-127
	.byte	-110
	.byte	-93
	.byte	-76
	.byte	-59
	.byte	-42
	.byte	-25
	.byte	-8
	.byte	9
	.byte	26
	.byte	43
	.byte	60
	.byte	77
	.byte	94
	.byte	111
	.align 32
.LC4:
	.byte	-128
	.byte	-111
	.byte	-94
	.byte	-77
	.byte	-60
	.byte	-43
	.byte	-26
	.byte	-9
	.byte	8
	.byte	25
	.byte	42
	.byte	59
	.byte	76
	.byte	93
	.byte	110
	.byte	127
	.byte	-112
	.byte	-95
	.byte	-78
	.byte	-61
	.byte	-44
	.byte	-27
	.byte	-10
	.byte	7
	.byte	24
	.byte	41
	.byte	58
	.byte	75
	.byte	92
	.byte	109
	.byte	126
	.byte	-113
	.align 32
.LC5:
	.byte	-96
	.byte	-79
	.byte	-62
	.byte	-45
	.byte	-28
	.byte	-11
	.byte	6
	.byte	23
	.byte	40
	.byte	57
	.byte	74
	.byte	91
	.byte	108
	.byte	125
	.byte	-114
	.byte	-97
	.byte	-80
	.byte	-63
	.byte	-46
	.byte	-29
	.byte	-12
	.byte	5
	.byte	22
	.byte	39
	.byte	56
	.byte	73
	.byte	90
	.byte	107
	.byte	124
	.byte	-115
	.byte	-98
	.byte	-81
	.align 32
.LC6:
	.byte	-64
	.byte	-47
	.byte	-30
	.byte	-13
	.byte	4
	.byte	21
	.byte	38
	.byte	55
	.byte	72
	.byte	89
	.byte	106
	.byte	123
	.byte	-116
	.byte	-99
	.byte	-82
	.byte	-65
	.byte	-48
	.byte	-31
	.byte	-14
	.byte	3
	.byte	20
	.byte	37
	.byte	54
	.byte	71
	.byte	88
	.byte	105
	.byte	122
	.byte	-117
	.byte	-100
	.byte	-83
	.byte	-66
	.byte	-49
	.align 32
.LC7:
	.byte	-32
	.byte	-15
	.byte	2
	.byte	19
	.byte	36
	.byte	53
	.byte	70
	.byte	87
	.byte	104
	.byte	121
	.byte	-118
	.byte	-101
	.byte	-84
	.byte	-67
	.byte	-50
	.byte	-33
	.byte	-16
	.byte	1
	.byte	18
	.byte	35
	.byte	52
	.byte	69
	.byte	86
	.byte	103
	.byte	120
	.byte	-119
	.byte	-102
	.byte	-85
	.byte	-68
	.byte	-51
	.byte	-34
	.byte	-17
	.ident	"GCC: (Debian 14.2.0-19) 14.2.0"
	.section	.note.GNU-stack,"",@progbits
