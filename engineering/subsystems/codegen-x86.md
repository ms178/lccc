# x86-64 codegen

Text `ArchCodegen` + string peephole, with optional MachInst disabled on large loops. Scalar FP uses VEX three-operand staging (`xmm0`/`xmm1` scratch; never `movaps` for scalar moves); FP returns load directly into `xmm0` (`load_fp_to_xmm0`). Scalar and packed `vfmadd231*` emitters are production, gated on the FMA3 ISA feature; FP contraction follows `FpContract { Off, OnExpr, Fast }` with **Off** as the default (GCC `gnu*` parity).

Production improvements include masked-index SIB loads, BMI1-gated direct-return `andn`, exact 64-byte AVX2 assignment (six instructions: four YMM moves, `vzeroupper`, `ret`), compare-to-branch fusion on relational float Cmps, and setcc/test/cmov collapse with dominance-correct liveness. Safe leaf DCE removes only proven-unneeded parameter homes; 32/48-byte copies remain XMM after a measured YMM slowdown.

Call-free CFG leaves with leading ParamRefs can retain up to six SysV register arguments in caller/ABI homes. If layout consumed only the conservative callee-save collision reserve, the duplicate local frame is elided while push/pop CFA remains exact. Kill switches: `CCC_NO_LEAF_PARAM_GPR`, `CCC_NO_EMPTY_LOCAL_FRAME_ELISION`.

Cmp flag fusion and load-cast fold live in `prologue.rs`; `cmp` must not be nohome. Narrowing/same-size casts always emit extending moves (stale upper bits killed zlib-ng once); mixed legacy-SSE/VEX inside YMM loops re-triggers the 9× AVX-SSE transition penalty; loop rotation is opt-in (`CCC_LOOP_ROTATE=1`) and runs after vectorize. Remaining high-value gaps include aggregate SROA, broad SSE-class aggregate handling, and the `imul`-chain hash-multiply shape (expat).

Boot/`-m16` pipeline: GAS-faithful `.code16gcc` encodings, mcount/`-pg` family emission, per_cpu `leaq sym(%idx)` SIB folds, and the 32 KiB boot gate measured on pre-pecompat content end (alignment cliff at `.text` ≤ 23,380).

Peephole call-site liveness has ONE authority, `passes/liveness.rs::call_site_reads` (the `LCCC_CALL_ARGS` / `LCCC_VA_CALL` / `LCCC_CHAIN_CALL` marker protocol plus retpoline `%r10` and textual operands). `FileLiveness` and the windowed dead-register scans (`call_window_verdict`, used by `memory_fold::is_reg_dead_after` and `scratch_dead_after_relaxed`) both consult it: a call is transparent to callee-saved families, kills only caller-saved families it does not read, and keeps its argument registers live. The windowed scans used to answer "dead" for every register at any call, so `movslq %eax,%rbx; movq %rbx,%rdi; call malloc; … movq %rbx,%rdx` lost its only `%rbx` write (regression `movslq_relay_callee_saved_across_call.c`).

Return liveness reaches the text peephole the same way, as prologue marker comments that are authoritative when present (tail-block inference otherwise): `# LCCC_RET_XMM <mask>` (bit 0 `%xmm0`, bit 1 `%xmm1` read by `ret`), `# LCCC_RET_XMM_BITS <n>` (the widest read, in bits from bit 0, of those registers: 32/64 for scalar `float`/`double`, 128/256/512 for vectors, 512 when the width cannot be bounded; `fp_liveness.rs::ret_xmm_read_bits` lets a rewrite change bits above an element only where no `ret` reads them), `# LCCC_RET_RDX 0|1` and `# LCCC_RET_RAX 0|1` (`prologue.rs::emit_gp_return_liveness_markers`). They are compiler-internal protocol, not environment knobs.

## Incoming MEMORY parameter homes

x86-64 complete `StructStack` / `LargeStructStack` arguments can directly
home their parameter alloca in the incoming argument area. This is the callee's
by-value object, not a reference to the caller's source object. Mutation and
address escape therefore do not require another copy. Positive virtual-frame
slots remain stable across outgoing calls and dynamic stack realignment;
FPO uses entry RSP + 8, frame-pointer mode uses RBP + 16.

`IrParam.struct_align` describes ABI placement, whereas `param_align` can
request stronger local-object alignment. The alloca takes their maximum.
A typedef raising only a parameter object's alignment must not move the incoming
argument: GCC copies it to an aligned local home. Such a home is retained when
its alignment exceeds the incoming guarantee. Clang 23 ignores this stronger
typedef alignment on aggregate parameter objects; the ABI positions still agree.

`CCC_NO_INCOMING_BYVAL_HOME` retains the incoming-to-local capture copies for
bisection and A/B measurement. It is invocation-owned in `RaConfig`, not read
per function or per instruction. The runtime gate checks both switch states,
both frame modes, both optimization levels and all four compiler pairings.

## FP loop promotion

`promote_loop_invariant_fp_load` may replace only a full `movq %rax, slot`
preheader store and an unpinned `movsd slot, %xmm0` body load. Both participants
must be unpinned. A byte/word/dword store preserves the other slot bytes; it
cannot supply the eight-byte MOVSD value from `%rax`. This is distinct from
the register-destination rule that a dword GP write zeroes the upper half.
The slot-stability, single-entry and FP-liveness proofs remain mandatory.

Its lazy label index retains the first matching label and admits only labels
strictly preceding the jump, matching the old prefix scan even for malformed
duplicate names. The pass rewrites only instructions, never label text,
classification or positions, so those entries remain valid for its invocation.
Tests pin that invariant, forward/missing/deleted labels and one construction
across 256 jumps. Do not reuse this index in a pass that edits labels without
explicit invalidation.
