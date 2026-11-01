	.file	"predicate-materialize.c"
	.text
	.p2align 4
	.globl	from_cmp
	.type	from_cmp, @function
from_cmp:
.LFB0:
	.cfi_startproc
	xorl	%eax, %eax
	cmpl	%esi, %edi
	setne	%al
	ret
	.cfi_endproc
.LFE0:
	.size	from_cmp, .-from_cmp
	.p2align 4
	.globl	from_cmp_neg
	.type	from_cmp_neg, @function
from_cmp_neg:
.LFB1:
	.cfi_startproc
	xorl	%eax, %eax
	cmpl	%esi, %edi
	sete	%al
	ret
	.cfi_endproc
.LFE1:
	.size	from_cmp_neg, .-from_cmp_neg
	.p2align 4
	.globl	from_lt
	.type	from_lt, @function
from_lt:
.LFB2:
	.cfi_startproc
	xorl	%eax, %eax
	cmpl	%esi, %edi
	setl	%al
	ret
	.cfi_endproc
.LFE2:
	.size	from_lt, .-from_lt
	.p2align 4
	.globl	from_lt_neg
	.type	from_lt_neg, @function
from_lt_neg:
.LFB3:
	.cfi_startproc
	xorl	%eax, %eax
	cmpl	%esi, %edi
	setge	%al
	ret
	.cfi_endproc
.LFE3:
	.size	from_lt_neg, .-from_lt_neg
	.p2align 4
	.globl	zero_or_one
	.type	zero_or_one, @function
zero_or_one:
.LFB4:
	.cfi_startproc
	xorl	%eax, %eax
	testl	%edi, %edi
	setne	%al
	ret
	.cfi_endproc
.LFE4:
	.size	zero_or_one, .-zero_or_one
	.p2align 4
	.globl	one_or_zero
	.type	one_or_zero, @function
one_or_zero:
.LFB5:
	.cfi_startproc
	xorl	%eax, %eax
	testl	%edi, %edi
	sete	%al
	ret
	.cfi_endproc
.LFE5:
	.size	one_or_zero, .-one_or_zero
	.p2align 4
	.globl	mask_of_sign
	.type	mask_of_sign, @function
mask_of_sign:
.LFB6:
	.cfi_startproc
	movl	%edi, %eax
	notl	%eax
	shrl	$31, %eax
	ret
	.cfi_endproc
.LFE6:
	.size	mask_of_sign, .-mask_of_sign
	.p2align 4
	.globl	zero_or_one64
	.type	zero_or_one64, @function
zero_or_one64:
.LFB7:
	.cfi_startproc
	xorl	%eax, %eax
	testq	%rdi, %rdi
	setne	%al
	ret
	.cfi_endproc
.LFE7:
	.size	zero_or_one64, .-zero_or_one64
	.p2align 4
	.globl	or_or
	.type	or_or, @function
or_or:
.LFB8:
	.cfi_startproc
	xorl	%eax, %eax
	orl	%esi, %edi
	setne	%al
	ret
	.cfi_endproc
.LFE8:
	.size	or_or, .-or_or
	.p2align 4
	.globl	and_eq
	.type	and_eq, @function
and_eq:
.LFB9:
	.cfi_startproc
	xorl	%eax, %eax
	orl	%esi, %edi
	sete	%al
	ret
	.cfi_endproc
.LFE9:
	.size	and_eq, .-and_eq
	.p2align 4
	.globl	or3
	.type	or3, @function
or3:
.LFB10:
	.cfi_startproc
	orl	%edx, %esi
	xorl	%eax, %eax
	orl	%edi, %esi
	setne	%al
	ret
	.cfi_endproc
.LFE10:
	.size	or3, .-or3
	.p2align 4
	.globl	keep_and_ne
	.type	keep_and_ne, @function
keep_and_ne:
.LFB11:
	.cfi_startproc
	testl	%edi, %edi
	setne	%dl
	xorl	%eax, %eax
	testl	%esi, %esi
	setne	%al
	andl	%edx, %eax
	ret
	.cfi_endproc
.LFE11:
	.size	keep_and_ne, .-keep_and_ne
	.p2align 4
	.globl	keep_or_eq
	.type	keep_or_eq, @function
keep_or_eq:
.LFB12:
	.cfi_startproc
	testl	%edi, %edi
	sete	%al
	testl	%esi, %esi
	sete	%dl
	orl	%edx, %eax
	movzbl	%al, %eax
	ret
	.cfi_endproc
.LFE12:
	.size	keep_or_eq, .-keep_or_eq
	.p2align 4
	.globl	keep_xor
	.type	keep_xor, @function
keep_xor:
.LFB13:
	.cfi_startproc
	testl	%edi, %edi
	setne	%al
	testl	%esi, %esi
	setne	%dl
	xorl	%edx, %eax
	movzbl	%al, %eax
	ret
	.cfi_endproc
.LFE13:
	.size	keep_xor, .-keep_xor
	.p2align 4
	.globl	keep_three_two
	.type	keep_three_two, @function
keep_three_two:
.LFB14:
	.cfi_startproc
	cmpl	$1, %edi
	movl	$2, %eax
	sbbl	$-1, %eax
	ret
	.cfi_endproc
.LFE14:
	.size	keep_three_two, .-keep_three_two
	.p2align 4
	.globl	keep_one_two
	.type	keep_one_two, @function
keep_one_two:
.LFB15:
	.cfi_startproc
	cmpl	$1, %edi
	movl	$1, %eax
	adcl	$0, %eax
	ret
	.cfi_endproc
.LFE15:
	.size	keep_one_two, .-keep_one_two
	.p2align 4
	.globl	keep_shared
	.type	keep_shared, @function
keep_shared:
.LFB16:
	.cfi_startproc
	xorl	%eax, %eax
	cmpl	%esi, %edi
	setne	%al
	movl	%eax, g_shared(%rip)
	sete	%al
	movzbl	%al, %eax
	ret
	.cfi_endproc
.LFE16:
	.size	keep_shared, .-keep_shared
	.globl	g_shared
	.bss
	.align 4
	.type	g_shared, @object
	.size	g_shared, 4
g_shared:
	.zero	4
	.ident	"GCC: (GNU) 16.2.0"
	.section	.note.GNU-stack,"",@progbits
