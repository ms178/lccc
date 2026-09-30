	.file	"global_addr_pressure.c"
	.text
	.p2align 4
	.type	mix.constprop.0, @function
mix.constprop.0:
.LFB25:
	.cfi_startproc
	xorl	%eax, %eax
	xorl	%ecx, %ecx
	leaq	count(%rip), %rsi
	leaq	perm(%rip), %r8
	leaq	perm1(%rip), %rdi
	.p2align 5
	.p2align 4
	.p2align 3
.L2:
	movl	(%r8,%rax), %edx
	addl	(%rsi,%rax), %edx
	addl	(%rdi,%rax), %edx
	addl	%edx, %ecx
	movl	%ecx, (%rsi,%rax)
	addq	$4, %rax
	cmpq	$512, %rax
	jne	.L2
	movl	%ecx, %eax
	ret
	.cfi_endproc
.LFE25:
	.size	mix.constprop.0, .-mix.constprop.0
	.p2align 4
	.type	mix.constprop.1, @function
mix.constprop.1:
.LFB26:
	.cfi_startproc
	xorl	%eax, %eax
	xorl	%ecx, %ecx
	leaq	perm1(%rip), %rsi
	leaq	count(%rip), %r8
	leaq	perm(%rip), %rdi
	.p2align 5
	.p2align 4
	.p2align 3
.L6:
	movl	(%r8,%rax), %edx
	addl	(%rsi,%rax), %edx
	addl	(%rdi,%rax), %edx
	addl	%edx, %ecx
	movl	%ecx, (%rsi,%rax)
	addq	$4, %rax
	cmpq	$512, %rax
	jne	.L6
	movl	%ecx, %eax
	ret
	.cfi_endproc
.LFE26:
	.size	mix.constprop.1, .-mix.constprop.1
	.p2align 4
	.type	mix.constprop.2, @function
mix.constprop.2:
.LFB27:
	.cfi_startproc
	xorl	%eax, %eax
	xorl	%ecx, %ecx
	leaq	perm(%rip), %rsi
	leaq	perm1(%rip), %r8
	leaq	count(%rip), %rdi
	.p2align 5
	.p2align 4
	.p2align 3
.L9:
	movl	(%r8,%rax), %edx
	addl	(%rsi,%rax), %edx
	addl	(%rdi,%rax), %edx
	addl	%edx, %ecx
	movl	%ecx, (%rsi,%rax)
	addq	$4, %rax
	cmpq	$512, %rax
	jne	.L9
	movl	%ecx, %eax
	ret
	.cfi_endproc
.LFE27:
	.size	mix.constprop.2, .-mix.constprop.2
	.p2align 4
	.globl	kernel
	.type	kernel, @function
kernel:
.LFB23:
	.cfi_startproc
	movl	%edi, %r11d
	testl	%edi, %edi
	jle	.L14
	xorl	%r10d, %r10d
	xorl	%r9d, %r9d
	.p2align 4
	.p2align 3
.L13:
	call	mix.constprop.2
	addl	$1, %r10d
	addl	%eax, %r9d
	call	mix.constprop.1
	addl	%eax, %r9d
	call	mix.constprop.0
	addl	%eax, %r9d
	cmpl	%r10d, %r11d
	jne	.L13
	movl	%r9d, %eax
	ret
	.p2align 4,,10
	.p2align 3
.L14:
	xorl	%r9d, %r9d
	movl	%r9d, %eax
	ret
	.cfi_endproc
.LFE23:
	.size	kernel, .-kernel
	.section	.rodata.str1.1,"aMS",@progbits,1
.LC48:
	.string	"%d\n"
	.section	.text.startup,"ax",@progbits
	.p2align 4
	.globl	main
	.type	main, @function
main:
.LFB24:
	.cfi_startproc
	pushq	%rbp
	.cfi_def_cfa_offset 16
	.cfi_offset 6, -16
	movl	%edi, %eax
	movl	$2000, %edi
	movq	%rsp, %rbp
	.cfi_def_cfa_register 6
	cmpl	$1, %eax
	jle	.L17
	movq	8(%rsi), %rdi
	movl	$10, %edx
	xorl	%esi, %esi
	call	strtol@PLT
	movl	%eax, %edi
