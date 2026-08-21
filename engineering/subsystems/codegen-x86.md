# x86-64 codegen

Text `ArchCodegen` + string peephole, with optional MachInst disabled on large loops. Accumulators `%rax`/`xmm0` still dominate some FP/aggregate paths.

Production improvements include masked-index SIB loads, BMI1-gated direct-return `andn`, and exact 64-byte AVX2 assignment. The latter is six instructions (four YMM moves, `vzeroupper`, `ret`), matching GCC 16.2, Clang 22.1 and ICX latest. Safe leaf DCE removes only proven-unneeded parameter homes; 32/48-byte copies remain XMM after a measured YMM slowdown.

Call-free CFG leaves with leading ParamRefs can retain up to six SysV register arguments in caller/ABI homes. If layout consumed only the conservative callee-save collision reserve, the duplicate local frame is elided while push/pop CFA remains exact. Kill switches: `CCC_NO_LEAF_PARAM_GPR`, `CCC_NO_EMPTY_LOCAL_FRAME_ELISION`.

Cmp flag fusion and load-cast fold live in `prologue.rs`; `cmp` must not be nohome. Remaining high-value gaps include `btq` classification, aggregate SROA, and broad SSE-class aggregate handling.
