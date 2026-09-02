# FOLLOWUP 2026-09-02e — MachInst typed direct calls (CallTyped) + xmm scratch relays

Base: `d1dba8d2` (origin/main, PR #354). Session snapshot: **S25**.
Deliverable: `/home/z/my-project/ms178-1.patch` (APPLIES-CLEAN, verified in a virgin clone at base).

## 1. What was built

### 1.1 `MachInst::CallTyped` — the SysV integer-register call as one atomic machine instruction

Before: every `Instruction::Call` was a census rejection (500 at -O2 on the regression
corpus — the largest class, 2.76% of all instructions) and the mature path staged every
argument through the rax accumulator: a caller-saved-homed argument cost two moves
(`movq %r11, %rax; movq %r11, %rsi`) where one suffices, and the dead staging move
survived even the peephole's source rewrite.

Now:
- **`CallTyped { caller_saves, args, target, ret }`** — caller-save spills, argument
  moves, the call, the return home, and the restores as ONE machine instruction with
  declared clobbers (the full SysV caller-clobber set) and declared uses, the way an
  MIR `CALL` or a Cranelift `call` models it. Atomic under the replay fallback: one IR
  instruction ↔ one MachInst, so a mid-run fallback re-emits through the mature path
  without double-emission hazards.
- **Pure builder** (`isel::build_typed_call`) — consumes pre-resolved argument homes,
  returns the execution-ordered move plan. Fully unit-testable without codegen state;
  the full admission/ordering matrix is pinned by tests (see §3).
- **Topological move ordering** — readers-before-writers per register (a reader must
  observe the home's original value, so it precedes the writer). The mature path's
  accumulator staging papers over exactly this ordering problem; the typed layer solves
  it and refuses register-exchange cycles to the text path (which owns the hazard-spill
  area machinery).
- **`xorl` zero arguments** — 3 bytes vs 7 for `movq $0`, the GCC/Clang/ICC form.
- **Canonical move widths** — narrow C arguments move at their promoted 32-bit width
  (same doctrine as `narrow_defined_load`); the return home names the sized `%eax`.
- **`AllocaAddr` arguments** (`f(&local, buf)`) — a separate pre-move
  (`Mov { AllocaAddr }` → resolver-rendered `leaq slot, %reg` with the frame's
  addressing mode) before the call; two guards keep it sound: no other argument may
  source from the register, and the register must not be in the caller-save set (the
  pre-move precedes the saves).
- **PLT/version naming** mirrors `emit_call_instruction_impl` (base name in `@PLT`
  references — GAS 2.47 rejects `sym@ver@PLT`).
- **Flushing discipline**: the preceding run is drained first, the call is buffered as
  its own run, and that run is flushed immediately. This reproduces the mature path's
  program-point semantics exactly (the caller-save interval check runs at the call's own
  point), keeps the pre-colored argument/return registers conflict-free (no allocated
  vreg can coexist with the call in the buffer), and its flush tail is precisely
  `clobber_after_call_like`.
- **Inline-memcpy precedence preserved**: the dispatch loop's fixed-size memcpy
  interception keeps winning (`inline_memcpy_len` made `pub(crate)`, single source of
  truth).
- **Kill switch**: `CCC_MI_DISABLE_KINDS=call-typed`.

### 1.2 xmm0/xmm1 pre-colored float scratch + slot-homed float relays

- **`XMM0_SCRATCH` (PhysReg 18) / `XMM1_SCRATCH` (19)** — below the allocator's xmm
  pool (20..=33), so no allocated float home can ever collide with the scratch.
- **Slot-homed float Store/Load relays**: a spill-slot-homed float value lowers as
  two FMovs through xmm0 (x86 has no mem-to-mem SSE move); the load is the mirror.
  D32/D64 carriers ride the same bit-exact SSE moves.
- **`xmm_scratch_unsafe` guard** — the hazard this session caught *before* the suite
  did: float parameters are pre-colored to their INCOMING registers, and xmm0/xmm1 ARE
  incoming registers (PhysReg 18/19). A live parameter home in the scratch refuses the
  relay to the text path. Exact check: the allocator never assigns 18/19, so any entry
  is a live incoming param.
- `reg_name`/`freg_name` know 18/19 (loudly — no fallback); the scratch FMov shapes
  joined the real-assembler differential corpus.

## 2. Evidence

| Check | Result |
|---|---|
| census `CCC_ISEL_STATS` (571-file corpus, -O2) | **Call rejections 500 → 28** (−94.4%); coverage 47.08% → 48.46%; every residual Call rejection labeled by reason (`arg-unrepresentable`, `imm-too-wide`, `move-cycle`, `ret-unrepresentable`) |
| unit `cargo test --lib` | **1558/0/6** (+15: builder matrix, golden stage order, xorl form, sized-rax ret, swap-cycle refusal, self-move elision, imm32 window edges, alloca-addr conflict guard, scratch-name table, relay shapes, relay liveness guard) |
| regression suite (amd64+ELF32) | **571/0/15, AB-diff 0** (with i686 runner env) |
| call-site asm (callsite.c) | dead `movq %r11, %rax` staging gone; `movl` canonical widths; arg-move counts **equal to GCC's** per call site |
| two-TU oracle (opaque callee, -O2) | lccc 29 vs gcc 23 instructions — arg sequences identical in count (6 vs 6); the residual delta is frame layout (`subq` reservation), the BinOp xor tail (accumulator staging, the documented R5 follow-up), and gcc's scheduling — all pass-level, not call-layer |
| in-TU oracle caveat | gcc's 16-instruction figure exploits IPA-RA (its own callee's register usage); not a call-sequence comparison |
| assembler differential | CallTyped corpus (6 shapes + void call + xmm scratch FMovs) accepted by the real assembler |

## 3. Test matrix (the "brilliant and exhaustive" list)

Builder (pure, no codegen state): readers-before-writers ordering; register-exchange
cycle refusal; self-move elision; imm32 window edges (i32::MIN/MAX ok, 2^32 refused);
alloca-addr conflict guard (accepted + refused shapes); rax/rbp defensive refusal;
return homes (reg / slot / rax-elision / rbp-refusal / void); full six-argument shape
with cross-register reader/writer constraints asserted positionally.

Emitter golden: full stage order (saves → args → call → ret → reversed restores);
`xorl` zero form; 32-bit return naming `%eax`.

Corpus: CallTyped (6 argument shapes × mixed widths + void call) and the xmm0/xmm1
scratch FMov shapes flow through the real-assembler differential, the determinism test,
and the no-vreg test.

Lowering integration: slot-homed float store relay (two FMovs, xmm0 pinned); float
load to slot-homed dest (mirror); the liveness-guard refusal when a live param occupies
xmm0.

## 4. Known traps re-verified this session

- Float parameters live at PhysReg 18/19 (incoming xmm0/xmm1) — "never allocatable"
  ≠ "never occupied". Any new scratch use MUST run `xmm_scratch_unsafe`.
- The sandbox's text tools mangle `[h` sequences and phantom-match whitespace regexes;
  rustc + python byte counts are the authorities (bit this session's `blocks[header]`
  phantom).
- Upstream PR #353's `loop_unroll.rs` editor-paste corruption (fixed in S24) is
  upstream now; the census script (`scripts/isel_census.py`) is the coverage oracle.

## 5. Follow-up backlog (data-driven, ranked by the fresh census)

1. **Memcpy (94)** — typed `Memcpy { dst, src, size }` with inline small-size
   expansion; needs declared scratch clobbers (rax/rcx are never homes — safe).
2. **Store(other) (79)** — residual shapes: GEP-folded ptrs and const stores.
3. **Float-const stores (bulk of Store(float) 123)** — needs a const→rodata-label
   pre-pass so the isel can emit `movss .LC(%rip), %xmm0` relays.
4. **VaStart/VaEnd (71 combined)** — small fixed instruction sequences.
5. **CallIndirect (30)** — the typed call with an indirect target (retpoline-aware).
6. **Alloca-addr args in caller-save registers (residual 28 Call)** — relax the
   pre-move/save ordering by emitting saves before the pre-moves (needs the saves to
   leave CallTyped's atomic scope, or a CallTyped pre-stage).
7. **BinOp accumulator tail** (`movl %r13d, %r10d; xorl %eax, %r10d; movl %r10d, %eax`
   → `xorl %ebx, %eax`) — the R5-documented algebraic-fold family; worth 2 instrs per
   consumed call result at -O2.
8. **Frame layout** — push-only framing when no spill slots exist (2 instrs/call-heavy
   function).
