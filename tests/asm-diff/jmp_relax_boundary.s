# Regression (v4, GAS oracle): forward-jump relaxation must account for the
# jump's own shrink. A forward jcc with 32-bit displacement d fits the short
# form iff d <= 127 (the relaxation moves the target left by 4). The previous
# check used target - (offset + 2), overestimating by 4, so displacements
# 124..127 were never relaxed — diverging from GAS 2.47 (sqlite3 corpus t58).
# This file exercises both sides of every boundary:
#   jge +124, +127      -> 2-byte short form (relaxed)
#   jge +128            -> 6-byte long form (NOT relaxed)
#   jmp +127            -> 2-byte short form
#   jmp +128            -> 5-byte long form
#   backward jge -124   -> 2-byte short form
	.text
	.globl	main
	.type	main, @function
main:
	.cfi_startproc
	jge	.T127
	.fill	124, 1, 0x90
.T127:
	jge	.T128
	.fill	128, 1, 0x90
.T128:
	jmp	.J127
	.fill	124, 1, 0x90
.J127:
	jmp	.J128
	.fill	128, 1, 0x90
.J128:
	jge	.BACK
	.fill	200, 1, 0x90
.BACK:
	jge	.BACK2
	.fill	121, 1, 0x90
.BACK2:
	.fill	60, 1, 0x90
	jge	.T127
	xorl	%eax, %eax
	ret
	.cfi_endproc
	.size	main, .-main
