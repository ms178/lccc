	.file	"intervals.c"
	.text
	.p2align 4
	.globl	uchar_le
	.type	uchar_le, @function
uchar_le:
.LFB0:
	.cfi_startproc
	movl	$1, %eax
	ret
	.cfi_endproc
.LFE0:
	.size	uchar_le, .-uchar_le
	.p2align 4
	.globl	uchar_lt
	.type	uchar_lt, @function
uchar_lt:
.LFB22:
	.cfi_startproc
	movl	$1, %eax
	ret
	.cfi_endproc
.LFE22:
	.size	uchar_lt, .-uchar_lt
	.p2align 4
	.globl	uchar_gt
	.type	uchar_gt, @function
uchar_gt:
.LFB2:
	.cfi_startproc
	xorl	%eax, %eax
	ret
	.cfi_endproc
.LFE2:
	.size	uchar_gt, .-uchar_gt
	.p2align 4
	.globl	uchar_eq
	.type	uchar_eq, @function
uchar_eq:
.LFB34:
	.cfi_startproc
	xorl	%eax, %eax
	ret
	.cfi_endproc
.LFE34:
	.size	uchar_eq, .-uchar_eq
	.p2align 4
	.globl	ushort_le
	.type	ushort_le, @function
ushort_le:
.LFB24:
	.cfi_startproc
	movl	$1, %eax
	ret
	.cfi_endproc
.LFE24:
	.size	ushort_le, .-ushort_le
	.p2align 4
	.globl	uint_le
	.type	uint_le, @function
uint_le:
.LFB5:
	.cfi_startproc
	movl	$1, %eax
	ret
	.cfi_endproc
.LFE5:
	.size	uint_le, .-uint_le
	.p2align 4
	.globl	schar_lt
	.type	schar_lt, @function
schar_lt:
.LFB26:
	.cfi_startproc
	movl	$1, %eax
	ret
	.cfi_endproc
.LFE26:
	.size	schar_lt, .-schar_lt
	.p2align 4
	.globl	schar_gt
	.type	schar_gt, @function
schar_gt:
.LFB36:
	.cfi_startproc
	xorl	%eax, %eax
	ret
	.cfi_endproc
.LFE36:
	.size	schar_gt, .-schar_gt
	.p2align 4
	.globl	schar_ge
	.type	schar_ge, @function
schar_ge:
.LFB28:
	.cfi_startproc
	movl	$1, %eax
	ret
	.cfi_endproc
.LFE28:
	.size	schar_ge, .-schar_ge
	.p2align 4
	.globl	schar_eq
	.type	schar_eq, @function
schar_eq:
.LFB38:
	.cfi_startproc
	xorl	%eax, %eax
	ret
	.cfi_endproc
.LFE38:
	.size	schar_eq, .-schar_eq
	.p2align 4
	.globl	sshort_le
	.type	sshort_le, @function
sshort_le:
.LFB30:
	.cfi_startproc
	movl	$1, %eax
	ret
	.cfi_endproc
.LFE30:
	.size	sshort_le, .-sshort_le
	.p2align 4
	.globl	int_gt_max
	.type	int_gt_max, @function
int_gt_max:
.LFB40:
	.cfi_startproc
	xorl	%eax, %eax
	ret
	.cfi_endproc
.LFE40:
	.size	int_gt_max, .-int_gt_max
	.p2align 4
	.globl	int_lt_min
	.type	int_lt_min, @function
int_lt_min:
.LFB42:
	.cfi_startproc
	xorl	%eax, %eax
	ret
	.cfi_endproc
.LFE42:
	.size	int_lt_min, .-int_lt_min
	.p2align 4
	.globl	uint_le_max
	.type	uint_le_max, @function
uint_le_max:
.LFB13:
	.cfi_startproc
	movl	$1, %eax
	ret
	.cfi_endproc
.LFE13:
	.size	uint_le_max, .-uint_le_max
	.p2align 4
	.globl	uint_gt_max
	.type	uint_gt_max, @function
uint_gt_max:
.LFB14:
	.cfi_startproc
	xorl	%eax, %eax
	ret
	.cfi_endproc
.LFE14:
	.size	uint_gt_max, .-uint_gt_max
	.p2align 4
	.globl	int_ule_neg1
	.type	int_ule_neg1, @function
int_ule_neg1:
.LFB32:
	.cfi_startproc
	movl	$1, %eax
	ret
	.cfi_endproc
.LFE32:
	.size	int_ule_neg1, .-int_ule_neg1
	.p2align 4
	.globl	keep_uchar_lt
	.type	keep_uchar_lt, @function
keep_uchar_lt:
.LFB16:
	.cfi_startproc
	xorl	%eax, %eax
	cmpb	$99, %dil
	setbe	%al
	ret
	.cfi_endproc
.LFE16:
	.size	keep_uchar_lt, .-keep_uchar_lt
	.p2align 4
	.globl	keep_schar_gt
	.type	keep_schar_gt, @function
keep_schar_gt:
.LFB17:
	.cfi_startproc
	xorl	%eax, %eax
	cmpb	$-99, %dil
	setge	%al
	ret
	.cfi_endproc
.LFE17:
	.size	keep_schar_gt, .-keep_schar_gt
	.p2align 4
	.globl	keep_schar_ult
	.type	keep_schar_ult, @function
keep_schar_ult:
.LFB18:
	.cfi_startproc
	movsbl	%dil, %eax
	notl	%eax
	shrl	$31, %eax
	ret
	.cfi_endproc
.LFE18:
	.size	keep_schar_ult, .-keep_schar_ult
	.p2align 4
	.globl	keep_uint_ge_max
	.type	keep_uint_ge_max, @function
keep_uint_ge_max:
.LFB19:
	.cfi_startproc
	xorl	%eax, %eax
	cmpl	$-1, %edi
	sete	%al
	ret
	.cfi_endproc
.LFE19:
	.size	keep_uint_ge_max, .-keep_uint_ge_max
	.p2align 4
	.globl	keep_int_eq
	.type	keep_int_eq, @function
keep_int_eq:
.LFB20:
	.cfi_startproc
	xorl	%eax, %eax
	cmpl	$5, %edi
	sete	%al
	ret
	.cfi_endproc
.LFE20:
	.size	keep_int_eq, .-keep_int_eq
	.ident	"GCC: (Debian 12.2.0-14+deb12u1) 12.2.0"
	.section	.note.GNU-stack,"",@progbits
