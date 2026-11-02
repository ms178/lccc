# Symbol + constant-chain data expressions (kernel jump-label keys).
#
# The JUMP_TABLE_ENTRY macro emits `.quad %c0 + N + M - .` where %c0 already
# prints as "sym+8", producing the shape `sym+8 + 0 + 2 - .`. The " - " diff
# path must fold the WHOLE left-hand constant chain into one addend; peeling
# a single term left a relocation against the literal name "sym+8 + 0"
# ("undefined reference to `__tracepoint_read_msr+8 + 0 + 2'" at link).
.text
f:
	ret
.section .data
.quad ext_sym+8 + 0 + 2 - .
.quad ext_sym + (1 << 4) - 8 + 2 - .
.quad ext_sym+16 - .
