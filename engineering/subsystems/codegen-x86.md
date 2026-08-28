# x86-64 codegen

Text `ArchCodegen` + string peephole, with optional MachInst disabled on large loops. Scalar FP uses VEX three-operand staging (`xmm0`/`xmm1` scratch; never `movaps` for scalar moves); FP returns load directly into `xmm0` (`load_fp_to_xmm0`). Scalar and packed `vfmadd231*` emitters are production, gated on the FMA3 ISA feature; FP contraction follows `FpContract { Off, OnExpr, Fast }` with **Off** as the default (GCC `gnu*` parity).

Production improvements include masked-index SIB loads, BMI1-gated direct-return `andn`, exact 64-byte AVX2 assignment (six instructions: four YMM moves, `vzeroupper`, `ret`), compare-to-branch fusion on relational float Cmps, and setcc/test/cmov collapse with dominance-correct liveness. Safe leaf DCE removes only proven-unneeded parameter homes; 32/48-byte copies remain XMM after a measured YMM slowdown.

Call-free CFG leaves with leading ParamRefs can retain up to six SysV register arguments in caller/ABI homes. If layout consumed only the conservative callee-save collision reserve, the duplicate local frame is elided while push/pop CFA remains exact. Kill switches: `CCC_NO_LEAF_PARAM_GPR`, `CCC_NO_EMPTY_LOCAL_FRAME_ELISION`.

Cmp flag fusion and load-cast fold live in `prologue.rs`; `cmp` must not be nohome. Narrowing/same-size casts always emit extending moves (stale upper bits killed zlib-ng once); mixed legacy-SSE/VEX inside YMM loops re-triggers the 9× AVX-SSE transition penalty; loop rotation is opt-in (`CCC_LOOP_ROTATE=1`) and runs after vectorize. Remaining high-value gaps include aggregate SROA, broad SSE-class aggregate handling, and the `imul`-chain hash-multiply shape (expat).

Boot/`-m16` pipeline: GAS-faithful `.code16gcc` encodings, mcount/`-pg` family emission, per_cpu `leaq sym(%idx)` SIB folds, and the 32 KiB boot gate measured on pre-pecompat content end (alignment cliff at `.text` ≤ 23,380).
