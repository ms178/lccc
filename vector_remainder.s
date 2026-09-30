	.file	"vector_remainder.c"
	.text
	.p2align 4
	.type	dot_f64.constprop.0, @function
dot_f64.constprop.0:
.LFB25:
	.cfi_startproc
	testl	%edi, %edi
	jle	.L9
	leal	-1(%rdi), %eax
	cmpl	$2, %eax
	jbe	.L10
	movl	%edi, %edx
	vxorpd	%xmm0, %xmm0, %xmm0
	leaq	a(%rip), %rsi
	xorl	%eax, %eax
	shrl	$2, %edx
	leaq	b(%rip), %rcx
	salq	$5, %rdx
	.p2align 6
	.p2align 4
	.p2align 3
.L4:
	vmovapd	(%rsi,%rax), %ymm1
	vmulpd	(%rcx,%rax), %ymm1, %ymm1
	addq	$32, %rax
	vaddsd	%xmm1, %xmm0, %xmm0
	vunpckhpd	%xmm1, %xmm1, %xmm2
	vextractf128	$0x1, %ymm1, %xmm1
	vaddsd	%xmm2, %xmm0, %xmm0
	vaddsd	%xmm1, %xmm0, %xmm0
	vunpckhpd	%xmm1, %xmm1, %xmm1
	vaddsd	%xmm1, %xmm0, %xmm0
	cmpq	%rdx, %rax
	jne	.L4
	movl	%edi, %eax
	andl	$-4, %eax
	movl	%eax, %edx
	cmpl	%eax, %edi
	je	.L20
	vzeroupper
.L3:
	subl	%edx, %edi
	cmpl	$1, %edi
	je	.L7
	vmovapd	(%rsi,%rdx,8), %xmm1
	vmulpd	(%rcx,%rdx,8), %xmm1, %xmm1
	vaddsd	%xmm1, %xmm0, %xmm0
	vunpckhpd	%xmm1, %xmm1, %xmm1
	vaddsd	%xmm0, %xmm1, %xmm0
	testb	$1, %dil
	je	.L1
	andl	$-2, %edi
	addl	%edi, %eax
.L7:
	cltq
	vmovsd	(%rsi,%rax,8), %xmm4
	vfmadd231sd	(%rcx,%rax,8), %xmm4, %xmm0
	ret
	.p2align 4,,10
	.p2align 3
.L9:
	vxorpd	%xmm0, %xmm0, %xmm0
.L1:
	ret
	.p2align 4,,10
	.p2align 3
.L20:
	vzeroupper
	ret
.L10:
	xorl	%edx, %edx
	vxorpd	%xmm0, %xmm0, %xmm0
	leaq	a(%rip), %rsi
	xorl	%eax, %eax
	leaq	b(%rip), %rcx
	jmp	.L3
	.cfi_endproc
.LFE25:
	.size	dot_f64.constprop.0, .-dot_f64.constprop.0
	.p2align 4
	.type	sum_f64.constprop.0, @function
sum_f64.constprop.0:
.LFB26:
	.cfi_startproc
	movl	%edi, %ecx
	testl	%edi, %edi
	jle	.L27
	leal	-1(%rdi), %eax
	cmpl	$2, %eax
	jbe	.L28
	movl	%edi, %edx
	leaq	a(%rip), %rsi
	vxorpd	%xmm0, %xmm0, %xmm0
	shrl	$2, %edx
	movq	%rsi, %rax
	salq	$5, %rdx
	addq	%rsi, %rdx
	.p2align 5
	.p2align 4
	.p2align 3
.L24:
	vaddsd	(%rax), %xmm0, %xmm0
	addq	$32, %rax
	vaddsd	-24(%rax), %xmm0, %xmm0
	vaddsd	-16(%rax), %xmm0, %xmm0
	vaddsd	-8(%rax), %xmm0, %xmm0
	cmpq	%rdx, %rax
	jne	.L24
	movl	%ecx, %eax
	andl	$-4, %eax
	testb	$3, %cl
	je	.L30
.L23:
	movslq	%eax, %rdx
	vaddsd	(%rsi,%rdx,8), %xmm0, %xmm0
	leal	1(%rax), %esi
	cmpl	%esi, %ecx
	jle	.L21
	leaq	8+a(%rip), %rsi
	addl	$2, %eax
	vaddsd	(%rsi,%rdx,8), %xmm0, %xmm0
	cmpl	%eax, %ecx
	jle	.L21
	leaq	8(%rsi), %rax
	vaddsd	(%rax,%rdx,8), %xmm0, %xmm0
	ret
	.p2align 4,,10
	.p2align 3
