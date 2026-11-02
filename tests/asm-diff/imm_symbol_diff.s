# Symbol-difference and parenthesized-symbol immediates.
#
# relocate_kernel_64.S computes section-relative offsets in immediates:
#     0: addq $identity_mapped - 0b, %rsi
# and nospec-branch.h's __HANDLE_INTR_SAFERET emits parenthesized symbols
# with an addend:
#     cmpq $(srso_safe_ret)+5, RIP+pt_regs
# Both shapes previously degenerated into relocations against the LITERAL
# expression string ("identity_mapped - 0b", "(srso_safe_ret)+5") and the
# vmlinux link failed with undefined references.
#
# All references here are FORWARD (like the kernel's), where GAS also
# reserves imm32 — so the encodings must be byte-identical. (For
# backward-defined same-section pairs GAS folds at parse time and may
# pick imm8; lccc emits the semantically identical imm32 there, which is
# why this test deliberately avoids backward references.)
.text
0:	addq $fwd_lbl - 0b, %rsi
	subq $fwd_lbl2 - 0b, %rsi
	cmpq $(ext_sym)+5, 8(%rsp)
	cmpq $(ext_sym), 8(%rsp)
	ret
fwd_lbl:
	nop
fwd_lbl2:
	nop
