# Session 71 — 12 baseline FAILs + SSO bitfields

## Root cause of the 12

Not 12 bugs. Two tests (`clmul256`, `vectorize_fp_fast_math_contract`)
already passed with their `.flags`. The other ten failed because
`emit_float_binop_into_reg` materialised `+0.0` with `xorpd %dest,%dest`
*after* a multiply had been coalesced into that same XMM (and similarly
clobbered dest when loading a const/GPR rhs on top of a live lhs).

dot4 became:

    vmulsd (%rsi), %xmm2, %xmm2
    xorpd  %xmm2, %xmm2
    vaddsd %xmm2, %xmm2, %xmm2   # 0+0

Fix: 3-operand VEX, never xor dest when it holds an operand; `s = 0 + p`
is `load p`. Unblocked fma_chain, spectral, float_direct_slot,
vectorize_sum_f32, session45 RMW, and several tests that only failed
because the same FP path ran in their TU.

## SSO

`TypeSpecifier::{Struct,Union}` 6th flag + `StructLayout.reverse_sso`.
`scalar_storage_order("big-endian")` on LE remaps bit windows to
`N*8-p-w`. Fields round-trip (`tests/regression/sso_bitfields.c`).
Unit byte-swap vs GCC `aa f8 00 00` is still open (we store remapped
bits in host endian: `00 00 f8 aa`).