.L27:
	vxorpd	%xmm0, %xmm0, %xmm0
.L21:
	ret
	.p2align 4,,10
	.p2align 3
.L30:
	ret
.L28:
	vxorpd	%xmm0, %xmm0, %xmm0
	xorl	%eax, %eax
	leaq	a(%rip), %rsi
	jmp	.L23
	.cfi_endproc
.LFE26:
	.size	sum_f64.constprop.0, .-sum_f64.constprop.0
	.section	.rodata.str1.1,"aMS",@progbits,1
.LC35:
	.string	"%.0f\n"
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
	movl	$1, %r11d
	movq	%rsp, %rbp
	.cfi_def_cfa_register 6
	pushq	%r10
	subq	$24, %rsp
	.cfi_offset 10, -24
	cmpl	$1, %edi
	jle	.L32
	movq	8(%rsi), %rdi
	movl	$10, %edx
	xorl	%esi, %esi
	call	strtol@PLT
	movl	%eax, %r11d
.L32:
	vmovapd	.LC1(%rip), %ymm0
	xorl	%r10d, %r10d
	vxorpd	%xmm1, %xmm1, %xmm1
	leaq	72+bounds.0(%rip), %r9
	vmovapd	%ymm0, a(%rip)
	vmovapd	.LC2(%rip), %ymm0
	vmovapd	%ymm0, 32+a(%rip)
	vmovapd	.LC3(%rip), %ymm0
	vmovapd	%ymm0, b(%rip)
	vmovapd	.LC4(%rip), %ymm0
	vmovapd	%ymm0, 32+b(%rip)
	vmovapd	.LC5(%rip), %ymm0
	vmovapd	%ymm0, 64+a(%rip)
	vmovapd	.LC6(%rip), %ymm0
	vmovapd	%ymm0, 96+a(%rip)
	vmovapd	.LC7(%rip), %ymm0
	vmovapd	%ymm0, 64+b(%rip)
	vmovapd	.LC8(%rip), %ymm0
	vmovapd	%ymm0, 96+b(%rip)
	vmovapd	.LC9(%rip), %ymm0
	vmovapd	%ymm0, 128+a(%rip)
	vmovapd	.LC10(%rip), %ymm0
	vmovapd	%ymm0, 160+a(%rip)
	vmovapd	.LC11(%rip), %ymm0
	vmovapd	%ymm0, 128+b(%rip)
	vmovapd	.LC12(%rip), %ymm0
	vmovapd	%ymm0, 160+b(%rip)
	vmovapd	.LC13(%rip), %ymm0
	vmovapd	%ymm0, 192+a(%rip)
	vmovapd	.LC14(%rip), %ymm0
	vmovapd	%ymm0, 224+a(%rip)
	vmovapd	.LC15(%rip), %ymm0
	vmovapd	%ymm0, 192+b(%rip)
	vmovapd	.LC16(%rip), %ymm0
	vmovapd	%ymm0, 224+b(%rip)
	vmovapd	.LC17(%rip), %ymm0
	vmovapd	%ymm0, 256+a(%rip)
	vmovapd	.LC18(%rip), %ymm0
	vmovapd	%ymm0, 288+a(%rip)
	vmovapd	.LC19(%rip), %ymm0
	vmovapd	%ymm0, 256+b(%rip)
	vmovapd	.LC20(%rip), %ymm0
	vmovapd	%ymm0, 288+b(%rip)
	vmovapd	.LC21(%rip), %ymm0
	vmovapd	%ymm0, 320+a(%rip)
	vmovapd	.LC22(%rip), %ymm0
	vmovapd	%ymm0, 352+a(%rip)
	vmovapd	.LC23(%rip), %ymm0
	vmovapd	%ymm0, 320+b(%rip)
	vmovapd	.LC24(%rip), %ymm0
	vmovapd	%ymm0, 352+b(%rip)
	vmovapd	.LC25(%rip), %ymm0
	vmovapd	%ymm0, 384+a(%rip)
	vmovapd	.LC26(%rip), %ymm0
	vmovapd	%ymm0, 416+a(%rip)
	vmovapd	.LC27(%rip), %ymm0
	vmovapd	%ymm0, 384+b(%rip)
	vmovapd	.LC28(%rip), %ymm0
	vmovapd	%ymm0, 416+b(%rip)
	vmovapd	.LC29(%rip), %ymm0
	vmovapd	%ymm0, 448+a(%rip)
	vmovapd	.LC30(%rip), %ymm0
	vmovapd	%ymm0, 480+a(%rip)
	vmovapd	.LC31(%rip), %ymm0
	vmovapd	%ymm0, 448+b(%rip)
	vmovapd	.LC32(%rip), %ymm0
	vmovapd	%ymm0, 480+b(%rip)
	movq	.LC33(%rip), %rax
	movq	%rax, 512+a(%rip)
	movq	.LC34(%rip), %rax
	movq	%rax, 512+b(%rip)
	testl	%r11d, %r11d
	jle	.L43
	vzeroupper
	.p2align 4
	.p2align 3