.L17:
	vmovdqa	.LC0(%rip), %ymm0
	vmovdqa	%ymm0, perm(%rip)
	vmovdqa	.LC1(%rip), %ymm0
	vmovdqa	%ymm0, perm1(%rip)
	vmovdqa	.LC2(%rip), %ymm0
	vmovdqa	%ymm0, count(%rip)
	vmovdqa	.LC3(%rip), %ymm0
	vmovdqa	%ymm0, 32+perm(%rip)
	vmovdqa	.LC4(%rip), %ymm0
	vmovdqa	%ymm0, 32+perm1(%rip)
	vmovdqa	.LC5(%rip), %ymm0
	vmovdqa	%ymm0, 32+count(%rip)
	vmovdqa	.LC6(%rip), %ymm0
	vmovdqa	%ymm0, 64+perm(%rip)
	vmovdqa	.LC7(%rip), %ymm0
	vmovdqa	%ymm0, 64+perm1(%rip)
	vmovdqa	.LC8(%rip), %ymm0
	vmovdqa	%ymm0, 64+count(%rip)
	vmovdqa	.LC9(%rip), %ymm0
	vmovdqa	%ymm0, 96+perm(%rip)
	vmovdqa	.LC10(%rip), %ymm0
	vmovdqa	%ymm0, 96+perm1(%rip)
	vmovdqa	.LC11(%rip), %ymm0
	vmovdqa	%ymm0, 96+count(%rip)
	vmovdqa	.LC12(%rip), %ymm0
	vmovdqa	%ymm0, 128+perm(%rip)
	vmovdqa	.LC13(%rip), %ymm0
	vmovdqa	%ymm0, 128+perm1(%rip)
	vmovdqa	.LC14(%rip), %ymm0
	vmovdqa	%ymm0, 128+count(%rip)
	vmovdqa	.LC15(%rip), %ymm0
	vmovdqa	%ymm0, 160+perm(%rip)
	vmovdqa	.LC16(%rip), %ymm0
	vmovdqa	%ymm0, 160+perm1(%rip)
	vmovdqa	.LC17(%rip), %ymm0
	vmovdqa	%ymm0, 160+count(%rip)
	vmovdqa	.LC18(%rip), %ymm0
	vmovdqa	%ymm0, 192+perm(%rip)
	vmovdqa	.LC19(%rip), %ymm0
	vmovdqa	%ymm0, 192+perm1(%rip)
	vmovdqa	.LC20(%rip), %ymm0
	vmovdqa	%ymm0, 192+count(%rip)
	vmovdqa	.LC21(%rip), %ymm0
	vmovdqa	%ymm0, 224+perm(%rip)
	vmovdqa	.LC22(%rip), %ymm0
	vmovdqa	%ymm0, 224+perm1(%rip)
	vmovdqa	.LC23(%rip), %ymm0
	vmovdqa	%ymm0, 224+count(%rip)
	vmovdqa	.LC24(%rip), %ymm0
	vmovdqa	%ymm0, 256+perm(%rip)
	vmovdqa	.LC25(%rip), %ymm0
	vmovdqa	%ymm0, 256+perm1(%rip)
	vmovdqa	.LC26(%rip), %ymm0
	vmovdqa	%ymm0, 256+count(%rip)
	vmovdqa	.LC27(%rip), %ymm0
	vmovdqa	%ymm0, 288+perm(%rip)
	vmovdqa	.LC28(%rip), %ymm0
	vmovdqa	%ymm0, 288+perm1(%rip)
	vmovdqa	.LC29(%rip), %ymm0
	vmovdqa	%ymm0, 288+count(%rip)
	vmovdqa	.LC30(%rip), %ymm0
	vmovdqa	%ymm0, 320+perm(%rip)
	vmovdqa	.LC31(%rip), %ymm0
	vmovdqa	%ymm0, 320+perm1(%rip)
	vmovdqa	.LC32(%rip), %ymm0
	vmovdqa	%ymm0, 320+count(%rip)
	vmovdqa	.LC33(%rip), %ymm0
	vmovdqa	%ymm0, 352+perm(%rip)
	vmovdqa	.LC34(%rip), %ymm0
	vmovdqa	%ymm0, 352+perm1(%rip)
	vmovdqa	.LC35(%rip), %ymm0
	vmovdqa	%ymm0, 352+count(%rip)
	vmovdqa	.LC36(%rip), %ymm0
	vmovdqa	%ymm0, 384+perm(%rip)
	vmovdqa	.LC37(%rip), %ymm0
	vmovdqa	%ymm0, 384+perm1(%rip)
	vmovdqa	.LC38(%rip), %ymm0
	vmovdqa	%ymm0, 384+count(%rip)
	vmovdqa	.LC39(%rip), %ymm0
	vmovdqa	%ymm0, 416+perm(%rip)
	vmovdqa	.LC40(%rip), %ymm0
	vmovdqa	%ymm0, 416+perm1(%rip)
	vmovdqa	.LC41(%rip), %ymm0
	vmovdqa	%ymm0, 416+count(%rip)
	vmovdqa	.LC42(%rip), %ymm0
	vmovdqa	%ymm0, 448+perm(%rip)
	vmovdqa	.LC43(%rip), %ymm0
	vmovdqa	%ymm0, 448+perm1(%rip)
	vmovdqa	.LC44(%rip), %ymm0
	vmovdqa	%ymm0, 448+count(%rip)
	vmovdqa	.LC45(%rip), %ymm0
	vmovdqa	%ymm0, 480+perm(%rip)
	vmovdqa	.LC46(%rip), %ymm0
	vmovdqa	%ymm0, 480+perm1(%rip)
	vmovdqa	.LC47(%rip), %ymm0
	vmovdqa	%ymm0, 480+count(%rip)
	call	kernel
	leaq	.LC48(%rip), %rdi
	movl	%eax, %esi
	xorl	%eax, %eax
	vzeroupper
	call	printf@PLT
	xorl	%eax, %eax
	popq	%rbp
	.cfi_def_cfa 7, 8
	ret
	.cfi_endproc
