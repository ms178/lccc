# x86-64 codegen

Text `ArchCodegen` + string peephole, with optional MachInst disabled on large loops. Scalar FP uses VEX three-operand staging (`xmm0`/`xmm1` scratch; never `movaps` for scalar moves); FP returns load directly into `xmm0` (`load_fp_to_xmm0`). Scalar and packed `vfmadd231*` emitters are production, gated on the FMA3 ISA feature; FP contraction follows `FpContract { Off, OnExpr, Fast }` with **Off** as the default (GCC `gnu*` parity).

Production improvements include masked-index SIB loads, BMI1-gated direct-return `andn`, exact 64-byte AVX2 assignment (six instructions: four YMM moves, `vzeroupper`, `ret`), compare-to-branch fusion on relational float Cmps, and setcc/test/cmov collapse with dominance-correct liveness. Safe leaf DCE removes only proven-unneeded parameter homes; 32/48-byte copies remain XMM after a measured YMM slowdown.

Call-free CFG leaves with leading ParamRefs can retain up to six SysV register arguments in caller/ABI homes. If layout consumed only the conservative callee-save collision reserve, the duplicate local frame is elided while push/pop CFA remains exact. Kill switches: `CCC_NO_LEAF_PARAM_GPR`, `CCC_NO_EMPTY_LOCAL_FRAME_ELISION`.

Cmp flag fusion and load-cast fold live in `prologue.rs`; `cmp` must not be nohome. Narrowing/same-size casts always emit extending moves (stale upper bits killed zlib-ng once); mixed legacy-SSE/VEX inside YMM loops re-triggers the 9× AVX-SSE transition penalty; loop rotation is opt-in (`CCC_LOOP_ROTATE=1`) and runs after vectorize. Remaining high-value gaps include aggregate SROA, broad SSE-class aggregate handling, and the `imul`-chain hash-multiply shape (expat).

Boot/`-m16` pipeline: GAS-faithful `.code16gcc` encodings, mcount/`-pg` family emission, per_cpu `leaq sym(%idx)` SIB folds, and the 32 KiB boot gate measured on pre-pecompat content end (alignment cliff at `.text` ≤ 23,380).

Peephole call-site liveness has ONE authority, `passes/liveness.rs::call_site_reads` (the `LCCC_CALL_ARGS` / `LCCC_VA_CALL` / `LCCC_CHAIN_CALL` marker protocol plus retpoline `%r10` and textual operands). `FileLiveness` and the windowed dead-register scans (`call_window_verdict`, used by `memory_fold::is_reg_dead_after` and `scratch_dead_after_relaxed`) both consult it: a call is transparent to callee-saved families, kills only caller-saved families it does not read, and keeps its argument registers live. The windowed scans used to answer "dead" for every register at any call, so `movslq %eax,%rbx; movq %rbx,%rdi; call malloc; … movq %rbx,%rdx` lost its only `%rbx` write (regression `movslq_relay_callee_saved_across_call.c`).

Return liveness reaches the text peephole the same way, as prologue marker comments that are authoritative when present (tail-block inference otherwise): `# LCCC_RET_XMM <mask>` (bit 0 `%xmm0`, bit 1 `%xmm1` read by `ret`), `# LCCC_RET_XMM_BITS <n>` (the widest read, in bits from bit 0, of those registers: 32/64 for scalar `float`/`double`, 128/256/512 for vectors, 512 when the width cannot be bounded; `fp_liveness.rs::ret_xmm_read_bits` lets a rewrite change bits above an element only where no `ret` reads them), `# LCCC_RET_RDX 0|1` and `# LCCC_RET_RAX 0|1` (`prologue.rs::emit_gp_return_liveness_markers`). They are compiler-internal protocol, not environment knobs.

## Sibling calls in branch arms and the tail-call marker

`backend::x86::ret_dup` (an IR pass after the PGO fingerprint) hands each
call arm of a shared return block its own `Return`, so `if (c) f(); else g();`
and `return c ? f(x) : g(x);` end in two sibling `jmp`s, as with GCC 16.2,
Clang 23.1.4 and ICX 2026.2. `CCC_DISABLE_PASSES=ret_dup` keeps the shared
return block and `CCC_NO_SIBCALL` disables sibling-call conversion entirely;
both serve bisection and A/B measurement. A function whose every call converts owes no
call-alignment pad (`prologue::every_call_is_sibling`). The pad is kept with
i128 helper calls, and codegen asserts the rule at every real call.

The codegen publishes each sibling call as `jmp sym` followed by the
`# LCCC_TAIL_CALL` marker and the suppressed call's own `LCCC_CALL_ARGS` /
`LCCC_CALL_FP` markers. Both liveness oracles read that block. A marked `jmp`
reads the call's arguments plus the callee-saved set and has no successor;
in particular it gets no `ret` live-out, so a dead `%rax` staging copy dies.
The flag-consumer walk ends at it exactly as at a `call`. An unmarked
`jmp` to a foreign symbol stays unknown. The peephole's terminal
`fold_conditional_tail_calls` turns `jcc .L` into `jcc sym` when `.L` holds
only a marked sibling `jmp`; this is Clang's form and one instruction shorter
than GCC's. The marker block moves behind the `jcc`, so the oracles model
the taken edge as that tail call.

`fold_load_mask_into_memory_test` folds a load plus `and`/`test` immediate,
whose register is dead afterwards, into `test $imm, mem`. It narrows to the
smallest byte window, matching GCC's `testb $3, G+2(%rip)`, only when every
flag consumer reads ZF; otherwise it keeps the flag-exact and-width form.

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