.L33:
	leaq	bounds.0(%rip), %r8
	.p2align 4
	.p2align 3
.L35:
	movl	(%r8), %edi
	addq	$4, %r8
	call	sum_f64.constprop.0
	vaddsd	%xmm1, %xmm0, %xmm2
	vmovsd	%xmm2, -24(%rbp)
	call	dot_f64.constprop.0
	vaddsd	-24(%rbp), %xmm0, %xmm1
	cmpq	%r8, %r9
	jne	.L35
	addl	$1, %r10d
	cmpl	%r10d, %r11d
	jne	.L33
.L34:
	vmovapd	%xmm1, %xmm0
	leaq	.LC35(%rip), %rdi
	movl	$1, %eax
	call	printf@PLT
	addq	$24, %rsp
	xorl	%eax, %eax
	popq	%r10
	popq	%rbp
	.cfi_remember_state
	.cfi_def_cfa 7, 8
	ret
.L43:
	.cfi_restore_state
	vzeroupper
	jmp	.L34
	.cfi_endproc
.LFE24:
	.size	main, .-main
	.section	.rodata
	.align 32
	.type	bounds.0, @object
	.size	bounds.0, 72
bounds.0:
	.long	0
	.long	1
	.long	2
	.long	3
	.long	4
	.long	5
	.long	7
	.long	8
	.long	9
	.long	15
	.long	16
	.long	17
	.long	31
	.long	32
	.long	33
	.long	63
	.long	64
	.long	65
	.local	b
	.comm	b,520,32
	.local	a
	.comm	a,520,32
	.section	.rodata.cst32,"aM",@progbits,32
	.align 32
.LC1:
	.long	0
	.long	1072693248
	.long	0
	.long	1073741824
	.long	0
	.long	1074266112
	.long	0
	.long	1074790400
	.align 32
.LC2:
	.long	0
	.long	1075052544
	.long	0
	.long	1075314688
	.long	0
	.long	1075576832
	.long	0
	.long	1075838976
	.align 32
.LC3:
	.long	0
	.long	1073741824
	.long	0
	.long	1074266112
	.long	0
	.long	1074790400
	.long	0
	.long	1075052544
	.align 32
.LC4:
	.long	0
	.long	1075314688
	.long	0
	.long	1075576832
	.long	0
	.long	1075838976
	.long	0
	.long	1075970048
	.align 32
.LC5:
	.long	0
	.long	1075970048
	.long	0
	.long	1076101120
	.long	0
	.long	1076232192
	.long	0
	.long	1076363264
	.align 32
.LC6:
	.long	0
	.long	1076494336
	.long	0
	.long	1076625408
	.long	0
	.long	1076756480
	.long	0
	.long	1076887552
	.align 32
.LC7:
	.long	0
	.long	1076101120
	.long	0
	.long	1076232192
	.long	0
	.long	1076363264
	.long	0
	.long	1076494336
	.align 32
.LC8:
	.long	0
	.long	1076625408
	.long	0
	.long	1076756480
	.long	0
	.long	1076887552
	.long	0
	.long	1076953088
	.align 32
.LC9:
	.long	0
	.long	1076953088
	.long	0
	.long	1077018624
	.long	0
	.long	1077084160
	.long	0
	.long	1077149696
	.align 32
.LC10:
	.long	0
	.long	1077215232
	.long	0
	.long	1077280768
	.long	0
	.long	1077346304
	.long	0
	.long	1077411840
	.align 32