.LFE24:
	.size	main, .-main
	.local	count
	.comm	count,512,32
	.local	perm1
	.comm	perm1,512,32
	.local	perm
	.comm	perm,512,32
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
	.align 32
.LC1:
	.long	128
	.long	127
	.long	126
	.long	125
	.long	124
	.long	123
	.long	122
	.long	121
	.align 32
.LC2:
	.long	0
	.long	1
	.long	4
	.long	9
	.long	16
	.long	25
	.long	36
	.long	49
	.align 32
.LC3:
	.long	8
	.long	9
	.long	10
	.long	11
	.long	12
	.long	13
	.long	14
	.long	15
	.align 32
.LC4:
	.long	120
	.long	119
	.long	118
	.long	117
	.long	116
	.long	115
	.long	114
	.long	113
	.align 32
.LC5:
	.long	64
	.long	81
	.long	100
	.long	121
	.long	144
	.long	169
	.long	196
	.long	225
	.align 32
.LC6:
	.long	16
	.long	17
	.long	18
	.long	19
	.long	20
	.long	21
	.long	22
	.long	23
	.align 32
.LC7:
	.long	112
	.long	111
	.long	110
	.long	109
	.long	108
	.long	107
	.long	106
	.long	105
	.align 32
.LC8:
	.long	256
	.long	289
	.long	324
	.long	361
	.long	400
	.long	441
	.long	484
	.long	529
	.align 32
.LC9:
	.long	24
	.long	25
	.long	26
	.long	27
	.long	28
	.long	29
	.long	30
	.long	31
	.align 32
.LC10:
	.long	104
	.long	103
	.long	102
	.long	101
	.long	100
	.long	99
	.long	98
	.long	97
	.align 32
.LC11:
	.long	576
	.long	625
	.long	676
	.long	729
	.long	784
	.long	841
	.long	900
	.long	961
	.align 32
.LC12:
	.long	32
	.long	33
	.long	34
	.long	35
	.long	36
	.long	37
	.long	38
	.long	39
	.align 32
.LC13:
	.long	96
	.long	95
	.long	94
	.long	93
	.long	92
	.long	91
	.long	90
	.long	89
	.align 32
.LC14:
	.long	1024
	.long	1089
	.long	1156
	.long	1225
	.long	1296
	.long	1369
	.long	1444
	.long	1521
	.align 32
.LC15:
	.long	40
	.long	41
	.long	42
	.long	43
	.long	44
	.long	45
	.long	46
	.long	47
	.align 32
.LC16:
	.long	88
	.long	87
	.long	86
	.long	85
	.long	84
	.long	83
	.long	82
	.long	81
	.align 32
.LC17:
	.long	1600
	.long	1681
	.long	1764
	.long	1849
	.long	1936
	.long	2025
	.long	2116
	.long	2209
	.align 32
.LC18:
	.long	48
	.long	49
	.long	50
	.long	51
	.long	52
	.long	53
	.long	54
	.long	55
	.align 32
.LC19:
	.long	80
	.long	79
	.long	78
	.long	77
	.long	76
	.long	75
	.long	74
	.long	73
	.align 32
