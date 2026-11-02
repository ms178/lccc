# GAS character literals in macro arguments and instruction operands.
#
# arch/x86/kernel/relocate_kernel_64.S passes ' ' (a SPACE char literal) and
# ',' (a COMMA char literal) as macro arguments:
#     pr 'r', '8', ' ', ':', %r8
# The macro-argument splitter, the operand splitter, and the whitespace
# tokenizer must all treat the byte after a single quote as DATA — splitting
# inside the literal turned $' ' into a relocation against the symbol `'`
# ("undefined reference to `''" at vmlinux link).
.text
.macro pr c1, c2, c3, c4, reg
	movb $\c1, %bl
	movb $\c2, %bl
	movb $\c3, %bl
	movb $\c4, %bl
	movq \reg, %rdx
.endm
f:
	pr 'r', '8', ' ', ':', %r8
	pr ',', 'x', '\n', ' ', %r9
	movb $' ', %bl
	movb $',', %bl
	movb $'\n', %bl
	movb $'r', %bl
	ret
