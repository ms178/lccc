	.file	"fir_filter.c"
	.text
	.section	.rodata.str1.1,"aMS",@progbits,1
.LC24:
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
	movl	$327685, %esi
	leaq	d.0(%rip), %rcx
	leaq	14+a.2(%rip), %rax
	vmovd	%esi, %xmm3
	movl	$458759, %esi
	leaq	1024(%rcx), %r8
	movq	%rcx, %rdx
	vmovd	%esi, %xmm4
	movl	$720907, %esi
	vpbroadcastd	%xmm3, %ymm3
	movq	%rsp, %rbp
	.cfi_def_cfa_register 6
	vmovd	%esi, %xmm5
	andq	$-32, %rsp
	movl	$1114129, %esi
	subq	$256, %rsp
	vmovd	%esi, %xmm6
	movl	$1638425, %esi
	vmovdqa	.LC0(%rip), %ymm0
	vpbroadcastd	%xmm4, %ymm4
	vpbroadcastd	%xmm5, %ymm5
	vpbroadcastd	%xmm6, %ymm6
	vmovdqa	%ymm0, a.2(%rip)
	vmovdqa	.LC1(%rip), %ymm0
	vmovdqa	%ymm0, 32+a.2(%rip)
	vmovdqa	.LC2(%rip), %ymm0
	vmovdqa	%ymm0, 64+a.2(%rip)
	vmovdqa	.LC3(%rip), %ymm0
	vmovdqa	%ymm0, 96+a.2(%rip)
	vmovdqa	.LC4(%rip), %ymm0
	vmovdqa	%ymm0, 128+a.2(%rip)
	vmovdqa	.LC5(%rip), %ymm0
	vmovdqa	%ymm0, 160+a.2(%rip)
	vmovdqa	.LC6(%rip), %ymm0
	vmovdqa	%ymm0, 192+a.2(%rip)
	vmovdqa	.LC7(%rip), %ymm0
	vmovdqa	%ymm0, 224+a.2(%rip)
	vmovdqa	.LC8(%rip), %ymm0
	vmovdqa	%ymm0, 256+a.2(%rip)
	vmovdqa	.LC9(%rip), %ymm0
	vmovdqa	%ymm0, 288+a.2(%rip)
	vmovdqa	.LC10(%rip), %ymm0
	vmovdqa	%ymm0, 320+a.2(%rip)
	vmovdqa	.LC11(%rip), %ymm0
	vmovdqa	%ymm0, 352+a.2(%rip)
	vmovdqa	.LC12(%rip), %ymm0
	vmovdqa	%ymm0, 384+a.2(%rip)
	vmovdqa	.LC13(%rip), %ymm0
	vmovdqa	%ymm0, 416+a.2(%rip)
	vmovdqa	.LC14(%rip), %ymm0
	vmovdqa	%ymm3, 160(%rsp)
	vmovd	%esi, %xmm3
	movl	$2293795, %esi
	vmovdqa	%ymm0, 448+a.2(%rip)
	vmovdqa	.LC15(%rip), %ymm0
	vpbroadcastd	%xmm3, %ymm3
	vmovdqa	%ymm4, 128(%rsp)
	vmovd	%esi, %xmm4
	vmovdqa	%ymm0, 480+a.2(%rip)
	vmovdqa	.LC16(%rip), %xmm0
	vpbroadcastd	%xmm4, %ymm4
	vmovdqa	%ymm5, 96(%rsp)
	vmovdqa	%xmm0, 512+a.2(%rip)
	vmovdqa	.LC17(%rip), %xmm0
	vmovdqa	%ymm6, 64(%rsp)
	vmovdqa	%xmm0, c.1(%rip)
	vmovdqa	%ymm3, 32(%rsp)
	vmovdqa	%ymm4, (%rsp)
	.p2align 4
	.p2align 3