.LC20:
	.long	2304
	.long	2401
	.long	2500
	.long	2601
	.long	2704
	.long	2809
	.long	2916
	.long	3025
	.align 32
.LC21:
	.long	56
	.long	57
	.long	58
	.long	59
	.long	60
	.long	61
	.long	62
	.long	63
	.align 32
.LC22:
	.long	72
	.long	71
	.long	70
	.long	69
	.long	68
	.long	67
	.long	66
	.long	65
	.align 32
.LC23:
	.long	3136
	.long	3249
	.long	3364
	.long	3481
	.long	3600
	.long	3721
	.long	3844
	.long	3969
	.align 32
.LC24:
	.long	64
	.long	65
	.long	66
	.long	67
	.long	68
	.long	69
	.long	70
	.long	71
	.align 32
.LC25:
	.long	64
	.long	63
	.long	62
	.long	61
	.long	60
	.long	59
	.long	58
	.long	57
	.align 32
.LC26:
	.long	4096
	.long	4225
	.long	4356
	.long	4489
	.long	4624
	.long	4761
	.long	4900
	.long	5041
	.align 32
.LC27:
	.long	72
	.long	73
	.long	74
	.long	75
	.long	76
	.long	77
	.long	78
	.long	79
	.align 32
.LC28:
	.long	56
	.long	55
	.long	54
	.long	53
	.long	52
	.long	51
	.long	50
	.long	49
	.align 32
.LC29:
	.long	5184
	.long	5329
	.long	5476
	.long	5625
	.long	5776
	.long	5929
	.long	6084
	.long	6241
	.align 32
.LC30:
	.long	80
	.long	81
	.long	82
	.long	83
	.long	84
	.long	85
	.long	86
	.long	87
	.align 32
.LC31:
	.long	48
	.long	47
	.long	46
	.long	45
	.long	44
	.long	43
	.long	42
	.long	41
	.align 32
.LC32:
	.long	6400
	.long	6561
	.long	6724
	.long	6889
	.long	7056
	.long	7225
	.long	7396
	.long	7569
	.align 32
.LC33:
	.long	88
	.long	89
	.long	90
	.long	91
	.long	92
	.long	93
	.long	94
	.long	95
	.align 32
.LC34:
	.long	40
	.long	39
	.long	38
	.long	37
	.long	36
	.long	35
	.long	34
	.long	33
	.align 32
.LC35:
	.long	7744
	.long	7921
	.long	8100
	.long	8281
	.long	8464
	.long	8649
	.long	8836
	.long	9025
	.align 32
.LC36:
	.long	96
	.long	97
	.long	98
	.long	99
	.long	100
	.long	101
	.long	102
	.long	103
	.align 32
.LC37:
	.long	32
	.long	31
	.long	30
	.long	29
	.long	28
	.long	27
	.long	26
	.long	25
	.align 32
.LC38:
	.long	9216
	.long	9409
	.long	9604
	.long	9801
	.long	10000
	.long	10201
	.long	10404
	.long	10609
	.align 32
.LC39:
	.long	104
	.long	105
	.long	106
	.long	107
	.long	108
	.long	109
	.long	110
	.long	111
	.align 32
.LC40:
	.long	24
	.long	23
	.long	22
	.long	21
	.long	20
	.long	19
	.long	18
	.long	17
	.align 32
.LC41:
	.long	10816
	.long	11025
	.long	11236
	.long	11449
	.long	11664
	.long	11881
	.long	12100
	.long	12321
	.align 32
.LC42:
	.long	112
	.long	113
	.long	114
	.long	115
	.long	116
	.long	117
	.long	118
	.long	119
	.align 32
.LC43:
	.long	16
	.long	15
	.long	14
	.long	13
	.long	12
	.long	11
	.long	10
	.long	9
	.align 32
.LC44:
	.long	12544
	.long	12769
	.long	12996
	.long	13225
	.long	13456
	.long	13689
	.long	13924
	.long	14161
	.align 32
.LC45:
	.long	120
	.long	121
	.long	122
	.long	123
	.long	124
	.long	125
	.long	126
	.long	127
	.align 32
.LC46:
	.long	8
	.long	7
	.long	6
	.long	5
	.long	4
	.long	3
	.long	2
	.long	1
	.align 32
.LC47:
	.long	14400
	.long	14641
	.long	14884
	.long	15129
	.long	15376
	.long	15625
	.long	15876
	.long	16129
	.ident	"GCC: (Debian 14.2.0-19) 14.2.0"
	.section	.note.GNU-stack,"",@progbits