.LC11:
	.long	0
	.long	1077018624
	.long	0
	.long	1077084160
	.long	0
	.long	1077149696
	.long	0
	.long	1077215232
	.align 32
.LC12:
	.long	0
	.long	1077280768
	.long	0
	.long	1077346304
	.long	0
	.long	1077411840
	.long	0
	.long	1077477376
	.align 32
.LC13:
	.long	0
	.long	1077477376
	.long	0
	.long	1077542912
	.long	0
	.long	1077608448
	.long	0
	.long	1077673984
	.align 32
.LC14:
	.long	0
	.long	1077739520
	.long	0
	.long	1077805056
	.long	0
	.long	1077870592
	.long	0
	.long	1077936128
	.align 32
.LC15:
	.long	0
	.long	1077542912
	.long	0
	.long	1077608448
	.long	0
	.long	1077673984
	.long	0
	.long	1077739520
	.align 32
.LC16:
	.long	0
	.long	1077805056
	.long	0
	.long	1077870592
	.long	0
	.long	1077936128
	.long	0
	.long	1077968896
	.align 32
.LC17:
	.long	0
	.long	1077968896
	.long	0
	.long	1078001664
	.long	0
	.long	1078034432
	.long	0
	.long	1078067200
	.align 32
.LC18:
	.long	0
	.long	1078099968
	.long	0
	.long	1078132736
	.long	0
	.long	1078165504
	.long	0
	.long	1078198272
	.align 32
.LC19:
	.long	0
	.long	1078001664
	.long	0
	.long	1078034432
	.long	0
	.long	1078067200
	.long	0
	.long	1078099968
	.align 32
.LC20:
	.long	0
	.long	1078132736
	.long	0
	.long	1078165504
	.long	0
	.long	1078198272
	.long	0
	.long	1078231040
	.align 32
.LC21:
	.long	0
	.long	1078231040
	.long	0
	.long	1078263808
	.long	0
	.long	1078296576
	.long	0
	.long	1078329344
	.align 32
.LC22:
	.long	0
	.long	1078362112
	.long	0
	.long	1078394880
	.long	0
	.long	1078427648
	.long	0
	.long	1078460416
	.align 32
.LC23:
	.long	0
	.long	1078263808
	.long	0
	.long	1078296576
	.long	0
	.long	1078329344
	.long	0
	.long	1078362112
	.align 32
.LC24:
	.long	0
	.long	1078394880
	.long	0
	.long	1078427648
	.long	0
	.long	1078460416
	.long	0
	.long	1078493184
	.align 32
.LC25:
	.long	0
	.long	1078493184
	.long	0
	.long	1078525952
	.long	0
	.long	1078558720
	.long	0
	.long	1078591488
	.align 32
.LC26:
	.long	0
	.long	1078624256
	.long	0
	.long	1078657024
	.long	0
	.long	1078689792
	.long	0
	.long	1078722560
	.align 32
.LC27:
	.long	0
	.long	1078525952
	.long	0
	.long	1078558720
	.long	0
	.long	1078591488
	.long	0
	.long	1078624256
	.align 32
.LC28:
	.long	0
	.long	1078657024
	.long	0
	.long	1078689792
	.long	0
	.long	1078722560
	.long	0
	.long	1078755328
	.align 32
.LC29:
	.long	0
	.long	1078755328
	.long	0
	.long	1078788096
	.long	0
	.long	1078820864
	.long	0
	.long	1078853632
	.align 32
.LC30:
	.long	0
	.long	1078886400
	.long	0
	.long	1078919168
	.long	0
	.long	1078951936
	.long	0
	.long	1078984704
	.align 32
.LC31:
	.long	0
	.long	1078788096
	.long	0
	.long	1078820864
	.long	0
	.long	1078853632
	.long	0
	.long	1078886400
	.align 32
.LC32:
	.long	0
	.long	1078919168
	.long	0
	.long	1078951936
	.long	0
	.long	1078984704
	.long	0
	.long	1079001088
	.set	.LC33,.LC32+24
	.section	.rodata.cst8,"aM",@progbits,8
	.align 8
.LC34:
	.long	0
	.long	1079017472
	.ident	"GCC: (Debian 14.2.0-19) 14.2.0"
	.section	.note.GNU-stack,"",@progbits