.L2:
	vmovdqu	-12(%rax), %ymm0
	vmovdqa	-14(%rax), %ymm7
	addq	$64, %rdx
	addq	$32, %rax
	vmovdqa	160(%rsp), %ymm11
	vmovdqu	-42(%rax), %ymm6
	vmovdqa	128(%rsp), %ymm10
	vmovdqu	-40(%rax), %ymm5
	vpmullw	%ymm11, %ymm0, %ymm1
	vpmulhw	%ymm11, %ymm0, %ymm0
	vmovdqa	96(%rsp), %ymm15
	vmovdqu	-38(%rax), %ymm4
	vmovdqu	-36(%rax), %ymm3
	vmovdqu	-34(%rax), %ymm2
	vmovdqu	-32(%rax), %ymm8
	vpunpcklwd	%ymm0, %ymm1, %ymm14
	vpunpckhwd	%ymm0, %ymm1, %ymm1
	vmovdqa	%ymm1, 224(%rsp)
	vpmullw	%ymm10, %ymm7, %ymm1
	vpmulhw	%ymm10, %ymm7, %ymm7
	vperm2i128	$32, 224(%rsp), %ymm14, %ymm9
	vperm2i128	$49, 224(%rsp), %ymm14, %ymm14
	vpunpcklwd	%ymm7, %ymm1, %ymm0
	vpunpckhwd	%ymm7, %ymm1, %ymm7
	vmovdqa	%ymm7, 192(%rsp)
	vpmullw	%ymm11, %ymm6, %ymm7
	vpmulhw	%ymm11, %ymm6, %ymm6
	vperm2i128	$32, 192(%rsp), %ymm0, %ymm1
	vperm2i128	$49, 192(%rsp), %ymm0, %ymm0
	vpaddd	%ymm9, %ymm1, %ymm1
	vmovdqa	64(%rsp), %ymm9
	vpaddd	%ymm14, %ymm0, %ymm0
	vpunpcklwd	%ymm6, %ymm7, %ymm13
	vpunpckhwd	%ymm6, %ymm7, %ymm7
	vperm2i128	$32, %ymm7, %ymm13, %ymm6
	vperm2i128	$49, %ymm7, %ymm13, %ymm13
	vpaddd	%ymm6, %ymm1, %ymm1
	vpmullw	%ymm10, %ymm5, %ymm6
	vpaddd	%ymm13, %ymm0, %ymm0
	vpmulhw	%ymm10, %ymm5, %ymm5
	vpunpcklwd	%ymm5, %ymm6, %ymm12
	vpunpckhwd	%ymm5, %ymm6, %ymm6
	vperm2i128	$32, %ymm6, %ymm12, %ymm5
	vperm2i128	$49, %ymm6, %ymm12, %ymm12
	vpaddd	%ymm5, %ymm1, %ymm1
	vpmullw	%ymm15, %ymm4, %ymm5
	vpaddd	%ymm12, %ymm0, %ymm0
	vpmulhw	%ymm15, %ymm4, %ymm4
	vmovdqa	32(%rsp), %ymm15
	vpunpcklwd	%ymm4, %ymm5, %ymm11
	vpunpckhwd	%ymm4, %ymm5, %ymm5
	vperm2i128	$32, %ymm5, %ymm11, %ymm4
	vperm2i128	$49, %ymm5, %ymm11, %ymm11
	vpaddd	%ymm4, %ymm1, %ymm1
	vpmullw	%ymm9, %ymm3, %ymm4
	vpaddd	%ymm11, %ymm0, %ymm0
	vpmulhw	%ymm9, %ymm3, %ymm3
	vpunpcklwd	%ymm3, %ymm4, %ymm10
	vpunpckhwd	%ymm3, %ymm4, %ymm4
	vperm2i128	$32, %ymm4, %ymm10, %ymm3
	vperm2i128	$49, %ymm4, %ymm10, %ymm10
	vpaddd	%ymm3, %ymm1, %ymm1
	vpmullw	%ymm15, %ymm2, %ymm3
	vpaddd	%ymm10, %ymm0, %ymm0
	vpmulhw	%ymm15, %ymm2, %ymm2
	vpunpcklwd	%ymm2, %ymm3, %ymm9
	vpunpckhwd	%ymm2, %ymm3, %ymm3
	vperm2i128	$32, %ymm3, %ymm9, %ymm2
	vperm2i128	$49, %ymm3, %ymm9, %ymm9
	vpaddd	%ymm2, %ymm1, %ymm1
	vmovdqa	(%rsp), %ymm2
	vpaddd	%ymm9, %ymm0, %ymm0
	vpmullw	%ymm2, %ymm8, %ymm15
	vpmulhw	%ymm2, %ymm8, %ymm2
	vpunpcklwd	%ymm2, %ymm15, %ymm8
	vpunpckhwd	%ymm2, %ymm15, %ymm2
	vperm2i128	$32, %ymm2, %ymm8, %ymm15
	vperm2i128	$49, %ymm2, %ymm8, %ymm8
	vpaddd	%ymm15, %ymm1, %ymm1
	vpaddd	%ymm8, %ymm0, %ymm0
	vmovdqa	%ymm1, -64(%rdx)
	vmovdqa	%ymm0, -32(%rdx)
	cmpq	%rdx, %r8
	jne	.L2
	movabsq	$1099511628211, %rdi
	movl	$146959, %edx
	.p2align 6
	.p2align 4
	.p2align 3
.L3:
	movl	(%rcx), %eax
	addq	$4, %rcx
	movzwl	%ax, %esi
	shrl	$16, %eax
	xorq	%rdx, %rsi
	imulq	%rdi, %rsi
	xorq	%rsi, %rax
	imulq	%rdi, %rax
	movq	%rax, %rdx
	cmpq	%rcx, %r8
	jne	.L3
	movq	%rax, %rsi
	leaq	.LC24(%rip), %rdi
	xorl	%eax, %eax
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
	.comm	d.0,1024,32
	.local	c.1
	.comm	c.1,16,16
	.local	a.2
	.comm	a.2,528,32
	.section	.rodata.cst32,"aM",@progbits,32
	.align 32
