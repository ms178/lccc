# x86-64 codegen

Text `ArchCodegen` + string peephole, and optional MachInst (off on large loops). Accumulators `%rax`/`xmm0` still dominate some FP/aggregate paths. FMA and YMM memcpy **emitters exist**. Gaps: SIB CRC, `andn`, `btq`, xmm field copies, assignment-as-ymm.

Cmp flag fusion and load-cast fold live in `prologue.rs`; `cmp` must not be nohome.