.LC0:
	.value	-499
	.value	-242
	.value	15
	.value	272
	.value	-495
	.value	-238
	.value	19
	.value	276
	.value	-491
	.value	-234
	.value	23
	.value	280
	.value	-487
	.value	-230
	.value	27
	.value	284
	.align 32
.LC1:
	.value	-483
	.value	-226
	.value	31
	.value	288
	.value	-479
	.value	-222
	.value	35
	.value	292
	.value	-475
	.value	-218
	.value	39
	.value	296
	.value	-471
	.value	-214
	.value	43
	.value	300
	.align 32
.LC2:
	.value	-467
	.value	-210
	.value	47
	.value	304
	.value	-463
	.value	-206
	.value	51
	.value	308
	.value	-459
	.value	-202
	.value	55
	.value	312
	.value	-455
	.value	-198
	.value	59
	.value	316
	.align 32
.LC3:
	.value	-451
	.value	-194
	.value	63
	.value	320
	.value	-447
	.value	-190
	.value	67
	.value	324
	.value	-443
	.value	-186
	.value	71
	.value	328
	.value	-439
	.value	-182
	.value	75
	.value	332
	.align 32
.LC4:
	.value	-435
	.value	-178
	.value	79
	.value	336
	.value	-431
	.value	-174
	.value	83
	.value	340
	.value	-427
	.value	-170
	.value	87
	.value	344
	.value	-423
	.value	-166
	.value	91
	.value	348
	.align 32
.LC5:
	.value	-419
	.value	-162
	.value	95
	.value	352
	.value	-415
	.value	-158
	.value	99
	.value	356
	.value	-411
	.value	-154
	.value	103
	.value	360
	.value	-407
	.value	-150
	.value	107
	.value	364
	.align 32
.LC6:
	.value	-403
	.value	-146
	.value	111
	.value	368
	.value	-399
	.value	-142
	.value	115
	.value	372
	.value	-395
	.value	-138
	.value	119
	.value	376
	.value	-391
	.value	-134
	.value	123
	.value	380
	.align 32
.LC7:
	.value	-387
	.value	-130
	.value	127
	.value	384
	.value	-383
	.value	-126
	.value	131
	.value	388
	.value	-379
	.value	-122
	.value	135
	.value	392
	.value	-375
	.value	-118
	.value	139
	.value	396
	.align 32
.LC8:
	.value	-371
	.value	-114
	.value	143
	.value	400
	.value	-367
	.value	-110
	.value	147
	.value	404
	.value	-363
	.value	-106
	.value	151
	.value	408
	.value	-359
	.value	-102
	.value	155
	.value	412
	.align 32
.LC9:
	.value	-355
	.value	-98
	.value	159
	.value	416
	.value	-351
	.value	-94
	.value	163
	.value	420
	.value	-347
	.value	-90
	.value	167
	.value	424
	.value	-343
	.value	-86
	.value	171
	.value	428
	.align 32
.LC10:
	.value	-339
	.value	-82
	.value	175
	.value	432
	.value	-335
	.value	-78
	.value	179
	.value	436
	.value	-331
	.value	-74
	.value	183
	.value	440
	.value	-327
	.value	-70
	.value	187
	.value	444
	.align 32
.LC11:
	.value	-323
	.value	-66
	.value	191
	.value	448
	.value	-319
	.value	-62
	.value	195
	.value	452
	.value	-315
	.value	-58
	.value	199
	.value	456
	.value	-311
	.value	-54
	.value	203
	.value	460
	.align 32
.LC12:
	.value	-307
	.value	-50
	.value	207
	.value	464
	.value	-303
	.value	-46
	.value	211
	.value	468
	.value	-299
	.value	-42
	.value	215
	.value	472
	.value	-295
	.value	-38
	.value	219
	.value	476
	.align 32
.LC13:
	.value	-291
	.value	-34
	.value	223
	.value	480
	.value	-287
	.value	-30
	.value	227
	.value	484
	.value	-283
	.value	-26
	.value	231
	.value	488
	.value	-279
	.value	-22
	.value	235
	.value	492
	.align 32
.LC14:
	.value	-275
	.value	-18
	.value	239
	.value	496
	.value	-271
	.value	-14
	.value	243
	.value	500
	.value	-267
	.value	-10
	.value	247
	.value	504
	.value	-263
	.value	-6
	.value	251
	.value	508
	.align 32
.LC15:
	.value	-259
	.value	-2
	.value	255
	.value	-512
	.value	-255
	.value	2
	.value	259
	.value	-508
	.value	-251
	.value	6
	.value	263
	.value	-504
	.value	-247
	.value	10
	.value	267
	.value	-500
	.section	.rodata.cst16,"aM",@progbits,16
	.align 16
.LC16:
	.value	-243
	.value	14
	.value	271
	.value	-496
	.value	-239
	.value	18
	.value	275
	.value	-492
	.align 16
.LC17:
	.value	7
	.value	5
	.value	5
	.value	7
	.value	11
	.value	17
	.value	25
	.value	35
	.ident	"GCC: (Debian 14.2.0-19) 14.2.0"
	.section	.note.GNU-stack,"",@progbits
