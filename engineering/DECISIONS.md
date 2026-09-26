# Decision & negative-results ledger

**Purpose.** Every load-bearing lesson from the point-in-time session journals
that used to live in `docs/history/` (121 files, 2026-03…2026-08),
`updates/` (10 files), `engineering/agent/SESSION_FOLLOWUP_KERNEL_BOOT.md` (deleted),
`engineering/agent/RA23_COMPLETION.md` (deleted),
`docs/linker/FOLLOWUP_2026-08-17{,_SESSION2}.md`, `hotspots/`, and
`docs/simd-audit.md` — all deleted; session-day history lives in
[`journal/`](journal/README.md), full text in git. Nothing
here is a status snapshot. Each entry is a constraint, a measured negative, or
a root cause a future agent must not relearn the hard way. Entries cite the
source file they were distilled from.

**Read order / navigation** (this file is long — search or jump):

- §Register allocation · `## RA-*` / `## Session *` (RA-PRESSURE, valve,
  split, coalesce, GLA-01/02/03) — biggest section.
- §Optimizer (IR passes) · `## Session 10–15` — expression sinking, OP-42,
  PF-15, phi-acyclic, in-loop-use supply.
- §Codegen x86 · epilogue cross-jumping, MS-09, ROT-16, the peephole
  audits. §Vectorizer · §Frontend/sema · §Linker (lccc-ld) · §Multi-arch.
- §Process & environment — harness/toolchain rules.
- Appendix: §Status corrections (superseded public claims).
- Session-day narratives and per-day edit history:
  [`journal/2026-08.md`](journal/2026-08.md),
  [`journal/2026-09-W1.md`](journal/2026-09-W1.md),
  [`journal/2026-09-W2.md`](journal/2026-09-W2.md),
  [`journal/2026-09-W3.md`](journal/2026-09-W3.md).

This file is append-only in spirit: when a new session produces a measured
negative or a reverted experiment, add one bullet. When a lesson is fully
encoded in [`agent/RULES.md`](agent/RULES.md) as a hard rule, keep the bullet
anyway (the bullet carries the *why*).

---

## Register allocation

- **[history/session14]** REVERTED: within-32-bit Cast hazard relaxation
  (cast-of-phi results in `%edx`). m32 fuzz seeds 0/116 diverged: the select
  arm's Sub read stale `%edx` after an intermediate temp reused it. Correct
  only under `CCC_NO_COALESCE=1`. Fix belongs on allocator select-arm/phi
  handling, not the hazard whitelist. (Session-12 variant — Cast marked
  `%edx`-clean — also reverted: loop-carried value clobbered across a
  `URem`'s `xorl %edx,%edx; divl`.)
- **[history/session11-kernel-boot-memcmp-fold]** REVERTED (measured zero):
  per-value `%ecx` self-exemption. Phase 1 hands hot-loop values to
  callee-saved first; `%ecx`/`%edx` only see leftovers. The enabler is a
  caller-saved-first "Phase 0" rework, not exemptions.
- **[history/session20-kernel-boot-32k-gate]** Two REVERTED experiments:
  (1) `CCC_CALLER_SAVE_SPANNING=1` −193 B but **unsound on i686** (the i686
  prologue discards `caller_save_spans` — the "win" is the size of the
  missing save/restore); (2) leaf-param relax + ≤32-bit Cast-clean hazard
  scan +47/+97/+188 B (consumers staging through `%eax` pay a home→`%eax`
  relay that outweighs saved slot traffic). Home selection must be
  consumer-shape-aware.
- **[history/session8-grok-red-team]** Hole-aware call-spanning keyed on raw
  `liveness.segments` gave −188 B but miscompiled (`sqlite_yy_shift`).
  Soundness requirements: segment-boundary checks need **inclusive-left**
  (`seg.start <= cp < seg.end`); segment checks must key on the **merged
  coalesce-leader union**, not raw per-value segments; hole-packing needs
  splitting + move insertion (linear scan = one register per value_id).
- **[history/session21-lever-implementation]** Two unsound Lever-4 drafts:
  register-resident pointers DO stage `%ecx` (`emit_load_ptr_from_slot`
  copies reg→`%ecx` unconditionally; 300/900 fuzz failures); re-entering
  caller-saved pools already handed out by Phase 2 without a holder-overlap
  conflict filter aliased a loop counter with its bound. Re-entering a pool
  needs the same interference proof as the primary scan.
- **[history/session22-eax-alloc]** First `%eax`-homing draft homed the binop
  RHS in `%eax` and zeroed the accumulator; m32 fuzz seed 0 caught it →
  shipped with the `acc_first_uses` shape gate (BinOp/Cmp LHS, Store val,
  Cast/UnaryOp/Copy src, Load ptr, Phi incoming, Return/CondBranch operand).
  Also: a regparm sweep's "436 mismatches" were a harness bug (stack args
  driven into a regparm probe) — identical failures with the lever disabled
  proved it.
- **[history/session39]** Full RA-23 replacement vetoed: durable stack homes +
  deleting consumer-order legality regressed caller homes/frames; Tier-2
  still crashed huft/SQLite.
- **[history/session31]** Tier-2 graph coloring enabled before RA-23 →
  `huft_build_crash` + `sqlite_vdbe_peephole` crashes, 166/450 phi-CFG
  mismatches. Default-enable only after RA-23 stage B (session 40), kill
  switch `CCC_NO_TIER2_GRAPH`.
- **[history/session44]** Zero-future-use eviction search measured Adler +15 %
  and reverted — do not resurrect without the Adler/gzip oracles.
- **[history/session47-nrs]** Adler RA-03: eviction-policy A/B modes 2/4/5 all
  worse. The real fix is RA-06 live-range splitting / next-use allocation,
  **not another eviction knob**. Adler root cause (session 41): the reassoc
  pass expands `sum2 += sum1` ×8 into a closed form that materialises all
  eight byte temporaries simultaneously; GCC/ICX interleave a parallel
  prefix-sum (~26 insns, 0 stack). Fix = refuse the closed form above the
  GPR budget or emit the prefix tree.
- **[history/session32]** `CCC_SROA_COPYOUT=1` did not transform struct_copy
  and made `structs_bitfields` fail (exit 15) — enabling needs dominance +
  ABI return/copy reasoning.
- **[history/session23/24]** zlib-ng gz_reset SIGSEGV root cause:
  `compose_const_gep_folds` ran before `propagate_stable_aliases`, and
  const-addr folds accepted register bases that die across calls. Fix:
  re-compose after propagation; const-addr folds restricted to alloca bases.
- **[history/session25]** `folded_gep_values`/`gep_base_offset` were never
  cleared in `reset_for_function` — value IDs restart at 0 per function, so
  stale state from function A fired spurious rematerialisations in function
  B. Any per-function backend state must be reset.
- **[history/session26]** `operand_to_rax`'s no-home fallback silently
  fabricated `xorl %eax,%eax` — now a hard panic on x86-64. i686 keeps the
  legacy no-op (**i686 acc-cache audit remains open** before the hard gate
  can land there). Also: i128 compares must stay excluded from
  flag-fusion/cmp-replay (`emit_i128_cmp` has no fusion hooks).
- **[history/session16]** Slot-operand folding **in codegen** is unsound:
  deferred/coalesced values get slots lazily; the peephole may only fold
  after all memory operands are materialized.
- **[history/session50]** Pre-existing miscompile class: loop IV carried in a
  caller-saved register across a call. Reproducer
  `tests/gauntlet-repros/loop_iv_across_call.c`; fixed via window-local
  `phi_window_clobbers_caller_saved` (sessions 52/53). The window-local rule
  is deliberately narrower than a function-wide `live_across_any_call`
  (session 52: keep the source eligible so Phase 1 can home it).
- **[history/session53]** `fuse_compare_and_branch` accepted any
  `movzbl %al,%reg` as the setcc result and fused an unrelated later
  `testq`. Only `movzbl %al,%eax`/`movzbq %al,%rax` fuse; Cmp dests never
  enter the immediately-consumed nohome set.
- **[history/session20 S05]** Unary RMW classification gave
  `notl/negl/incl/decl %reg` `dest_reg = REG_NONE`, so copy aliases survived
  a unary RMW (`v0 = ~v0` returned old v0 at every -O level). Found by the
  `slot_rmw_differential.py` fuzzer — the m32 fuzz never covered unary ops.
- **[history/session7-aggfwd]** i686 F64/I64 pair load with the base in
  `%eax` emitted `movl (%eax),%eax; movl 4(%eax),%edx` — first load
  destroys the base. Order pairs so the base is written last. Also fixed:
  redundant_ext byte re-extension on upper32_zero; `movq→movl` narrowing
  gated only on "next insn uses 32-bit form" (fuzz 147/150 → 300/300);
  `emit_bit_test_reg_direct` never zero-extended above the `setc` byte.
- **[history/session15]** The i686 memory-operand fold must prove the loaded
  register dead: continue past conditional jumps, safe-stop at
  label/call/ret/directive/pure-write, **unsafe on any read** (explicit or
  implicit — `idivl` reads `%eax:%edx`).
- **[history/session17/18]** REJECTED: memory-dest/memory-source ALU as
  proposed (`operand_mem_ref` = bare `get_slot` — the codegen fold changes
  the read shape so store-forwarding eliminates the materializing store);
  `%ecx`/`%edx` scratch in `emit_mul_imm_into_reg` same-register cases.
  Mul LEA chains ×6/×7/×10 and magic div/rem rejected **for -Os** (size
  regressions). i686 const div/rem gated by `!optimize_for_size`; the magic
  sign-correction must use `sarl $31`, not `shrl $31` (every negative
  dividend came out one low — caught by per-function differential).
- **[history/session27]** SIB offset-producer skipping must stop **AT** the
  fold's index value (an unpeeled const-materialised offset is consumed at
  the access; skipping it fabricated a garbage index). Load→cmp-mem folding
  **outranks** SIB.
- **[history/session28]** Hot-loop Phase-1 threshold sweep: uc≥12/span≥10
  best (31,105 B); documented per-file `string.c +91` trade-off. Debug-gate
  naming trap: the x86-64 peephole escape hatch is `LCCC_NO_PEEPHOLE`, NOT
  `CCC_NO_PEEPHOLE` (that is the i686 gate).
- **[history/session46-machinst]** MachInst `lower_cast` emitted truncating
  moves for narrowing casts → stale upper bits consumed at 64-bit width
  (zlib-ng `zng_emit_dist` OOB). Every narrowing/same-size cast emits an
  **extending** move; the →U32 case uses `Movzx` because a plain `Mov` is
  elided as a self-move on a die-at-birth shared home.
- **[history/session35-o0]** At -O0 phi elimination leaves non-SSA multi-def
  Copy webs; all four backends pass empty register pools at O0. 125/200
  phi-CFG O0 mismatches → 600/600. Do not run the scan on multi-def IR
  (RULES #21).
- **[history/session46-vecreg]** vecreg admission returns
  `Option<usize>` (exact leading-vector-arg count), not a bool —
  shifts/shuffles/AES/CLMUL/blends/inserts have trailing scalar operands
  that must never get XMM homes. Generic 128-bit FP and EVEX families stay
  closed.
- **[history/session81-i686-slots]** 4-byte slots on i686 needed two fixes:
  phi elimination represents a U32 incoming as an **untyped Copy with an
  `IrConst::I64` container** (destructive second word corrupted adjacent
  slots), and a one-pass walk can meet the untyped backedge Copy before its
  typed producer (fixed-point width inference).
- **[history/session77-small-slots]** Width-partitioned slots needed three
  class fixes: MachInst slot-to-slot Copy relays were unconditional 64-bit;
  Tier-3 offsets 4-mod-8 could shift an 8-byte slot onto a 4-byte slot at
  finalize; Tier-3 greedy expiry is **definition-order-relative** (a
  size-descending reorder handed a live occupant's slot away). Latent F32
  spill used `movsd` not `movss`. Processing order is a soundness invariant.
- **[history/session75-liveness]** `%r10` (static chain) is liveness **family index 10**
  (not 11) in the call-reads set — a bit index, NOT the PhysReg id
  (PhysReg(11) = `%r10`); a shift's `%cl` count is a fixed operand
  renaming may not touch (coalescing renamed it → `shlq %sil,%r8` stopped
  assembling).
- **[history/session76-redteam-pf06]** PF-06 kill switch was **inverted**
  (setting the NO_ variable enabled the more dangerous fold). i686 SIB
  emitters ignored `disp`; AArch64 returned success without it;
  `sib_mem64_sym("foo",…,4)` emitted symbol `foo4` not `foo+4`;
  `fold_load_test_into_cmp` changed SF for unsigned/narrow-signed loads —
  fold only when every subsequent flag consumer is ZF-only, except
  signed-to-64 loads. v6 reverted the `add(iv,const)` SIB peel after 5
  failures; v7's real root cause: the check must key on
  `(iv_id, add_result_id)` — when the add's result feeds the Copy that
  redefines the IV, RA coalesces them into an in-place `add` that may be
  scheduled before the SIB load.
- **[history/session18-cross-backend-codesize]** hash_table timing spread is
  ±9 % on **identical binaries** on the shared VM — never trust single
  wall-clock runs; diff the asm.
- **[history/session87]** `pass_disabled()` matched by **substring**
  (`postinline` also disabled `inline`; `univsr` disabled `ivsr`) — the
  primary inliner never ran on -m16 -Os/-Oz. Per-pass re-measurement
  (`CCC_M16_KEEP`): ifconv +410, licm +367, gaddrcse +355, postinline +56 —
  all four m16 policy entries are individually correct; the policy stays.
- **[updates/session05]** ILP32 pitfall: `unsigned long` is **32-bit on
  i686**; the "u64 miscompile" scare was a test bug (use
  `unsigned long long`).
- **[updates/session07/08]** i686 peephole lessons: ungated value-text
  propagation is a trap (third strike, +327 B — only a zero-read
  census-gated form guarantees deletion); P17 select-diamond **polarity**
  (the branch already selects the else path; emitting the inverted mnemonic
  swaps arm values); P11 stays dormant on purpose (its fire perturbs the
  Phase-3.8 fixpoint, net +9 B); **return registers are census-invisible**
  (`%eax`/`%edx` observed by the caller at every `ret`; 64-bit-return
  functions carry `# lccc-i686-return-uses-edx`). Div/rem pair fusion: tail
  dest liveness must be extended to the head before interval derivation;
  store order matters (`%eax` result into a `%edx` home destroys the
  remainder).

## Optimizer (IR passes)

- **[history/session23/24]** GVN F32/F64 load CSE miscompiled
  `fp_die_at_birth` (CSE-created FP Copies perturb die-at-birth FP chain
  coalescing). FP stays out of CSE until copy-aware coalescing treats GVN
  `Copy{F64}` as chain-transparent. Agent B's `folded_index_uses` gate
  sniffed PhysReg ids `32..=55` — x86-64's XMM pool is 20..=33; gates must
  take an explicit per-backend parameter, never id ranges. Also Agent B's
  `redundant_loads` shipped with the store-invalidation predicate
  **inverted**. Volatile loads are never merged (C11 5.1.2.3).
  `quadratic_sr`'s "bit-identical including wraparound" claim is **false**
  (t=2^16: 32768 vs 2147516416) — legal only under signed-overflow-is-UB,
  divergent under `-fwrapv`.
- **[history/session35-gaddr / session36]** GlobalAddr CSE must run **before
  the first GVN**; late runs after structural inline and after iter-0
  unroll/vectorize. GVN's `ExprKey::GlobalAddr` must split on `must_mat`.
  Placement evidence: cold singletons stay branch-local (unconditional
  entry hoist = 0.98284 ratio loss); mutually-exclusive sibling merge
  refused; loop addresses → innermost containing preheader (0.75792); the
  site-local safeguard walks **derived** chains (Copy/same-size
  Cast/const-GEP/const-Add-Sub) back to the originating GlobalAddr;
  cross-block hoisting refused in multi-block functions containing
  intrinsics (hoisting miscompiled `rdtscp` ordering).
- **[history/session35-gaddr]** PR #161's entry-block hoist merged
  variable-index table bases into one long-lived web and regressed
  `movl sym(,%idx,4)` into shift/add/reload — variable-index GEP bases are
  a third, site-local GlobalAddr class.
- **[history/session61-unroll]** Complete-unroll's two handwritten
  substitution functions missed **25 distinct use positions** (Memcpy
  dest/src, all Atomic* fields, Va* ptr fields, SetReturnF*, Phi incomings,
  post-loop Return/CondBranch). First reproducer made `abort()`
  unconditional. Rule: cloning must use the canonical visitors
  (`for_each_operand_mut`/`for_each_value_use_mut`/terminator variant).
- **[history/session75-v3]** General complete unroller defects caught by
  differential: cloned terminators did not rename value uses (infinite
  loop); header phi→Copy took `incoming.first()` — take the **non-latch**
  incoming. The FP-aware expansion budget (256 vs 512) is **load-bearing**:
  unrolling nbody's advance fully yields 1183 insns/476 stack-refs vs
  594/159.
- **[history/session78]** REVERTED prototype: Select-guard conditional-sum
  vectorization emitted the **unguarded widening add** (differential caught
  immediately). Skeleton kept inert until the masked-add emitter existed.
- **[history/session80]** AVX-SSE transition penalty measured **9×**
  (318 ms vs 35 ms) when legacy SSE mixed with VEX inside/after a
  YMM-writing loop. All widening-loop instructions must be VEX
  three-operand forms.
- **[history/session47/48/49]** Reduction homes: `x86_fp_pool` tested only
  whether the first register was exactly PhysReg 20, so the XMM2 quarantine
  disabled the entire x86 SIMD allocator — the check must accept any first
  register in 20..=33. More than two accumulators stay rejected until a
  measured pressure/profitability model exists.
- **[history/session41]** Expat 1.90× is branch-prediction shaped on the
  weak VM predictor — instruction-count reduction alone may not close it.
  PIE `leaq sym(%rip)` remat inside loops is the CRC/longest_match defect;
  in PIC/PIE the SIB fold is impossible, so hoisting is correct *and*
  profitable (PF-07; the old "crc SIB fold" entry was **wrong** — x86-64
  cannot combine a RIP-relative displacement with an index register).
- **[history/session73]** Designed but NOT shipped: `fold_cmov_increment` —
  the emitted `cmovneq %r8,%rbx` preserves upper 32 bits of `%rbx` on the
  not-taken path while the replacement `addl` zero-extends; valid only if
  every definition of the cmov destination is a 32-bit write, which a
  textual peephole cannot establish locally.
- **[history/session74]** REJECTED external patches: `scalar_storage_order`
  bitfield support (storage unit never byte-swapped: `00 00 f8 aa` vs GCC
  `aa f8 00 00`; plain scalar members unhandled — accepting the attribute
  while laying memory out differently is worse than silent ignore; **the
  byte-swap remains open**). RA occupancy/segment-fill infrastructure
  (its own measurements showed miscompiles). Folding `0.0 + x` to a plain
  load is wrong for x = −0.0 (signbit observable; regression pins it).
- **[history/session63]** Aggregate-DSE fixes are intentionally conservative
  — do not re-open with a purely local read-set argument (pointer phis with
  partial tracking are escape boundaries). GVN unknown-pointer epoch
  (invalidate unknown-base entries on every store) is the right trade-off.
  LICM load hoisting has a must-execute requirement — if it loses perf, add
  dereferenceability analysis instead of relaxing the dominance guard.
- **[history/session64-backlog]** Do NOT route inline-candidate functions
  through split Return+SetReturnF64Second (the inliner merges only the
  Return operand — `__m128` wrappers lost lanes 2-3); do NOT forward stores
  through `Other`-root pointers; do NOT emit vzeroupper unconditionally
  (32/48-byte XMM policy measured slower); do NOT CSE loads from param
  allocas.
- **[history/session20-volatile]** Volatile semantics: mem2reg silently
  rebuilt loads with `volatile:false`; volatile gating required in
  store_load_forward, load_forward, gvn, dce, licm, loop_memory_promote,
  and the i686 load+cmp memfold. **Phase 2 open**: casts to
  volatile-qualified pointer types (MMIO idiom), volatile struct fields,
  `int * volatile p`, volatile aggregate initializers; asm-level
  dead-load-deletion audit. Single-call-site static inlining needs the
  address-not-taken escape scan to include inline-asm templates and
  `i`-constraint symbols; the zlib-ng Adler final-site inline was a
  regression (262-inst body; measured loop-nest exemption ceiling 160).
- **[history/session11-variadic / session12]** `_Float128`'s U128 carrier is
  ambiguous with `unsigned __int128` — const-cast folding must decline both
  directions. The inliner and switch-outliner must not clear
  `ret_is_f128_sse`/`struct_arg_is_f128_sse` when remapping CallInfo.
  FPO virtual frame base is `entry %rsp ≡ 8 (mod 16)`. Still open:
  caller-side >16-aligned stack args (GCC's dynamic `andq $-32,%rsp`)
  would break the compile-time `rsp_frame_size` FPO invariant.
- **[history/session65-nested]** Nested functions: do NOT emit trampoline
  jumps as rel32 (ASLR: stack→text exceeds ±2 GiB — shipped once; the
  24-byte `movabs; jmp *[rip+0]; .quad func` form is required); do NOT
  clear `dirty_upper_ymm` in `invalidate_vec_peephole`; do NOT key
  cross-function label references on BlockIds or (block-id→alias) maps; do
  NOT frame a captured variable twice (memoize by name). Non-local goto
  must restore the FULL frame state (rbp, rsp, rbx, r12–r15).
- **[history/session72]** `eliminate_redundant_leaq` must treat
  push/pop/leave/enter as `%rsp` writes — cached `leaq X(%rsp)` addresses
  go stale across stack adjustment. VLA-struct varargs are by-reference on
  the **variadic path only**.
- **[engineering/evidence/levkropp/backedge_pre_x86_spike.md (kept on
  main)]** The precise BLOCKED mechanism for FP backedge PRE:
  `detect_phi_coalesce_groups` rejects the loop-carried FP value because
  its def block mismatches the loop-form expectation — revisit only
  together with that coalescing path (mandelbrot A/B required).
- **[history/session65/66]** Backedge PRE: naive FP PRE regressed Mandelbrot
  (extra loop-carried FP phi/copy raised pressure; still regressed after
  phi-coalescing because the carried square is on the critical path).
  Policy: integer recurrences default-on (1.14×), singly-used FP top
  expressions disabled, multi-use FP enabled, `CCC_BEPRE_FP=1` research.
  A broader integer cross-block phi-coalesce variant was rejected by
  torture `20041011-1.c`.
- **[history/session58]** Disposition rules for the levkropp fork: treat
  every commit as broken until proven on the current RA; old RA patches
  must never be replayed over hole-aware liveness/segments; lev's runtime
  percentages on the shared VM are not accepted evidence; QEMU wall time is
  not hardware-performance evidence.
- **[history/session33-levkropp]** Do-not-resubmit from the fork:
  call-spanning F64 → callee-saved (SysV x86-64 has no callee-saved XMM);
  default-on full unroll; AArch64 RA-pool changes on a crashing baseline;
  the NEON `fmsub` path without an ICX nbody oracle; `8ef1978f`'s
  `gep_uses_iv(...,4)` hard-codes element size 4 (i64 maps mis-scale).
- **[history/session19]** The Ptr→float signedness bug lived in **three
  layers** (lowering elided the 8-byte cast; the MachInst gate's coarse
  size/sign classification; the register-direct path). Same bug class can
  live in three layers; fix all, pin with one script.

## Vectorizer

- **[docs/simd-audit.md (deleted 2026-08-13 audit)]** Six verified
  vectorizer/SIMD defect classes it closed (kept so nobody re-derives
  them): AVX2 tail-loop OOB reads; width-unaware SSE2 remainder handling;
  dot-product second-GEP scaling; matmul element-type gate; cast-kind
  gate; `vblendvps` implicit-mask operand bug. All are pinned by the
  `tests/regression/check_vector_remainder_codegen.sh` /
  `simd_permutevar_set1.c` / `pblendvb_implicit_mask.c` class of tests.
  Its strategic finding — hand-written intrinsics ran ≈5.3× slower purely
  from zero register retention across intrinsic boundaries — is the
  root-cause seed of the vector-temp-promotion rewrites (sessions 42–45).
- **[history/session7-aggfwd]** Expat classify: GCC's shape is
  `andl $-33` case-fold + range + `btq $mask` cluster; set_membership v2
  took `xml_name_continue` 45 → 28 vs GCC 20; the remaining 8 are
  phi-copy materialization. The exhaustive in-crate IR interpreter running
  0..255 equivalence is the validation pattern for classifier rewrites.
- **[history/session45]** Vector temp promotion invariants: `Loadldi128`
  reads 64 bits and zeroes the upper half (not forwardable as a full
  128-bit load); `Load256`/`Store256` 32-byte alignment contracts must not
  be erased; FMA accumulator forms read+write `dest_ptr` (promotion
  requires `intrinsic_overwrites_full_result`); constant-address load
  sources can be clobbered by any write not provably alloca-confined; the
  pointer-root fixpoint needs SCC decomposition to solve
  `p = phi(param, p+stride)`. GLM-style reports with 14700KF PMU numbers
  and no commands/raw samples are not evidence. An intermediate "static
  improvement" was rejected when Callgrind showed +897 dynamic
  instructions.
- **[history/session42/43]** Promotion defect classes: write-through-the-
  loaded-slot must invalidate the forward; `VecStoreI64x2` is the
  memory-form store with `dest_ptr: None` and is a full forwarding barrier;
  `VecLoadF64x4(base, offset)` forwarded base alone, dropping the offset;
  `index == 0` is the vector argument, index 1 is the byte offset; root
  propagation through `Add`/`Sub` when exactly one operand is rooted.
- **[history/session76]** Calibration: `gcc -fno-tree-vectorize` nbody ≈
  vectorized GCC — GCC's 4× edge is **scalar code quality**, not
  vectorization; lccc nbody output is bit-identical to `gcc -O0` (the FP
  gold standard) at 500k and 5M steps. The memory-source VEX fold is valid
  for **commutative ops only** (AT&T's first source position is Intel
  src2). Widening traps: 256-bit loads double-count lanes 4..7 (IV
  advances 4); `vextracti128` cannot extract dword lanes 2..3 after a
  128-bit store — `vpunpckhqdq` is the correct mover.
- **[history/session75/76/77/78/80]** Root-caused remaining vectorizer gaps
  (do not re-derive): spectral_norm (non-reduction FP / computed-invariant
  dot), nbody (multi-store scatter + marching-pointer slot-homing — 151 of
  nbody's 159 stack refs), adler32 (RA-06), expat (hash-multiply imul
  chains), find_bit (branchy ffs tree), fannkuch (vpshufd rotation),
  mandelbrot (escape-loop FP chain), loop_patterns residual (LCG init
  recurrence, widening int dot = `vpmuldq` class).

## Codegen x86

- **[history/session71/74]** FP binop dest-aliasing miscompile (root cause,
  kept): `emit_float_binop_into_reg` materialised `+0.0` with
  `xorpd %dest,%dest` *after* a multiply had been coalesced into that same
  XMM, and three RHS materialisations wrote into the register holding the
  LHS (`x + 1.5` computed `1.5 + 1.5`). Fix: 3-operand VEX staging with
  `xmm0`/`xmm1` as safe scratch; dest-holds-rhs for sub/div stages the LHS
  in `xmm1`. Never `movaps` for scalar FP moves (width/domain rule).
- **[history/session85]** Corrected rationales: `LCCC_BEST_EFFORT_NO_HOME`
  REJECTED (fabricated zeros hide RA bugs). `MIN_JUMP_TABLE_CASES` 4→5 is
  GCC-x86 parity + branch-prediction economics (the objtool rationale was
  false — GCC's own 5-case tables pass objtool daily); the constant is
  shared by i686/ARM/RISC-V where GCC's generic default is 4 (documented
  benign deviation). The `parse_sym_addend`-first probe applied
  unconditionally regressed `kernel_altinstr_layout`
  (`.long 760b - 770b + 5` mis-parsed); gated on
  `rhs_full.starts_with('(')` and rejects digit-only labels. Static_call
  `.long func - (. + 4)` once emitted a SILENT ZERO (worst failure class:
  misassembly, no diagnostic); GAS reference `R_X86_64_PC32 target_fn-4`.
  mcount ABI: classic `-pg` emits `call mcount` **after** frame setup and
  GCC hard-errors on `-pg -fomit-frame-pointer`; `-mfentry` calls
  `__fentry__` first; `-mrecord-mcount`/`-mnop-mcount` add
  `.section __mcount_loc` + `.quad 1b` + 5-byte NOP `0f 1f 44 00 00`;
  `-pg` is the trigger, sub-modes inert without it.
- **[history/session83]** `load_dest_reg` width contract: I8/U8/I16/U16
  loads paired with `movsbq/movzbl/movswq/movzwl` need **64/32-bit
  destinations**, not `%al`/`%ax` (GNU as rejects `movzbl %sil,%al`). GNU
  C treats char-pointee types as assignment-compatible (`-Wpointer-sign`
  warning, never a hard error even under
  `-Werror=incompatible-pointer-types` — the kernel relies on it).
- **[history/session41]** Text-peephole parsing: split operands at the
  **last** comma (AT&T destination last) — first-comma split silently
  disabled zero-extend tracking after every SIB load; an opaque SIB
  `movq (base,idx),%rax` must clear the stale byte fact for `%rax`. IMUL
  immediate: 32-bit form accepts any imm32 bit pattern; 64-bit form
  restricted to signed i32. BT's index is an ordinary r/m operand (the
  "%rcx fixed" comment belongs to variable shifts only). 32-bit ALU writes
  are upper-32-zero; `cmpl`/`testl` deliberately excluded (flags-only).
- **[history/session37/38]** The session-37 x86 text bit-test peephole was
  removed when `IrBinOp::BitTest` became a first-class IR op. Residuals:
  simplify built I64 BitTests on 32-bit targets → ICE (span cap 31 on
  -m32); the BitTest accumulator fallback still stages mask+rcx — audit
  its `movzbq` too.
- **[history/session44]** `gen_lcccsimd.py` declared every imm-taking
  builtin one parameter too many (trailing `int __imm`), rejecting every
  `_mm_shuffle_ps`-style call — fixed in the **generator** (52 proto
  sites). Three regression "failures" were runner-acceptance bugs (`.env`
  opt-outs): GCC 14 evaluates `__has_attribute` only in `#if`;
  `builtin_cpu_supports_raptor` folds against the fixed allowlist by
  design; `fp_domain_crossing` differs by ulps (legal reassociation).
- **[history/session59]** TCE must refuse self-tail-calls when a
  stack-derived pointer is passed — distinct recursive automatic objects
  would become one reused object. Store-only aggregate forwarding
  candidates must contain at least one write attributable by
  `pointer_paths`.
- **[history/session20 S03]** A missing `#include` in `.S` input is now
  **fatal** (GCC parity): header.S previously preprocessed against an
  EMPTY voffset.h and silently evaluated `VO_*` guards to 0. mkcapflags.sh
  argument order is `OUT cpufeatures vmxfeatures`.
- **[history/session86 / updates/session05/06]** The 32 KiB boot-gate
  regression root cause: the GAS-faithful -m16 correctness fixes in
  `e12597c7` each grew encodings ~1.5 KiB and nobody re-ran the gate;
  session 82's PASS used the old, incorrect encodings. Do NOT revert the
  encodings or disable VIDEO/EDD/serial. `CCC_M16_FULL_PIPELINE=1` inlines
  rdfs8 correctly but grows cmdline.o 571→713 — the m16 no-inline policy is
  correct until RA keeps loop values in caller-saved regs across call-free
  bodies. The gate is an **alignment cliff**: `.pecompat` requires 4096
  alignment; gate passes iff pre-pecompat content end ≤ 24,576 ⇔
  `.text` ≤ ~23,380–23,384 (session06 computed 23,380; session08's
  refreshed math recorded the cliff as 23,384 with PASS at 23,378 —
  use the CI metric, pre-pecompat content end, not the `.text` proxy) — **use pre-pecompat content end as the CI metric**,
  not `_end` (which jumps in 4 KiB steps).
- **[updates/session09]** `movl $symbol,label` → `R_386_32` (pio_ops fn-ptr
  tables) unblocked boot main.c.

## Frontend / sema

- **[history/session47-nrs]** Do NOT ship a naive `is_const`
  modifiable-lvalue check: the parser folds all declarator qualifiers into
  one flag (`const int *p` vs `int *const p` indistinguishable). Needs
  per-pointer-level qualifier tracking (same infra as FE-02/TBAA).
- **[history/session46]** Sema accepts writes through `const` arrays that
  GCC rejects — the C11 6.5.16.2 assignment-to-const-object diagnostic is
  still missing.
- **[history/session62]** Do NOT disable `-fbuiltin` to fix `llabs`
  overrides; do NOT fold `fabs(x) >= 0` (false for NaN); do NOT fold
  NaN/Inf float-to-int (IEEE invalid — saturate finite out-of-range only);
  do NOT treat pointer-to-VLA as a VLA for sizeof. `sizeof` of a
  struct-with-VLA is evaluated at the **type definition**.
- **[history/session33]** The shared FP-home selector identified x86-64
  solely from `PhysReg(1)` + pointer width — RISC-V also has
  `s1 == PhysReg(1)`. Requiring x86's `%r10` marker (**PhysReg(11)** — PhysReg(10) is `%r11`) prevents
  cross-target register-class leakage.
- **[history/session20]** i686 compare-with-memory fold treated
  `movzbl/movsbl/movzwl/movswl` as 4-byte loads because their **dest
  register** is 32-bit → `structs_bitfields` union check compared the whole
  word. Both fold sites require a genuine `movl`.
- **[history/session65-lev]** `p += i` / `p -= i` scaled the index without
  widening to pointer width — a negative index multiplied as a bogus 64-bit
  positive. Fixed in `expr_assign.rs`; regression
  `pointer_compound_assign_signed_index.c`.
- **[history/session52]** `&x` is the alloca, not `GEP x,0` — GEP+0 poisons
  mem2reg after inlining; Load/Store through GEP+0 fold to the base; extra
  mem2reg after post-inline fold2. `int (*cb)(void*, T*)` parameters typed
  via `fptr_params` (unblocks SQLite).

## Linker & assembler (lccc-ld)

- **[linker/sessions-2026-08 (doc folded) §0]** Stale **release** oracles produced two
  false failures (wild 0.7.0 blamed lccc; wild-git agreed lccc was right).
  Always build mold/wild from git before drawing conclusions. mold CMake
  pins: `-DMOLD_TARGETS='X86_64;I386'`, `-DMOLD_USE_MIMALLOC=OFF`,
  `-DMOLD_LTO=OFF`. Never compare Callgrind Ir across build profiles.
- **[linker/sessions-2026-08 (doc folded) §2]** Measured negatives: symtab **index
  sort** worse (57.8M vs 53.7M Ir); pre-sizing the FDE Vec with
  `count_eh_frame_fdes` 1.7 % slower; `push_strtab_name` as a tidy
  `extend(...)` iterator 5.8 % worse (defeats the vectorised copy — the
  reason that function carries `unsafe`); `Vec<u8> → Arc<[u8]>` **always
  copies** (removing the last whole-file copy needs mmap); the first
  zero-copy `SectionData` was 1.4M Ir *slower* because Deref re-derives
  bounds-checked subslices — bind `as_slice()` once. Removing a copy is not
  automatically a win.
- **[linker/sessions-2026-08 (doc folded) §2.2]** `parallel_reloc.rs` shipped real UB
  once: its doc claimed sorted-by-offset disjointness nothing established,
  and the bounds check was a `debug_assert!` (out-of-bounds **write** in
  release). Rewritten with high-water-mark partitions + checked writes; a
  randomised differential test caught an ordering bug **in the fix itself**
  (a stable sort only protects equal offsets).
- **[linker/sessions-2026-08 (doc folded) §2.3/§4.6]** Congruent segment packing
  (−59.5 % binaries): `p_offset ≡ p_vaddr (mod p_align)` is all the gABI
  requires. RELRO's boundary is an **address** boundary. `emit_shared.rs`
  had the identical defect (separate layout implementations drift
  silently). CORRECTION on record: the session-1 claim that
  `emit_script.rs` had the page-padding defect was **wrong** — an
  unverified claim in a follow-up doc is worse than no claim.
- **[linker/sessions-2026-08 (doc folded) §2.10+]** Linker correctness rules with named
  traps: `--exclude-libs` must suppress the PLT too (else `r_info` index 0
  → "undefined symbol: <empty>"); "the first matching arm wins — a
  duplicate arm added later is silently dead"; "a comment asserting
  something is handled elsewhere becomes a lie the moment elsewhere is
  deleted"; differential tests must compare **exit codes**; testing with
  library types instead of user-defined ones exercises a different linker
  path; ICF byte-only identity is unsound (`e8 00000000 c3` pairs differing
  only in a relative PLT32 reloc); `--icf=safe` must scan absolute
  relocations across the **whole link**; ICF does almost nothing for C —
  it is a C++ lever; `.gnu.hash` bucket clamp to 64 made average lookup
  52× longer at 5000 exports; "a correct value is not a working feature"
  (TLS TPOFF32 with no PT_TLS); `SIZEOF_HEADERS` undercount by one
  synthesized program header → SIGILL (derived quantities must have one
  source); DTPOFF after LD→LE relaxation is **TP-relative** ("relaxation
  changes what a register holds" — links cleanly, computes garbage);
  symbol-level checks cannot see body-level duplication (COMDAT dedup
  missing from the exe path forever while every symbol tool agreed) —
  search for raw byte patterns; `--as-needed` was reset by a second
  GROUP-script handler (`libm.so` is exactly such a script); validate a
  fuzzer against a planted bug; "a clean `git apply --check` is not
  integration proof — build it."
- **[linker/sessions-2026-08 (doc folded) §13]** The mmap work shipped a
  `private_interfaces` warning for four sessions because the gate was
  `cargo build 2>&1 | grep -E "^error"` — blind to warnings by construction.
  Fix the gate, not the instance: `build_lccc_fast.sh` now exports
  `RUSTFLAGS=-D warnings` (`LCCC_ALLOW_WARNINGS=1` to opt out).
  Mutation-verify the gate.
- **[linker/sessions-2026-08 (doc folded) §14]** DSE deleted the stack-top write of an
  inline **retpoline** (`movq %target,(%rsp); ret`) → infinite
  `pause; lfence; jmp` spin. DSE must recognize a stack-top store
  immediately consumed by `ret` or `pop`. `movsbl`/`movswl` must not be
  identified as implicit `movs` string instructions by mnemonic prefix.
  The broad i686 caller-saved ParamRef experiment was rejected (+75 B
  string.o, no strlen gain; needs real parallel-move handling incl.
  `%edx↔%ecx` cycles).
- **[history/session84]** Thin-archive `/NNN` refs: GNU ar thin rewrites can
  leave `/4611          /` (space-padded *after* the slash); BFD reads via
  plain strtol prefix semantics — do not "normalize" the archive. The ELF
  writer **silently writes symbol index 0** for any relocation whose name
  it cannot find — emit a write-time diagnostic (this cost hours);
  `identical_blocks` must scan both `.long` and `.quad` jump-table targets,
  register short blocks, and key function identity on global labels
  (CFI-keyed grouping merged blocks **across functions** when
  `-fno-asynchronous-unwind-tables`). `git am` fails by design on the
  deliverable — use `git apply`. Cargo trap: `--config
  target.<triple>.rustflags=[]` does **not** override `.cargo/config.toml`
  rustflags (cargo concatenates) — use the `RUSTFLAGS` env var. Snapshot
  immediately after every validation; move `artifacts/.base_ref` on every
  rebase.

## Multi-arch backends (AArch64 / RISC-V / cross-target)

- **[history/session21/33]** AArch64 PhysReg-class panic (`invalid ARM
  register index` — FP-pool PhysReg on a GP binop when float+double arrays
  and union bitcasts mix) blocked the levkropp AArch64 quartet — do not
  graft RA-pool changes onto a crashing baseline.
- **[history/session60/58]** A general x0-x3 AArch64 allocator is **not yet
  safe**: current lowering also uses x0/x1 as implicit accumulators/staging
  registers. The shipped solution is a strict fixed-register plan for the
  machine-combined conditional-increment leaf family; `select_pressure`
  needs machine-IR scratch-clobber modeling + fixed-register interference.
- **[history/session67]** The pre-existing "ARM regression failures" were
  **host-header dependencies** (`bits/libc-header-start.h`). Structural ARM
  regressions must stay freestanding (local typedefs +
  `extern int printf(...)`) so backend failures aren't masked by host
  headers.
- **[history/session68/70]** ARM traps: F128 parameter spill calls
  `__trunctfdf2` and clobbers x0 before a later GP ParamRef assigned to x19
  is materialized (x19 is an allocatable callee-saved home; save x0
  first); same-instruction same-register ALU operands reload one operand
  from its stack home; subword compares must explicitly sign/zero-extend
  before `cmp`; AArch64 static chain is **x18**; VLA aggregates in varargs
  pass by reference. Narrow unsigned compare constants mis-selected as CMN:
  normalize every integer constant per comparison type before CMP/CMN
  selection. Unassigned 32-bit Select emitted mixed-width `csel x0, w23,
  w28` — choose `w0` vs `x0` per arm width.
- **[history/session59]** Harness facts: the AArch64 suite refuses any
  assembler whose version string is not 2.47 (local wrapper delegating to
  the real `as` is the documented workaround); `codegen_oracle.py` had a
  same-temp-filename race between concurrent requests (mkstemp per
  request).
- **[updates/session09]** RISC-V va_arg `struct{long double}` needed **three**
  coordinated fixes (caller per-arg padding from the previously-ignored
  `struct_arg_aligns`; padding-aware `compute_stack_arg_space` — 8+16+8+16
  needs 64 B, the old math reserved 48; named-param layout + va_start base
  derived from ParamClass offsets). ARM got the identical caller padding
  and honors `align` (AAPCS64 `__gr_offs`). The AArch64 MOVW relocation
  numbers were dead-wrong (the ABI interleaves non-NC forms, G0=263…
  SABS_G2=272; there is **no** G3_NC/SABS_G3). CASP even-start/
  consecutive/uniform-width validation is architectural (the encoding has
  no fields for Xs+1/Xt+1).

## PGO / FP contract

- **[hotspots/integrated/pgo-regressions-eliminated.md (deleted)]** The
  PGO regression eliminations: flat-profile inliner gate is
  `summary::has_spread()` (profile only informative when one function
  uniquely dominates the runner-up); cost-aware devirtualization leaves
  ≥95 % single-target sites indirect (`CCC_PGO_PROMOTE_STABLE=95`;
  promoting them measured +28 %); measured outcomes adler 0.827→0.997,
  expat 0.718→1.014 (PGO became ≥1.0× everywhere on the workload set).
- **[STATE.md / history/session33]** PGO/feedback **block layout must not
  reorder hot loops** — it regressed expat 131→248 ms. The active
  vectorizer applies exact profile profitability per natural loop (trip <8
  vetoed; >80-instruction bodies need ≥32 trips; absent profile preserves
  static policy). A PGO gate must connect at per-loop granularity.
- **[history/session65-openai / session74]** FP-contraction history
  (load-bearing for numerics): default `fp_contract_fast=true` at plain -O2
  was a **conformance bug** and caused the nbody divergence (59.05 vs GCC
  −0.17) by fusing across SSA temporaries that are separate C statements.
  Default is now `FpContract::Off`; `-ffast-math` enables Fast; the
  tri-state `FpContract { Off, OnExpr, Fast }` with frontend
  `fp_expr_tags` is the proper model; FMA emission additionally requires
  the FMA3 ISA feature — contracting on baseline SSE2 targets is SIGILL on
  pre-Haswell hardware. Do not attempt a clean-slate SGSA allocator
  rewrite; incremental RA-05a→RA-06a is strictly safer.

## Process & environment

- **[current, Rust-2024 migration]** Environment protocol (harness wipes
  everything between sessions): swap first (`ensure_swap.sh`; some
  sandboxes cannot swapon — no `CAP_SYS_ADMIN`); `rust-toolchain.toml`
  follows the current `stable` channel — never pinning a specific version —
  while `Cargo.toml` records the tested compatibility floor
  (`rust-version = 1.98.1`); stable-only regressions are fixed at the source
  instead of freezing the toolchain.  Maintenance/build scripts source
  `scripts/rust_toolchain.sh`, so the manifest is the sole channel selector;
  explicit `LCCC_RUST_TOOLCHAIN`/`RUSTUP_TOOLCHAIN` overrides remain
  available for a deliberate bisection or historical reproduction.  Restore
  scripts must install the `rustfmt` and `clippy` components too. Install Rust
  under a persisted path; apt `gcc-multilib libc6-dev-i386` (m32 tests
  "cannot execute" ENOENT if absent — environment, not code),
  bison/flex/bc/cpio/libelf-dev/kmod; the kernel tree + 26 CachyMod patches is
  NOT persisted (re-extract each session); `/tmp` is wiped without warning
  (the LK-26 reproducers `/tmp/tlsinit.i` + `/tmp/offprobe.c` are gone — regen
  via the recorded `/tmp/tlspp.sh` recipe or reconstruct from `git show
  afa22485`).
- **[history/session84 + kernel-boot P0s, §below]** Build recipe
  for mold-less sandboxes: `.cargo/config.toml` pins
  `clang -fuse-ld=mold`; when absent pass
  `CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=gcc` **and** `RUSTFLAGS` in
  the environment (`--config rustflags=[]` does NOT override the file).
  Prebuilt static mold 2.42.0 user-local + `RUSTFLAGS="-C
  link-arg=-fuse-ld=mold"` brings cold builds to ~3 m 10 s.
- **[history/session86 / agent/CARGO_CONFIG_GCC_BFD]** **Supersedes the
  session84 recipe above.** `.cargo/config.toml` now defaults to
  `linker = "gcc"` (GNU ld/bfd) for both x86_64-unknown-linux-gnu and
  i686-unknown-linux-gnu (the latter keeps `-m32`). Rationale: neither
  clang nor mold is installed on the current research VM, so the old pin
  forced every build either to fail or to rely on per-invocation
  environment overrides — a standing footgun. The reference recipe is now
  simply `scripts/build_lccc_o1_j2.sh` (release profile, Rust `-O1`, `-j 2`,
  foreground, gcc/bfd) with no environment overrides needed. mold/wild
  remain available as **differential test oracles only**
  (`tests/linker/setup_oracles.sh`, git-HEAD builds) — they are build
  linkers nowhere.
- **[history/session84/10]** Snapshot discipline: commit + regenerate the
  `ms178-N.patch` + artifact + bundle immediately after every validated
  fix (session 84 lost a fix to a mid-session wipe and re-implemented it);
  verify the patch applies to a pristine origin/main worktree, not just the
  current tree. After any tree-wide `git checkout`, re-verify the
  reproducer sweep (a checkout once silently reverted committed fixes).
- **[history/session24 §4 / session63/64]** Pinned oracles/workloads:
  Compiler Explorer GCC 16.2 (`cg162`), Clang 22.1 (`cclang2210`), ICC
  2021.10, ICX latest; mold/wild **git-HEAD only** (release tarballs
  produced false failures); binutils/GAS 2.47; `ORACLE_REVISIONS.txt`
  records resolved revisions. Workload pins: gzip 1.14 (SHA `01a7b881…`,
  30/30 `make check` — run serially with one retry; use a disk-backed
  TMPDIR, the /tmp tmpfs caused false failures), zlib-ng 2.3.3 (69/69
  CTest), expat 2.7.1/2.8.2, SQLite 3.53.4 (recipe digest mismatch
  recorded; lemon patch needs the 3.53.4-specific variant), glibc 2.44
  (SHA `37f600f2…`, `ms178-glibc.patch` `52c9953f…`) with capability gates
  that must not be removed without evidence: `--disable-multi-arch` (no
  IFUNC yet), `--disable-mathvec` (no EVEX assembler),
  `--disable-sframe`, `--enable-kernel=3.2`.
- **[history/session26 §post-mortem]** Process rules: auto-save in the SAME
  command chain as validation; no bisection on unverified builds (`make
  CC=gcc X.o` silently skips rebuilds via timestamps; `|| true` on compile
  steps is forbidden); right tools over guessing (`CCC_DUMP_EACH_PASS`,
  targeted backend probes, gdb watchpoints); **minimal deterministic repro
  before touching the workload**. Callgrind is the deterministic
  PMU-independent metric (bit-identical across runs; single-threaded
  algorithmic work only; always reported next to best-of-N wall).
- **[linker/sessions-2026-08 (doc folded) §6]** Testing rules that paid off: fuzz
  before claiming robustness; write the randomised differential test
  before trusting a concurrency fix; **mutation-verify new tests** ("a
  test that passes under the bug it was written for is worse than no
  test"); flaky tests are worse than failing ones (PID-keyed temp dirs).
- **[history/session6/7/47]** Re-base checklist: the pinned reproducer set
  (spectral_norm -m32 -O2 must print 1.274224152; `f(191)==0` for the byte
  re-extension bug; fuzz seeds 0:150 at O1/O2 = 300/300; regparm
  conformance) must be re-run after every re-base because upstream merges
  of agent patches may take the PRE-audit version (five audit fixes once
  had to be re-applied after a PR merged them un-audited).

## Status corrections discovered during the v3 documentation audit

These supersede stale statements that existed in deleted documents or in
stale sections of live ones:

- **LK-26 (struct pthread `robust_head` 736/784/1296 / NULL-page store) is
  FIXED** — commit `afa22485` (2026-08-22). Root cause was NOT layout
  divergence: `ir/lowering` derived the address space of array-subscript
  lvalues differently from the load path, so every pointer-typed
  `THREAD_SETMEM` store lost its `%fs` segment prefix and wrote to the flat
  NULL page. Files: `src/ir/lowering/lvalue.rs`,
  `src/ir/lowering/expr_access.rs` (both carry `— LK-26` comments).
  Supporting commits: `e290be59` (direct `%fs/%gs` const-offset access),
  `c8b80fe7` (segment-store operand clobber, cited LK-24 in
  `memory.rs:2993`). Pinned by `tests/regression/segfs_pointer_declarator_matrix.c`.
- **LK-24 (externally-compiled PIE segfaults at startup against lccc-glibc)
  is still OPEN end-to-end**: the 2026-08-22 sub-bugs are fixed
  (`c8b80fe7` ld.so ET_DYN `e_entry`, verdef `vd_next` chain,
  segment-prefixed store clobber; `afa22485` versym for plain defined
  symbols; `symbols.rs:132` ld.so self-relocation), but the
  custom-interpreter smoke still SIGSEGVs (139) before output — recorded as
  a hard P0 gate in session 63/64: "do not weaken the smoke gate."
- **MS-11 (glibc `make check` triage harness) never materialised** — it was
  promised in session 56 and vanished; it is re-queued in
  [`tasks/`](tasks/README.md).
- **Loop rotation status**: `src/passes/loop_rotate.rs` is **opt-in**
  (`CCC_LOOP_ROTATE=1`; kill switch `CCC_NO_LOOP_ROTATE=1`; debug
  `CCC_DEBUG_LOOP_ROTATE`). The v16 default-enable produced 16 miscompiles
  (15 remaining after the v17 cross-phi latch-incoming rewrite fixed
  `fib`): vectorize_sse2_path, vectorize_reduction_dyn, simd_crc_adler,
  simd_vecreg, backedge_pre_*, bitops_builtins, adler_inline_tail,
  aggregate_dse_soundness, alloca_bare_builtin, alu_peepholes,
  arm_vec_load_offset, huft_build_crash, loop_promote_affine_alias,
  stmt_expr_asm_typeof, vectorize_iv_dependent_base. Root-cause these
  before flipping the default (queued as TASK-PF-17).
- **Kernel-boot P0 dispositions** (from the deleted
  the kernel-boot P0 list in [`journal/2026-08.md`](journal/2026-08.md) + session 85): the
  `emit_int_binop` Add with slot-spilled GlobalAddr base
  (workqueue_prepare_cpu ICE class) is **FIXED** — session 85 integrated
  the per_cpu_ptr-style `Add(Cast(GlobalAddr), reg)` → SIB
  `leaq sym(reg)` fold; the kernel-boot session's 4 bugs
  (incompatible-pointer-types cast suppression, `enclu/encls/enclv`,
  per_cpu GEP `leaq sym(,%idx,scale)`, `@file` response files) are all
  landed. `-pg`/`-mfentry`/`-mrecord-mcount`/`-mnop-mcount` are emitted
  (session 85); remaining ftrace gaps: emitted-inline functions skipped,
  ARM/RISC-V `-pg` warn-and-skip. Kernel build status at session close:
  467 PASS / 0 FAIL on the regression suite; next gates are objtool +
  vmlinux link (LK-28/LK-29 queue files).
- **fix_dash (RISC-V)**: still deferred; the failure mode (compile vs
  runtime) is unknown — a RISC-V execution environment (qemu-user + riscv64
  sysroot) is required first.
- **FE-25 (`-march=native`)**: no implementation work started; x86-64-v3
  remains a hard-coded static feature set, and `__builtin_cpu_supports`
  folds against a fixed Raptor Lake allowlist (`expr_builtins.rs`) —
  compile-time, not runtime CPUID.
- **`ci-codegen-gate.py` was never actually wired into `bench.yml`** (the
  v3 audit found the gate orphaned — the workflow only ran `ci-bench.py`);
  the v3 patch wires the structural gate into the benchmark job, matching
  the original MS-01a intent.
- **Tooling trap for future audits:** in this session's remote-terminal
  rendering, a literal `[main]` in YAML was displayed as `ain]`, which
  briefly looked like a corrupted CI trigger. Verify suspicious output at
  the byte level (`od -c`) before acting on it; the v3 workflow "fix"
  turned out to be a no-op because nothing was broken.

## 2026-08-31 — CCC fork survey: what to adopt and what to never touch

Surveyed all 241 forks of `anthropics/claudes-c-compiler` via the GitHub API
(plus levkropp/ms178 lineage). Only a handful have unique code; the default
branch of ~230 is byte-identical to upstream `6f1b99ac`.

- **ADOPT:** `regehr/claudes-c-compiler` `yarpgen` branch (John Regehr):
  ~60 differential-testing reproducers + fixes + `yarpgen_diff.py`/`csmith_diff.py`
  harnesses. Ported 28 reproducers into `tests/regression/`; they exposed 17 real
  LCCC miscompiles, 8 now fixed (see
  `journal/2026-08.md`).
- **DEFER (valuable):** `CrazyTodd-one` SCCP/use_def optimizer passes; `thanhtoantnt`
  property-based tests (needs a dev-dep).
- **REJECT FOREVER:** `wadsaek/claudes-c-compiler` is **malicious** — its only
  diff injects a random 4 KB penguin QUOTE string into string literals with
  10/256 probability via `/dev/random` (`get_random_value() <= 10`). Never
  adopt anything from it. Also rejected `rosubra` (deleted all passes),
  `Matr1x-101` (zlib dump), `yishangzhang` (messy logging).
- **Snapshot base:** this session rebased on `b49414c` (PR #316). The stale
  `.base_ref` (`4630de0`) in the repo root from an earlier session is NOT the
  base; `artifacts/.base_ref` is authoritative.

## v2 session (2026-08-31) — remaining 8 Regehr corpus bugs all fixed

Rebased onto upstream `main` `4e20ff0` (PR #317 merged v1). Closed all 8 open
corpus bugs; the 28 Regehr tests now pass and the full lccc suite is
**545 pass / 0 fail / 0 A/B diffs**.

| Decision | Rationale |
|---|---|
| **ADOPT Regehr `global_init_compound_ptrs.rs`** recursion/braces fixes | Two tests (`struct_array_double_singleton_inner_dim_ptr_global_init`, `global_union_array_scalar_braces_ptr_field_init`) share the compound-ptrs path; peeled braces for singleton inner dims and braced scalars. |
| **ADOPT Regehr `structs.rs`** packed-struct spill helpers | `store_packed_data_exact`/`spill_packed_data_to_alloca`/`packed_spill_alloc_size`/`load_packed_struct_i64` fix the packed small-struct assign clobber; ported with lccc's `volatile`/`semantic_volatile` IR fields. |
| **Local fix `cfg_simplify.rs`** const-branch fold cast truncation | `resolve_value_globally()` returned raw `Cast` source constants, folding `(char)512` as 512 (nonzero) instead of 0; apply `to_ty.truncate_i64(from_ty.truncate_i64(v))`. |
| **Local fix `expr_types.rs`** GNU stmt-expr/`typeof` scope resolution | `get_stmt_expr_ctype()` must resolve the tail expression against the compound-local scope before the (shadowing) outer sema scope; added `GnuConditional` + label-tail unwrapping + comparison→`int` to `get_expr_ctype_with_scope`. |
| **Local fix unnamed non-aggregate member** (C11 6.7.2.1p13) | Skip unnamed non-struct/union members in all 5 struct-field layout builders. |

No fork-specific risk: all fixes are upstream-compatible and validated by the
full regression suite + A/B harness.

## RA-06 (2026-09-05) — pressure-driven reload-at-next-use: measured NEGATIVE as an IR pre-pass

**Decision: implemented, validated correct, and shipped OFF by default
(`CCC_PRESSURE_SPLIT=1` to enable). Do not turn it on without first adding
cross-block SSA repair — see the mechanism below.**

### What was built

`split_ranges::split_high_pressure_ranges` — decoupled spill-then-color in the
Braun & Hack sense (CGO 2009), as an IR pre-pass:

* per-block program-point pressure over the GPR-eligible values;
* at each point where pressure exceeds the budget, Belady MIN — evict the
  value whose NEXT USE is farthest;
* materialise the eviction as a store at the free point and a reload as a
  **fresh SSA name** immediately before the next use, renaming from there on
  (plus successor phi operands from that block). The fresh name is what buys
  two locations for one logical value inside a backend whose assignment result
  is a single `value -> location` map.

It is correct: the full regression suite is **633 PASS / 0 FAIL / 0 A/B diffs**
with the pass forced on, and the torture corpus is clean with it on.

### Why it does not pay

| config (budget / min-gap) | kernel stkref | kernel insns |
|---|---|---|
| 12 / 4  | +119 | +121 |
| 13 / 12 | +87  | +98  |
| 14 / 20 | +87  | +98  |
| 16 / 30 | +57  | +58  |

Monotone toward zero, and **not one function in the corpus improves** — the
best-3 list is three regressions. `arith_loop` goes 165 -> 367 stack refs at
the aggressive setting.

The mechanism is not tuning. Braun–Hack pays because its spill phase
*guarantees* MAXLIVE <= k at **every** program point, so the colorer never
spills again and the store/reload traffic is the whole cost. An
**intra-block** splitter cannot provide that guarantee here: the dominant
candidates in these loops are loop-header phi values consumed in a *different*
block, and renaming those needs real cross-block SSA repair (dominance
frontiers, phi insertion). They are therefore rejected, residual pressure
stays above k, the colorer demotes anyway — and the program pays for **both**
the split traffic and the demotions.

Instrumented rejection counts for `arith_loop` block 1 at the peak: 32 of the
live values were rejected as "consumers outside this block are not successor
phi operands", which is exactly that class.

### What would change the answer

Cross-block SSA repair, so the spill phase can reach MAXLIVE <= k globally.
Until that exists this transform is strictly worse than lifetime demotion, and
the honest reading of the numbers above is that partial spill-then-color is
worse than either endpoint. Recorded here so the next attempt starts from the
guarantee rather than from the heuristic.

### Also invalidated

The earlier RA-28b hypothesis ("mode 6 loses only because of lifetime
demotion; splitting will flip its sign") cannot be tested with this pass: with
the pass on, mode 6's kernel deltas move in the same direction, because the
pass does not remove the demotions it was meant to remove.

## RA (2026-09-05, session 5) — the spill gap was slot-width classification, not splitting

**Decision: unify the width class across a copy/phi web toward the root's
wider slot. Measured −50 % hot-loop stack traffic on `arith_loop` and
−14 % corpus-wide; shipped ON.**

Two prior sessions attacked the `arith_loop` spill gap (lccc 123 hot-loop
stack refs vs GCC's 50) as a live-range-splitting problem. Both were measured
negative. This session measured the *shape* of the traffic instead of
assuming its cause, and the assumption was wrong.

### The measurement chain

1. Restricting the census to the **hot loop body** (the previous census
   weighted a benchmark's cold `main` equally): lccc 123 refs / 41 slots,
   GCC 50 refs / 22 slots, **same 15 GPR families used by both**. So it was
   never a register-count problem.
2. 40 of lccc's 82 loop-body slot reads were re-reads of a slot with no
   intervening write. A load→load reuse peephole was written for x86 (which,
   unlike AArch64, has none) — and fired **zero** times corpus-wide, because
   the slot's value is *consumed* by a folded ALU operand and never left in a
   register. The pass was deleted rather than shipped dead.
3. Reading the emitted loop body: `c += d*e` compiled to
   `movl slot_old,%eax; imull; addl slot_other,%eax; movl %eax,slot_NEW` —
   the phi and its incoming value occupy **different slots**, so every
   loop-carried variable also needs a latch copy. GCC emits one in-place
   `addl %r15d, slot`.
4. `CCC_DEBUG_SLOT_COALESCE=1`: `requested=21 resolved=0
   blocked_width_mismatch=21`. The CFG coalescer had already *proven* all 21
   phi pairs non-interfering. A width guard rejected every one.

### The defect

`is_small` requires `!wide_typed`. A Copy/Phi dest is `wide_typed` (its
emitter moves it with `movq`), while the BinOp feeding it has a narrow
`result_type()` and is classified small. A web that carries exactly one C
type therefore straddles the 4-byte and 8-byte classes, and the guard —
correctly refusing to mix widths in one slot — rejected the whole web.

### The fix

When the root is the wide member, drop the dest from `small_slot_values`.
Every access to the shared slot is then 8 bytes, so no stale upper half can
survive; and a 32-bit x86-64 ALU result is zero-extended into the full
register, so the `movq` store of the narrow value writes a well-defined
image. The opposite direction (wide dest into a 4-byte root slot) would
overrun the slot and is still refused.

### Result

| `arith_loop` hot loop body | before | after | GCC 14.2 |
|---|---|---|---|
| stack references | 123 | **62** | 50 |
| instructions | 164 | **114** | 131 |
| distinct slots | 41 | **21** | 22 |

Corpus-wide (`ra_ab_census.py`, coalescing on vs off): **−61 kernel stack
refs, −49 kernel instructions, −93 stack refs overall (−14 %)**.
Instruction count is now *below* GCC's on this kernel.

### Standing conclusion for RA-06

The remaining `arith_loop` gap (62 vs 50) is small and no longer obviously a
splitting problem. Re-measure before resuming RA-06; the two negative results
recorded above were both chasing a cause that turned out to be a width-class
bug in slot assignment.

## RA (2026-09-05, session 6) — post-RA-33 verification sweep; two negative results

RA-33 (copy/phi web slot-width unification) changed the baseline enough that
every open RA item needed re-measuring before more code was written. Three
measurements, two of which close items and one of which is a negative result.

### 1. Slot coalescing is now fully resolved — no headroom left there

Corpus-wide (`CCC_DEBUG_SLOT_COALESCE=1` over
`tests/benchmark/{programs,kernel_corpus}`):

```
requested=41  resolved=41  blocked_overlap=0  blocked_missing_root=0  blocked_width_mismatch=0
```

100 % resolve rate. Before RA-33 the same sweep blocked 21 of 21 on
`arith_loop` alone. This avenue is closed; further gains must come from the
coalescer *finding* more candidates, not from unblocking them.

### 2. RA-28 (eviction mode 6) re-measured — still not flippable, but much closer

| | before RA-33 | after RA-33 |
|---|---|---|
| kernel stkref delta | +17 | **+10** |
| kernel insns delta | +16 | +9 |
| whole-corpus stkref | −9 | −23 (−3.5 %) |

Mode 6 now wins clearly on the whole corpus but still regresses the hot
kernels, which is the gate. Default stays 3. The trend says the remaining
kernel penalty is small and may fall out of a future spill-model change
rather than out of tuning.

### 3. NEGATIVE — two-tier value typing (constants as a weak seed)

`compute_value_type_map` merges instruction result types and constant operand
types into one "widest wins" map. A literal is materialised at the target's
storage width, so `int a = 1;` lowers to `Copy(a, Const(I64(1)))` on LP64 and
types the whole web `I64`, which vetoes its 4-byte slot.

A two-tier version was implemented — instruction `result_type()` authoritative
(strong), constants advisory (weak), weak consulted only for values no
instruction types — on the theory that it would narrow `arith_loop`'s 32 `int`
accumulators from 8-byte to 4-byte slots.

**Measured effect: none.** Byte-identical output on `arith_loop` (114 insns /
62 refs / 11 `movq`, unchanged) and byte-identical census totals over 67
functions. Some other seed already types those values wide; the literal is not
the binding constraint. Reverted rather than shipped as unmeasured risk
surface. Anyone retrying this must first find which seed actually sets the
width — `Instruction::Phi { ty }` is a strong seed and is the next suspect.

### Where lccc now stands against GCC 14.2 (67-function corpus, -O2)

| bucket | lccc | GCC | |
|---|---|---|---|
| instructions | **2060** | 2107 | lccc ahead |
| stack references | **148** | 219 | lccc ahead 32 % |
| reg-reg moves | 317 | **272** | lccc behind |
| callee-saved pushes | 47 | **38** | lccc behind |

The two headline spill metrics are now in lccc's favour. The remaining
outlier is `glibc_memcmp_common_alignment` at 225 insns vs 87 with 26 reg-reg
moves vs 2 — inspection shows an inlining/unrolling difference around the
inlined byte-compare helper, **not** a register-allocation problem, so it does
not belong to any RA item. `rrmov` and `push` are the two buckets where RA
work still has headroom (RA-07 callee-saved policy, RA-08 affinity
coalescing).

## Codegen (2026-09-05, session 7) — epilogue cross-jumping; fresh timing baseline

### Landed: epilogue tail merging (`epilogue_merge.rs`)

Every `return` site in a function that uses callee-saved registers received
its own full epilogue. `glibc_memcmp_common_alignment` had **six byte-identical
copies** of `addq $120,%rsp; popq %rbp; popq %r15; popq %r14; popq %r13;
popq %r12; popq %rbx; ret` — 48 instructions, 40 redundant. GCC cross-jumps.

The pass groups exit sites by their exact epilogue text, labels the first, and
replaces the rest with one `jmp`. Only `popq %reg` (never `%rax`),
`addq $imm,%rsp` and `leave` are collected, so the return value — materialised
before the run — is never touched; a run containing a label is rejected; the
pass is per-function and bails on any function with CFI inside the body.

| | before | after |
|---|---|---|
| `glibc_memcmp_common_alignment` insns | 224 | **197** |
| its `popq` count | 36 | **6** |
| corpus instructions (67 fns) | 2060 | **2030** |

Corpus opportunity measured before implementing: 15 functions, 119 redundant
instructions.

### `glibc_memcmp` root cause — the remaining 197 vs GCC's 87

Diagnosed, not yet fixed. Two independent causes, both confirmed by
measurement:

1. **Inlining the byte-compare helper.** `CCC_INLINE_SKIP=glibc_memcmp_bytes`
   drops the function from 224 to 133 instructions. GCC also inlines it but
   keeps the 8-iteration byte loop **rolled** (7 instructions, one copy);
   lccc replicates it. Oracle check: GCC 16.2 = 112 instructions,
   ICX = 152, **Clang 23 = 238** — lccc at 225 was mid-field, not last.
2. **Six callee-saved registers where GCC uses zero.** The source loads all
   eight words up front; GCC *sinks* each load to its compare
   (`movq 8(%rax),%rdi; movq 8(%rcx),%rsi; cmpq; jne`), keeping two values
   live instead of eight. lccc has no scalar load-sinking pass — only
   `vec_load_sink` for vectors — so pressure forces six callee-saved pushes
   and a 120-byte frame.

**Scalar load sinking is the high-value item this exposes** (filed OPT-40): it
is the difference between two and eight simultaneously live values in every
"load a batch, compare with early exit" loop.

### Fresh timing baseline (paired median, 9 reps, VM screening, no PMU)

Worst ten vs GCC 14.2:

| # | benchmark | LCCC/GCC |
|---|---|---|
| 1 | spectral_norm | **1.635×** |
| 2 | nbody | 1.320× |
| 3 | fannkuch | 1.278× |
| 4 | expat_xml_scan | 1.259× |
| 5 | glibc_memcmp | 1.258× |
| 6 | hash_table | 1.190× |
| 7 | double_reduction | 1.179× |
| 8 | sqlite_varint | 1.125× |
| 9 | sieve | 1.122× |
| 10 | arith_loop | 1.113× |

Large wins on the other side: fib 23.8× faster, ackermann 19.3×,
constant_recursion 18.1×, libm_round_family 4.16×, bitops 1.22×,
loop_patterns 1.11×.

`spectral_norm` at 1.635× is the single worst result and the correct next
target; `nbody` (1.32×) and `double_reduction` (1.18×) are the same FP-kernel
family and are likely to share a root cause with it.

## Audit (2026-09-05, session 8) — red-team of #412–#419; width-test fix; i686 liveness oracle

### Upstream defect fixed: width unit test contradicted the width table

`declared_width_agrees_with_the_lane_count_in_the_name` asserted
`VecSubI32x8 / VecAndI32x8 / VecOrI32x8 / VecXorI32x8 == Some(16)` — the four
ops were pasted into the 128-bit `narrow` list by the same commit that added
them, while `vector_result_width` correctly reports `Some(32)` (8 lanes × 4 B;
codegen lowers them via `emit_avx_binary_256` on `%ymm`). Implementation
right, test data wrong. The test is now exhaustive by construction: it
extracts every `Vec*` variant from the enum via `include_str!` at compile
time and derives the expected width from the name's last `<Type>x<Lanes>`
token. Documented divergences: `Horizontal*` -> None, `VecStore*` -> None
except the deliberate `VecStoreI64x2` (slot sizing), unlowered
`Sadalp/Smlal` -> None. A mechanical sweep of the full enum found no other
table/name disagreement, and a future op is checked automatically instead of
relying on a hand-pasted list.

### Guard placement: the per-analyzer form is load-bearing

Hoisting the non-IV-phi carried-value guard from `analyze_map_pattern` /
`analyze_stencil_pattern` into `analyze_loop_pattern` looks like
deduplication but is a latent miscompile: `analyze_loop_pattern` is the
matmul analyzer, not a shared entry, and its `None` falls through the
dispatcher's else-if chain to analyzers without the guard. Reproduced as
`k_acc(1) = 0` (want 3) on `vectorize_carried_value_and_negzero`. The
per-analyzer guards stay.

### Ported (A/B-measured against current main)

1. `memfold_consumer_256`: order-correct memory-operand fold gates for the
   four integer lane ops (And/Or/Xor commutative, Sub src2-only, matching
   the packed min/max contract). Without the gates the xor map kernel emits
   `vmovdqu` loads plus stack round-trips; with them the consumer folds as
   `vpxor (mem), %ymm, %ymm`. `is_vec_ssa_producer` gains the same four
   families for classification consistency; on current main the omission is
   correctness-neutral (the deferred-store path was reorganized in #413 /
   #416 — the historical segfault claim does not reproduce at -O2/-O3,
   v3/SSE2, stack or static storage, or under CCC_NO_SMALL_SLOTS), so the
   entries are hardening, not a live fix.
2. i686 whole-function GPR liveness oracle for `fold_memory_operands`:
   the bounded forward scan stopped at the first barrier and ASSUMED
   deadness, which is unsound for register-allocated values —
   `maxi(a,b){return a>b?a:b;}` folded its param home
   `movl 20(%esp),%esi` into `cmpl 20(%esp),%ebx` across the select's
   conditional jump and the fall-through arm read a never-written `%esi`.
   The oracle is the same backward dataflow `optimize_cfg_register_liveness`
   already used, factored out; computed once per sweep, sound across folds
   (deleting a dead definition can only make the deleted register's cached
   liveness conservative). Spot-verified: maxi keeps its param homes across
   the select; divmod's `cltd`/`idivl` implicit `edx:eax` handling survives
   every rename.
3. Tests: `vex_promote_semantic.c` (VEX promotion semantics: remainder
   tails, jump-table dispatch, live-128-bit-half soundness, self-zeroing
   idioms, conversions) and `vectorize_int_map_lanes.c` extended ADDITIVELY
   with the exact FP min/max family (four strict folds + two non-strict
   no-folds over NaN/+-0 pairings; verified `vminps`/`vmaxps` fire for the
   strict forms while the no-fold forms keep compare+blend).

### Adopted as-is (audited, no defects found)

#412 five miscompiles + RA cost model; #413 if-convert deref coverage, C23
`va_start`, `folded_read_points`, vex BFS, xmm2 confinement; #414 lane ops +
carried-value guard + `-0.0` data emission + clamp fold; #415 cast-peel
injectivity + RA-06 (off by default, documented); #416 wider slot roots;
#417 docs; #418 epilogue cross-jumping (soundness gates audited: runs
rejected on labels, whole-run replacement, per-function CFI bail, return
regs untouched; known trade-off — exact-text merging can grow byte size for
2–3-insn epilogues since `jmp rel32` is 5 bytes; insn count is the canonical
metric here; LCS extension is CG-09); #419 latch-segment footprints
(`BlockTouches` closes the phi-eliminated latch hole that corrupted
ZSTD `HUF_decompress4X2`'s `endSignal`) and LEA invalidation routed through
the shared may-write oracle — the same over-approximation contract the x86
implicit-write oracle established; `sete %al` after a cached `leaq` is the
canonical one-operand writer the hand list missed.

Verification: `cargo test --lib` 1956/1956; regression suite 626 PASS /
0 FAIL / 15 SKIP (ELF32 — no runner on this host); map_sub emits `vpsubd`,
4×4 matmul 16× `vfmadd`, i64 reduction `paddq`; int map lanes hashes
identical to GCC at -O3 -march=x86-64-v3.
## Session 10 — expression sinking LANDED after three defects and a wrong cost model

Shipped enabled. Corpus static: **−4 instructions, −4 reg-reg moves**, zero
stack refs / pushes regression. Dynamic (minimum of 13 interleaved rounds):
`hash_table` **1.6 % faster**, `expat_xml_scan` and `sieve` neutral,
`glibc_memcmp` +1.1 %. No output mismatch anywhere.

The previous two attempts failed because three separate things were wrong.
All are fixed and each is now pinned by the measurement that found it.

### Defect 1 — stale indices when a block is a destination then a source

Moves were planned against one IR snapshot and applied in sequence. A block
that received an insertion had every later index shifted, so a subsequent
move *out* of that block took the wrong instruction. The old guard rejected
"source already used" and "destination already used" but not "source is a
previous destination". Now: at most one move per block per round, either
direction; the fixpoint picks up the rest.

### Defect 2 — operands that are not single-def (the `nbody` miscompile)

This IR is not strict SSA after accumulator forwarding: a value can be
assigned in several blocks. Sinking `t = a + b` past a *redefinition* of `a`
changes what it computes, and `nbody`'s FP accumulators are exactly that
shape. Every operand must now be single-def, and no operand may be redefined
anywhere in the traversed region. `nbody` output is now byte-identical with
the pass on and off.

### Wrong cost model — sinking a k-operand op costs k−1 registers

The premise "sinking shortens a live range" is only true for one-operand
computations. Sinking moves the boundary crossing from the RESULT to the
OPERANDS:

```
  t = load p     ->  p crosses instead of t      : 1 for 1
  t = a + b      ->  a AND b cross instead of t  : +1 register
```

Unguarded, the corpus went **+62 instructions, +28 reg-reg moves, +14
callee-saved pushes**. Requiring at most one operand's range to be extended
brought that to +14; requiring **zero** (every operand already live for other
reasons, so the sink purely frees the result) turned it into −22 insns / −7
rrmov / −2 push.

### Missing guard — execution FREQUENCY, not loop depth

Equal loop depth does not mean equal execution count. Sinking into a
frequently-taken conditional adds work to the hot path even though it
shortens a live range: `expat_xml_scan` went **51.9 ms → 67.4 ms (+30 %)**,
reproducibly, and identically with load sinking separately disabled — so it
was the motion itself, not a memory effect.

Requiring the target to **post-dominate** the source makes the sink
frequency-neutral by construction: every execution of the source reaches the
target exactly once, so the computation runs neither more nor less often and
the only change is the shorter live range. expat returned to 51.8 ms.

This also supersedes the session-9 conclusion. The −1.76 % measured then was
real, but it was the cost model and the missing frequency guard, not an
intrinsic property of sinking — and it was measured with a MEDIAN, which on
this noisy shared VM is biased upward by neighbour interference. Noise here
is strictly additive, so the minimum of N interleaved rounds is the correct
estimator; `scripts/perf_ab.py` now uses it.

### Load sinking

Kept enabled but separately switchable (`CCC_NO_LOAD_SINK`). With the
frequency guard in place it no longer shows the memory-level-parallelism
penalty that session 9 attributed to it — that penalty was the frequency
effect in disguise.

## Session 11 — expression sinking: the guard is LOOP ENTRY, not post-dominance

The session-10 pass shipped with a post-dominance guard I flagged as a
conservative proxy. It is replaced with the correct invariant, and two further
relaxations were measured and rejected. Final state: corpus **-4 instructions,
-4 reg-reg moves**, no pressure or timing regression anywhere.

### Post-dominance was the wrong invariant — for the opposite reason

Post-dominance says every path from the source reaches the target, i.e.
`freq(target) >= freq(source)`. That is the *reverse* of what a sink needs.
What actually bounds the cost is **dominance**, which the pass already
required: if the source dominates the target then every execution of the
target arrived through the source, so within one loop nest
`freq(target) <= freq(source)` for free.

### So why did `expat_xml_scan` regress 30 %?

Not frequency. A static block-frequency model (reverse-postorder propagation,
uniform branch probability, trip-count factor per nesting level) was built and
the guard replaced with `freq(target) <= freq(source)`. It admitted more sinks
— corpus -22 insns — and `expat` went straight back to **+30.8 %**, with
`hash_table` +8 %.

The cause is a **loop boundary the depth counter cannot see**. Comparing loop
*depths* passes a move that leaves one loop and enters another: the count is
equal, the execution multiplier is not. Replacing the guard with

> reject if any natural loop contains the target but not the source

fixes it exactly: `expat` 51.9 ms -> 51.9 ms (1.007), `sieve` 1.000,
`glibc_memcmp` 1.003. This is strictly better motivated than post-dominance
and, in principle, strictly more permissive — it allows the branch-guarded
sink that post-dominance forbids.

### Two relaxations measured and rejected

| relaxation | corpus insns | rrmov | push | verdict |
|---|---|---|---|---|
| frequency guard only (no loop-entry rule) | **-22** | -7 | -2 | REJECT — expat +30.8 %, hash_table +8 % |
| allow one extended operand when it is a sinkable CHAIN link | **+75** | +27 | +15 | REJECT — `glibc_memcmp` 197 -> 206 |

The chain relaxation is the interesting failure. `v = load p` with
`p = gep base, k` used only by that load extends `p`, but `p` is itself
sinkable and follows on the next round, so the chain *should* migrate as a
unit at no net cost. It does migrate — and it still loses, because the
intermediate rounds hold both ends live simultaneously and the allocator
commits to the worse shape before the chain lands. Sinking a whole chain
atomically in one round is the only way this could pay; a fixpoint of
single-instruction moves is not equivalent.

### Why `glibc_memcmp` still shows 6 callee-saved pushes

Its loads are exactly the rejected chain shape: each `load` extends its
single-use `gep`. The pressure win there needs atomic chain motion, which the
data above says must be built as one transaction rather than approximated by
iteration. That is the remaining lever and it now has a measured cost to beat
(+75 insns for the naive version).

## Session 12 — OPT-42 rejected with data; CG-09 landed (-109 instructions)

### CG-09 LANDED — epilogue merging by longest common SUFFIX

`epilogue_merge.rs` grouped exits by *identical* epilogue text, which only
catches paths that restore exactly the same registers. Real functions also
produce exits differing only in how much they restore — a path that never
touched `%r15` pops one fewer register — and the shorter epilogue is then a
strict **suffix** of the longer one. Jumping into the middle of the longer
copy merges those too.

Longest host first, so every shorter exit finds the deepest available host; a
label is planted once per (host, suffix length) and reused.

**Corpus: 6551 -> 6442 instructions, -109 (-1.7 %)** — up from the -30 that
exact matching achieved. No pressure or correctness change: the transform only
ever replaces a whole `pop*/addq %rsp/ret` run with one `jmp`, and the
soundness argument is unchanged (no `%rax` pop, no label inside a run,
per-function, bails on CFI in the body).

### OPT-42 REJECTED — atomic chain sinking

Implemented exactly as session 11 specified: collect the maximal set of
instructions in the block that feed the candidate and have no other consumer,
judge profitability on the CHAIN's external inputs rather than one
instruction's operands, and move the whole set as one transaction (remove
highest-index-first, re-insert in original relative order, never observable
partially migrated).

| | corpus insns | rrmov | stkref | push |
|---|---|---|---|---|
| atomic chain sinking | **+22** | +1 | +2 | **+11** |

And `glibc_memcmp` was unchanged at 197 instructions / 6 pushes — the chain
still does not fire there, so the +22 is pure cost from chains that do fire
elsewhere.

This closes the hypothesis carried since session 11. The chain reasoning —
"external inputs are live anyway, one result is freed, so the move is free" —
is correct about *pressure* and still loses, for the same reason the earlier
relaxation did: moving several instructions at once lengthens the region over
which the chain's external inputs must stay live, and that cost is not
captured by counting boundary crossings at a single point. A model that only
counts values crossing one edge cannot decide multi-instruction motion.
**Do not retry chain sinking without a live-range-length cost model.**

The single-instruction pass with the loop-entry guard (session 11) remains the
shipped configuration: corpus -4 instructions, -4 reg-reg moves, no regression.

## Session 13 — OP-42 length model supersedes the earlier rejection

The Session 12 rejection remains valid for its **boundary-only** atomic-chain
cost model, not for atomic motion in principle.  `89adb5c3` adds the missing
full live-range-length, call-crossing, and live-point accounting and commits a
chain only as a complete transaction after a strict Pareto check.  The targeted
unit cases and verifier-enabled corpus gate pass; the exact implementation and
negative controls are recorded in
`journal/2026-09-W1.md`.

## Session 13 — normal `-O2` inline pressure is measurable, not a size-only choice

Three independent workload shapes show that the existing unconditional small
static-inline path can inflate the merged function's live set more than it
saves in call overhead.  The policy is deliberately narrow: plain static
functions only; `static inline`, GNU inline, always-inline, custom-section,
PGO-forced and single-owner cases retain their stronger existing contracts;
`-Os` keeps its separately measured policy.

1. A two-site tiny wrapper around an inlineable loop (`spectral_norm`'s
   `mul_AtAv`) must remain outlined even after a later inliner invocation has
   expanded its descendant.  Persisting `has_inlined_calls` closes that
   fixed-point hole: same-window minimum `250.28 -> 205.50 ms` (`0.821`).
2. A multi-site plain loop callee called from an enclosing loop (`lookup` in
   `hash_table`) remains outlined.  The exact old/new screen is `9071.38 ->
   7632.42 ms` (`0.841`) and emits `228 -> 174` assembly instructions.
3. A 27-instruction small loop cloned five times into glibc memcmp branch
   exits stays out of line after a cap of four clones.  The mechanically
   amplified source has low-5 `78.155 -> 69.889 ms` (`0.8942`), while its
   caller falls `183 -> 93` instructions and `18 -> 0` stack references.

The decision is based on interleaved, CPU-pinned VM screens and retained raw
samples, not cross-run comparison.  The normal inline policy should evolve to
profile-aware benefit versus post-allocation pressure feedback; direct-call
count is a deliberately conservative interim proxy.  Full rationale,
exceptions, and validation are in
`journal/2026-09-W1.md`.

## Session 13 — TLS GlobalAddr CSE class unification

The global-address CSE previously kept foldable and must-materialize TLS uses
in separate webs.  That distinction is essential for ordinary globals (a
foldable use may become RIP-relative), but false for TLS: every TLS address
must materialize a thread-relative `%fs` base.  Merging TLS use classes removes
a redundant address sequence in `tls_pass` (three to two) without a measured
runtime regression (low-7 `0.9947`).  A unit test pins the normal-global
non-merge and TLS-only merge distinction.

## Session 14 — PF-15 signed byte-carrier fold landed; broad U8 pair rejected

The obvious `strcmp` repair is not merely a late peephole: a source byte can
be promoted to a wide temporary for a zero test, then independently promoted
again for a comparison after another load.  The simplifier now exposes the
narrow integer sources before allocation, but only after exact cast-result use
counting proves the wide carrier dies at that consumer.  This is a genuine
live-range-length argument: `source→cast + cast→consumer` equals
`source→consumer`; existing later source uses only make deleting the carrier
better.  Conditions use the injective-zero fact; pairs accept signed equality
and signed relational predicates only.

The first version also admitted zero-extended U8 pairs.  It was semantically
correct, static Expat was smaller, and runtime was **worse**: 101 alternating
rounds had broad default `41.89 ms` vs both-folds-disabled `40.14 ms`;
pair-only isolation had default `41.84 ms` vs pair-disabled `40.46 ms`.
A one-NOP controlled layout move at the affected quote-loop header erased and
reversed the loss, proving placement/RA sensitivity rather than a simple
`cmpb` throughput conclusion.  Therefore it is not shipped.

The final signed-root policy keeps the designed hot carrier win (a CPU-pinned,
AB/BA-balanced 101-sample `strcmp_signed` kernel: default `43.179 ms`,
pair-disabled `53.406 ms`, B/A `1.237`) while making final Expat neutral
(`40.17/40.27 ms`, B/A `1.003`) and strlen neutral within the 1% VM threshold
(`185.94/187.30 ms`, B/A `1.007`).  Kernel count falls `277→273`; the
55-source census falls `6864→6850` instructions with no stack-reference or
push increase.  Exhaustive signed-byte equality/termination and all-six-relop
C regressions, focused IR tests, the structural lowering checks, and the full
suite (`638 pass, 0 fail`) all pass.  The complete evidence, compiler-oracle
gap, and follow-up guardrails are in
`journal/2026-09-W2.md`.

## Session 15 — cycle-accurate phi copy ordering landed opt-in; the removed copy was a live-range splitter

Phi elimination decided "needs a temporary" by the predicate *my source is
somebody's destination*, which is true of every copy in a rotation.  SHA-256's
eight-word state rotation is acyclic — two chains — yet went through the
two-phase scheme in full, costing not one redundant instruction but a redundant
*web*: twice the loop-carried values and twice the unconditional copies per
round, which pushed the allocator over budget and staged the rotation through
the stack.  Replacing the predicate with an exact Kahn decomposition of the
precedence graph (`i -> j` iff `i` reads the destination `j` writes) takes the
reduced rotation kernel from `73` instructions / `27` stack refs to `55 / 0`,
against gcc's `71 / 0`, and `sha256_transform` from `210 / 62` to `188 / 45`.
An exhaustive oracle over `18,240` copy graphs checks every plan against the
simultaneous-assignment semantics, asserts non-degeneracy, and holds *both* arms
to it, so the retained legacy path is proved sound rather than merely
conservative.

It is **not shipped as the default**, because it is `~5–7%` slower at runtime on
`sha256_transform` *despite* the lower static counts.  The mechanism is
measured: with the two-phase temps the loop-carried word exists as two short
complementary ranges (phi input latch→header, body value header→latch) and the
allocator assigns both; merging them into one range that spans the back edge
loses, and `CCC_DEBUG_RA_PHASES` then shows that range in neither `assigned` nor
`spilled` — left stack-homed and unassigned.  The demoted values are exactly the
two recurrence words, re-read `7` times per iteration because a destructive
rotate needs a fresh destination register per use: `14` memops concentrated on
`2` slots, against the default's `25` spread over `11` at `2` reads/slot max.
The default arm has *higher* register pressure and still wins, so the operative
variable is range structure, not live-value count.  **Eliminating redundant
copies is not monotone in code quality while ranges split only at phi
boundaries** — the redundant copy was performing a back-edge live-range split.
The prerequisite for enabling it is a profitable back-edge split in the
allocator (RA-06 location pieces).

Falsified, do not retry: the `movq`-store/`movl`-load width mismatch in the
rotation is *not* the cause (hand-rewriting the loop's moves to 32-bit moves
runtime by `1.8%`, i.e. noise, and leaves the gap at `5.5%`); `CCC_EVICT_MODE`
and `CCC_NO_TIER2_GRAPH` do not change the victim set.

Harness lesson: `perf_ab.py`'s corpus geomean reported a verdict over arms of
which **6 of 8 compiled byte-identically** — their deltas are pure noise, and the
noise floor on a 2-core VM is `±4%`.  MD5-compare arm binaries before trusting
any ratio, and assert only on kernels whose binaries actually differ.
`scripts/paired_ab.py` now enforces this instead of leaving it to discipline: it
builds and hashes both arms, exits `3` *UNINFORMATIVE* with no verdict when they
are byte-identical, exits `4` on a stdout/status disagreement (a correctness
failure is never reported as a timing result), interleaves with alternated
order, reports `min` beside `median` and flags directional disagreement, and
runs a paired sign test.  With `--allow-identical` it reproduces the noise floor
on demand, and the result is worse than "noisy": two **byte-identical**
`base64_enc` arms, whose true effect is exactly zero, measure a `5.67%` median
delta at paired sign-test `p = 0.0164` — *nominally significant*.  Interleaving
and order alternation do not remove it, because on a 2-core shared VM the bias
is systematic and correlates within a round, so a paired test inherits it
instead of averaging it out.  **Statistical significance does not imply a real
effect, and no amount of paired-round discipline substitutes for hashing the
arms first** — which is why the identity guard is a hard exit, not a warning.
Raw evidence for the whole decision is frozen under
`engineering/evidence/phi-acyclic-copy-order-2026-09-11/`.

The shipping default arm is **byte-identical to base `4f527199` across `450`
translation units**, so the regression is entirely contained behind
`CCC_PHI_ACYCLIC_ORDER=1`.  The CI gate `phi-acyclic-copy-order` pins four
properties — mechanism fires and beats gcc, cyclic near misses stay correct in
both arms, the flag is wired, and the opt-in does not leak (unset/`0`/empty/
`true` all reproduce the default byte for byte) — and is mutation-verified in
both directions.  Full evidence, the census tables, and the two independent
allocator leads found while diagnosing are in
`journal/2026-09-W2.md`.  One of those leads
is recorded there as **corrected and closed**: `MACHINST_ALLOCATABLE_GPRS` has
`15` entries and *does* include `rax`/`rcx` (an earlier reading of this session
said `13` with the two reserved — wrong; only the *main* RA never homes them).
Widening the round loop's budget would mean forcing MachInst on a large loop,
which `agent/RULES.md` item 16 already measured negative (gzip `-3%`), so it was
not pursued.  The surviving lead is that x86 has no slot-load dedup —
`CCC_NO_SLOT_LOAD_DEDUP` exists only in the ARM peephole.

## Session 16 — web-wide in-loop-use supply landed; the phi-resolver default is withdrawn

Rebased onto `25ed36de` (upstream landed four commits, one of which adds a
cost-ratio escape to `select_evict_victim`), and the whole phi-acyclic
disposition from Session 15 had to be re-derived rather than carried across.

**Landed:** `mark_loop_spanning` now supplies merged coalesce members into
`uses_in_extents` from a per-value use-point map.  The aggregation was documented
as web-wide — "the member map carries the web-wide in-loop-use flag: a leader's
own `uses` under-count a phi web exactly the way its priority does" — but was
built from a pass over `ranges`, and a merged member owns no `LiveRange`, so it
was a silent no-op.  The flag therefore degraded to *does the leader have an
in-extent use*, which is false for every phi web led by a cold preheader
definition, i.e. every loop-carried recurrence.  The in-loop-USELESS-span
admission rule then demoted the hottest values in the loop: on
`sha256_transform`, `leader=v166 members=[166,389]` and `leader=v182
members=[182,392]`, each reloaded 7x per iteration, 14 of the round loop's 15
memory operations on two slots.  Span flags only — `LiveRange::uses` and
`priority` are untouched, because inflating a coalesce web's priority in the main
scan waves is a recorded negative (expat -30%, adler32 -23%, arith_loop -12%,
sha256 -56%).  Kill switch `CCC_NO_WEB_INLOOP_USE`
(`RaConfig::no_web_inloop_use`), which reproduces base assembly byte for byte;
gate `tests/regression/check_ra_web_inloop_use.sh`; unit test pins both arms of
the switch.

**Measured** (amplified to ~430 ms/arm, `PASSES=8 BLOCK_COUNT=131072`, paired
interleaved, arms byte-distinct and digest-identical to gcc, two replicates at 51
and 41 rounds): base -> RA fix alone **+3.63% / +4.33%** (p=0.0000 both, median
and min agreeing both times); RA fix -> + phi resolver **-1.99% / -0.71%**;
base -> both **+1.88% / +1.98%**.  The legs compose (0.9637 x 1.0199 = 0.9828 vs
0.9812 measured), so the decomposition is consistent rather than three noise
draws.  Static: 210 -> 198 insns, hottest-loop most-reloaded slot 17 -> 10.

**Withdrawn:** Session 15's plan to default-enable the resolver, and the
`+8.21%` "both together" reading taken earlier the same day.  That reading was
un-amplified (~55 ms/arm) and its median disagreed with its own min ratio
(+3.8%), violating the harness's `median_and_min_agree` criterion; it did not
survive a rebase onto a main that changes eviction.  Shipping the allocator fix
alone is worth roughly twice as much as shipping both.  Recorded as
`agent/RULES.md` item 30: re-derive factorial conclusions after every rebase,
amplify to >=200 ms/arm, and never let a median-only result choose what ships.

**Also corrected:** the gcc oracle comparison.  A function extractor matching
`^\s*\.size <name>` with a literal space silently falls back to the whole
translation unit on gcc's tab-separated `.size\tsha256_transform`, which produced
a bogus "gcc 240 insns / 31 stack refs" and a false claim that LCCC's 178 / 28
beat it.  The truth is gcc **142 / 8** against LCCC's shipping 198 / 62, and gcc is
**44.10% faster** at runtime (31-round amplified paired A/B, median ratio 1.4410,
min 1.4361, p=0.0000).  Both compilers emit the same number of loops here (2
backward jumps each), so the gap is not loop structure: it is 54 extra
frame-relative stack references, ~40 extra `mov`s, 9 extra labels and 6 extra
compares.  Per-loop attribution by backward-jump span is unreliable in LCCC's
output (one span covers most of the function) and an early "LCCC's round loop is
176 insns / 58 memops vs gcc's 47 / 0, so the loops are fused" reading is
withdrawn on the strength of those jump counts.  Both gate scripts now use
`\.size\s+<name>\b` and fail loudly instead of falling back.  The remaining P0-B
gap is spill traffic, not instruction count, and RA-06B's back-edge-split framing
is superseded in `tasks/TASK-RA-06A-RELOAD-AT-USE.md`.

**Upstream interaction, measured not assumed:** the new `evict_short_k` escape
(default 16) helps the legacy `rot()` arm (71/33 -> 71/27) and costs the resolver
arm (56/2 -> 61/4).  The resolver's own contribution is large at both settings, so
`check_phi_acyclic_order.sh` now pins that contribution at production settings
*and* at `CCC_EVICT_SHORT_K=0`, instead of an absolute "0 stack refs" that now
belongs to a different component.

**Gates:** `cargo test --lib` 2386/0, `ci_local.sh --fast` 24/0/3 (including the
new `ra-web-inloop-use` gate), clippy clean, rustfmt clean.

## Session 17 — auditing our own RA fix found a 40 % regression we had shipped; the supply is now boolean-only

Session 16 landed the web-wide in-loop-use supply on the strength of
`sha256_transform` alone.  This session re-examined it critically, and the audit
produced two separate results: the implementation was defective in five ways, and
the *design* was feeding a decision it should not have touched.

**Implementation, fixed with no change to machine code.**  The supply hand-rolled
a use-point walk over `func.blocks`, re-implementing `collect_range_metadata`;
that walk iterated instructions only and so dropped terminator uses, which
`record_terminator_uses` records at the block-end point — and the block-end point
is exactly `loop_extents`' latch end, so a value read only by a loop's terminating
branch was invisible on the member side while visible on the leader side.  It
counted operand occurrences where leaders count deduplicated distinct points
(`set_uses_weighted` sorts and merges duplicates by summing weights), putting the
two sides of one sum in different units — and `span_exposed_uses` is thresholded
against `MAX_SPAN_EXPOSED_USES`, not tested for non-zero, so that asymmetry can
flip demotability.  Its `loop_extents.is_empty()` bail-out sat after both full IR
walks, and a write-only `any_use_in_extent` map was left behind.

The canonical `RangeMetadata::uses` is now threaded out of
`build_live_ranges_with_config_and_meta` and passed into `mark_loop_spanning`, so
both sides share one numbering by construction rather than by convention; the
dead `collect_uses_for_values` wrapper and the dead map are deleted; the bail-out
is first and the supply is skipped entirely when there is no coalescing.
`meta_uses` is guaranteed non-decreasing per value because **Phi is an instruction
in this IR, not a terminator**, so every `record_use` happens at the current
monotonic point — which is also what makes adjacency-based dedup sound.  Note the
leader side's own `last_point` guard is vestigial, since `set_uses_weighted` has
already deduplicated `r.uses`.

Two of the five were latent bugs, not style, and both are now pinned by tests that
were mutation-verified: `mark_loop_spanning_member_terminator_use_supplies_the_flag`
fails against a terminator-blind mutant, and the dedup test failed against a
dedup-removed mutant.  A test that cannot fail is not evidence.

The refactor is provably output-neutral.  `scripts/differential_corpus.sh` (new)
byte-compares `-O2 -S` output for every `.c` under `tests/`: **805 of 807 compile,
0 exit-status differences, identical 2-file failure set, 0 assembly differences**.
Identical machine code is why no runtime A/B was re-run for the refactor itself —
the arms would be byte-identical and `paired_ab.py` exits 3 on those by design.

**Design, changed.**  Screening the whole corpus instead of re-timing only the
benchmark the fix was aimed at named 13 changed translation units, one of which
was `lz4_compress`.  Timed: **base is 40.53 % faster** (median 1.4053, min
1.4078, p=0.0000, 41 rounds, ~186 ms/arm) at **identical instruction count** —
251 instructions in `main` for both arms.  That is the §22/§23 case, and the
allocator's own trace gave the mechanism: with counts fed web-wide, v212's
`span_exposed_uses` goes 1 → 3, past `MAX_SPAN_EXPOSED_USES = 2`, so
`worth_capping` vetoes it at the admission cap (remcost 100, site 2); the pressure
falls through to the span-pressure valve, which selects on
`MAX_VALVE_SPAN_FUTURE_USES` with **no cost term** and spills v133 at remcost
1110 — an 11× more expensive victim.  `main`'s hot loop goes 25 → 29 frame refs.

The count is not lying: coalesced members share one register, so those three reads
really would become hot reloads.  The defect is that `worth_capping` is a veto with
no cost-aware replacement, so a more accurate input produced a worse global
decision.  Base gets the right answer here by accident — it under-counts the web,
which keeps the cheap span demotable.  Recorded in
`journal/2026-09-W2.md` with the required sequencing: make
the valve cost-ordered **first**, then re-feed web-wide counts.  It is deliberately
not bundled here, because the valve is core victim selection whose measured history
(chacha20's ARX webs, adler32's inlined NMAX loop, glibc_memcmp's address span)
would all have to be re-derived.

What ships is the strictly smaller change: the member supply feeds the web-wide
BOOLEAN `span_has_in_loop_use` — which is where the sha256 win comes from — and
leaves the cap's per-range counts as calibrated.  Blast radius drops 13 → 10 of
805 TUs; `lz4_compress` becomes byte-identical to base; `divrem_pair_opposite_flavour.c`
and `o0_phi_multidef.c` stop being touched at all; and `sha256_transform`'s and
`linux_rbtree`'s assembly is **byte-identical to the count-coupled build**, so
every sha256 number published in Session 16 still describes the shipping compiler.

**Measured, base → shipping:** `sha256_transform` +3.63 % median / +3.81 % min
(31 rounds, p=0.0000); `linux_rbtree` +1.05 % / +1.06 % (61 rounds, p=0.0000 —
amplification is only ~65 ms because larger `NODE_COUNT` or any `LOOKUP_ROUNDS`
change makes the codegen difference vanish, so read it as "not a regression,
probably a small win"); `strlen_bench` +1.18 % / +2.89 % (31 rounds, p=0.0012,
sd ≈ 9 % so the magnitude is soft); `adler32_do8` neutral at 1.001× against the
kill-switch arm (`bench_kernels.py`, 69 instructions both arms) — worth checking
because adler32's checksum webs are the counter-example that calibrated
`MAX_SPAN_REMCOST`; `k01_adler` improved 62 → 59 instructions.  `lz4_compress`
arms are byte-identical, so the harness correctly reports UNINFORMATIVE.

**Guards:** `check_ra_web_inloop_use.sh` gains property 5 — `lz4_compress` must be
byte-identical with the supply on and off.  Mutation-verified: against a
count-coupled build it exits 1 reporting "224 differing lines" while still
confirming the sha256 mechanism fires.  Unit twin:
`mark_loop_spanning_member_uses_do_not_inflate_the_cap_counts`, where a member read
at four distinct in-extent points must set the boolean and leave both counts at
the leader's own zero.  The `SPILL-TRACE` diagnostic now prints
`in_loop_uses`/`exposed`/`recur` at all six demotion sites; without those fields
the lz4 mechanism was not visible in the trace at all, which is why the first
hypothesis (sites 0/4) was wrong and had to be retracted.

**Method lesson, recorded because it generalises:** the regression was invisible
to two prior rounds of review because both re-timed only the benchmark the fix
targeted.  Screening the corpus for *which translation units changed codegen* is
cheap (~70 s for 807 files) and names exactly which benchmarks are worth timing.
That screen is now a tool, and it should run before any runtime A/B of an
allocator change.

**Gates:** `cargo test --lib` 2388/0 (was 2386; +3 new, one later replaced by the
cap-counts pin), `ci_local.sh --fast` 24/0/3, clippy clean, rustfmt clean.

## RA-GLA-01 (2026-09-12) — Global Location Allocation Phase 1: environment contract and fail-closed knobs

**Code:** `src/backend/location_alloc/` (directory module:
`mod.rs` gate/run, `policy.rs`, `pressure.rs`, `planner.rs`,
`materializer.rs`, `verifier.rs`), gate in `src/driver/pipeline.rs`,
shared helpers in `src/backend/split_ranges.rs`. Full design and
calibration record in the phase-1 report and the S16 audit, both
digested in [`journal/2026-09-W2.md`](journal/2026-09-W2.md)
(GLA entry).

The feature originally shipped **off**; **source-less rematerialization
graduated to default ON under RA-GLA-02 below** (2026-09-12) — the env var
is now the emergency kill switch. Spill/intra-block gaps remain OFF. Every
knob is fail-closed in the safe direction: loosening a filter can only
ADD edits; numeric knobs are parsed once per process (`OnceLock`), an
unparseable value keeps the default, and every value is clamped to the
range below so no parse result can drive an unbounded plan.

### Master gate and budget

| Variable | Default | Clamp | Effect / fail-closed direction |
|---|---|---|---|
| `CCC_RA_GLOBAL_LOCATION` | **ON** (RA-GLA-02; was off) | unset/`1`/`on` enable; `0/off/no/false/empty` disable | Master gate / kill switch. Off = zero GLA edits. |
| `CCC_RA_GLOBAL_LOCATION_MAX` | 64 | 0..=4096 | Per-function edit budget (remats + gaps). 0 disables planning. Raising ADDS edits. |
| `CCC_PRESSURE_BUDGET` | 12 (x86-64) / 6 (i686) | 2..=64 | GPR color-class budget the planner and the intra-block pressure splitter treat as "colorable". Lowering ADDS edits. Shared with RA-06. |
| `CCC_PRESSURE_MIN_GAP` | 4 | 1..=256 | Minimum program points a gap must span (RA-06 helper). Lowering ADDS edits. |
| `CCC_PRESSURE_SPLIT` | off | presence-gated | Enables the intra-block pressure splitter; `CCC_PRESSURE_SPLIT_MAX` (default 64, unclamped) is its per-function budget. Skipped in a function GLA already split. |

### Planner policy knobs

| Variable | Default | Clamp | Effect / fail-closed direction |
|---|---|---|---|
| `CCC_GLA_REACH` | derived: Speed **10 (aarch64)** / 6 (x86-64, riscv64) / 2 (i686); Debug 64; Size 0 | 0..=64 | Salvageable excess over budget. Derived from the named buyable callee-saved GPR sets (`X86_64_BUYABLE_CALLEE_SAVED_GPRS` = rbx/rbp/r12–r15; `AARCH64_BUYABLE_CALLEE_SAVED_GPRS` = x19–x28; `I686_BUYABLE...` = esi/edi, with ebx reserved for the PIC GOT and ebp for the frame pointer). i686 stays at 2 despite theoretical non-PIC capacity: bands 3/6 measured 1.038×/1.053× on loop_patterns. RISC-V stays at 6 despite 11 usable s-registers: band 9+ remats loop-invariant constants *inside* hot loops (RA-GLA-03). Raising ADDS edits. |
| `CCC_GLA_REMAT_MAX_SEGMENTS` | 1 | 1..=1024 | Max hole-aware live segments a rematerialized value may have. Raising ADDS edits (multi-segment globals were measured net-negative: nbody format-string base). |
| `CCC_RA_REMAT_MAX_USES` | 3 | 1..=64 | Max dynamic use weight for a rematerialized value; hotter source-less defs keep a register. Lowering ADDS edits. |
| `CCC_GLA_MIN_BENEFIT` | 40 | 1..=1_000_000 | Minimum weighted benefit a gap must promise. Lowering ADDS edits. |
| `CCC_RA_GLA_RATIO` | 2.0 | 1.0..=10.0 (stored in tenths, 10..=100) | Required benefit/cost ratio for a gap. Lowering ADDS edits. |
| `CCC_GLA_SPILL_GAPS` | off (`0`/`false` disable) | bool | Capture-store/reload gaps. Shipped OFF: pre-allocation gaps measured net-negative everywhere because the proxy cannot see residency the colorer already resolves; retained as the post-allocation P0-B substrate. |
| `CCC_GLA_ALLOW_INTRA` | off (`0`/`false` disable) | bool | Allow gaps whose register pieces share a block. Shipped OFF per the RA-06 intra-block negative result. |
| `CCC_GLA_TRACE` | off | presence-gated (requires split debug) | Per-plan tracing; no effect on decisions. |
| `CCC_SPLIT_MAX` | 30 | unbounded | Pre-existing cap for the call-spanning live-range splitter (also consumed by RA-06's high-pressure splitter); unrelated to the GLA edit budget. |
| `CCC_DEBUG_SPLIT` | off | presence-gated | Tracing for the range/GLA splitters; no effect on decisions. |
| `CCC_VERIFY_REGALLOC` | off | presence-gated | Runs the fail-closed structural verifier after a GLA rewrite (and the production allocator); a violation aborts rather than emitting bad code. |
| `CCC_VALIDATE_SSA` | off | presence-gated | SSA validation gate around IR rewrites; diagnostics only, never changes a decision. |

Tier rules: `-O0` → Debug (coloring tier disabled; band effectively
unbanded, remat is direct stack-traffic relief); `-Os/-Oz` (opt levels
4/5) → Size, GLA plans nothing; `-O1..-O3` → Speed with the calibrated
band. At `CCC_RA_GLOBAL_LOCATION=1` the shipped Speed policy fires on
two -O2 programs in the current corpus: zlib_ng_adler32 main (−12 stack
references, spills 19→7, frame 56→40 bytes, +4 cold instructions — the
calibrated win) and strlen_bench main (3 globaladdr remats; identical
on pristine 0e4cf54, so it predates the phase-1 follow-up refactor; its
on/off program output is identical). The full multi-corpus static
census (`scripts/census_full_delta.sh`, gate enabled on BOTH compilers)
shows zero TU deltas from the follow-up refactor itself. Callgrind A/B
of the gated path (2026-09-12, final binary, gate on vs off at -O2):
zlib_ng_adler32 Ir **0.9855 (−1.45 %)**; strlen_bench Ir 1.00000 with
all simulated events identical (its three remats land in cold setup
outside the hot region) — the A/B path wins where the calibration
predicted and is neutral everywhere else it fires.

### Fail-closed id exhaustion

Fresh SSA ids come from `next_value()` (`split_ranges.rs`), which
returns `None` at `u32::MAX`. The materializer builds every edit in
local structures before its first IR mutation; an exhausted value- or
block-id space now aborts the plan with zero edits and a `[GLA] …
fresh id space exhausted` warning instead of panicking (`expect` was
removed from all three mint sites). `liveness::collect_values_and_allocas`
caps its capacity hint by real instruction content, so an inflated
`next_value_id` counter (upstream id leak) cannot reserve gigabytes.

**Pins:** `tests/regression/check_gla_remat_policy.sh` (2×2 segment/band
matrix), `location_alloc::tests::reach_band_equals_buyable_callee_saved_count`,
`location_alloc::tests::next_value_exhaustion_fails_closed`.

## RA-GLA-02 (2026-09-12) — GLA source-less rematerialization graduates to default ON

Supersedes RA-GLA-01's ship-OFF status for the source-less rematerialization
vocabulary only. Spill gaps, intra-block gaps, and the Size tier stay OFF
exactly as before; every other RA-GLA-01 knob and fail-closed direction is
unchanged.

**Gate semantics after this record:** `CCC_RA_GLOBAL_LOCATION` is an
emergency kill switch. Unset ⇒ ON; `0`/`off`/`no`/`false`/empty (any case)
⇒ OFF; anything else (incl. `1`/`on`) ⇒ ON.

### Fire census (what default-ON actually changes)

`scripts/gla_fire_census.py` over the compiler corpus (785 TUs ×
{-O0,-O1,-O2,-O3,-Os} × {x86-64,i686}), gate ON:

| opt | x64 TUs/edits | i686 TUs/edits | capture slots | aborts/hard failures |
|---|---|---|---|---|
| -O0 | 29 / 353 | 108 / 637 | 0 | 0 |
| -O1 | 26 / 150 | 61 / 145 | 0 | 0 (see tramps below) |
| -O2 | 22 / 37 | 78 / 172 | 0 | 0 |
| -O3 | 22 / 37 | 78 / 172 | 0 | 0 |
| -Os | 0 / 0 | 0 / 0 | 0 | 0 (`Tier::Size` plans nothing) |

Every applied edit is a source-less rematerialization
(`GlobalAddr` or `Copy`-of-`Const`, simple GPR types only): edits == remat
counts in every row. Zero capture slots, zero id-space aborts, zero
gate-induced hard failures on both targets. One fan-out edge trampoline
traced at i686 -O1 in `tests/regression/peephole_inline_asm_barrier.c`, a TU
that does not assemble for i686 even with the gate OFF (x86-64-only inline
asm) — the emitter therefore had no successful end-to-end coverage and
motivated the tests below.

### Static emitted-text deltas (gate ON minus OFF)

`scripts/census_full_delta.sh` (adds `--32` mode): totals over changed TUs.

| target/opt | changed TUs | Δ instructions | Δ stack refs |
|---|---|---:|---:|
| x64 -O0 | 24 | −473 | −258 |
| x64 -O1 | 4 | −60 | −15 |
| x64 -O2 | 7 | −22 | −46 |
| x64 -O3 | 7 | −22 | −46 |
| i686 -O0 | 86 | −559 | −835 |
| i686 -O1 | 34 | −129 | −151 |
| i686 -O2 | 40 | +6 | −284 |
| i686 -O3 | 40 | +6 | −284 |

Positive deltas were triaged one by one; each is the policy-model-gated
LEA/mov-for-stack-ref trade (benefit/cost ≥ 2.0, min benefit 40) or debug
tier slot churn, and every positive TU is output-equivalent on/off (see
below): i686 -O2 vectorization regression TUs spend ≤+21 ALU insns for
−18…−33 stack refs (`affine_map_vectorization` +21/−33,
`vex_promote_semantic` +18/−14, `phi_coalesce_folded_shl_index` +15/−12,
`arm_fp_homed_int_binop` +10/−18, `vectorize_map_expr_tree` +6/−30,
benchmark `matmul` +6/−2); i686 -O0 two synthetic GEP-chain TUs
(`gep_chain_fold_root_liveness`, `peephole_load_reuse_self_addr`) each
+58 movl/−19 stack refs in functions containing ~30 global clones — the
i686 -O0 aggregate is still −559 insns/−835 stack refs over 86 TUs; x64 -O0
`vec_signed_range_fusion{,_sse2}` −31 insns/+14 stack refs in debug SSE
slot reshuffles. No positive delta occurs in a hot benchmark loop at -O2.

### Runtime equivalence (deterministic, both targets, all opts)

`scripts/gla_equiv_check.sh` compiles+RUNS every `tests/regression/*.c`
(honoring `.flags`/`.env`), three repetitions per side for stability:
x86-64 **3465 stable pairs, zero divergence**; i686 **3317 stable pairs,
zero divergence**. Skips are honest baseline issues independent of the
gate: `simd_new_hw_ops` at -O0 -mavx2 is nondeterministic with the gate
OFF too (pre-existing AVX2 backend bug; stable and identical on/off at
-O1…-Os), and `unroll_unsigned_domain_trip` times out on i686 under gcc
-m32 as well (32-bit domain loops).

Callgrind (-O2, x86-64, `scripts/callgrind_ab.py`): 31-program fast corpus
geomean Ir mine/ref **0.99953**; zlib_ng_adler32 **0.98548 (−1.45%)**;
every other program exactly 1.00000 or within ±0.002% (≤14 Ir in one-off
setup, hot loops identical, I1/LLi/branch events identical). Heavy fire
sites strlen_bench and binary_trees Ir 1.00000. (Tables:
`results/callgrind-20260912-gla-default-on-o2.md` (wiped: ephemeral worktree table, never preserved).) -O0 heavy Callgrind is
impractical (>15.9 B Ir × ~30 instrumentation) and Debug-tier static
deltas above cover it.

### Edge-trampoline path: new coverage

- Unit: `remat_on_fanout_phi_edge_isolates_edge_with_trampoline` builds the
  fan-out φ shape, asserts exactly one singleton trampoline block (global
  clone + unconditional branch), edge-specific terminator retargeting, φ
  incoming rename/relabel, untouched other edges, and verifier acceptance.
- End-to-end: `tests/regression/gla_remat_trampoline.c` — two tuned
  functions so the path fires in successful builds on BOTH targets
  (`select_ptr_32`: i686 -O1/-O2/-O3; `select_ptr_64`: x86-64 -O1/-O2/-O3).
  All 16 target/opt/gate builds link and print `56862`, identical to gcc
  and gcc -m32. Assembly shows the isolated singleton (i686:
  `lea .data@GOTOFF` + `jmp` merge reached only from the true edge).
- Fail-closed hardening: an `IndirectBranch` predecessor computes its
  successor at runtime (blockaddress operand), so a trampoline could never
  be reached by retargeting the static edge list. The materializer now
  refuses edge service on such edges (φ incoming stays on the original
  source-less value; in-block clusters elsewhere in the value still
  remat), pinned by `remat_through_indirect_branch_edge_is_refused`. The
  shape never occurred in the census; the gate is now provably incapable of
  miscompiling it. `Switch` jump-table edges remain supported (codegen
  rebuilds tables from terminator labels at emission).
- Hygiene: appending trampoline blocks now also bumps
  `IrFunction.next_label`.

### Tooling and gates

New/updated: `scripts/gla_fire_census.py` (per-opt/--32 fire census with
hard-failure tripwires), `scripts/census_full_delta.sh --32`,
`scripts/gla_equiv_check.sh` (on/off runtime differential, stability
repeats, honest ELF32 handling); `run_regression_suite.sh` gains an
explicit GLA-off A/B arm at -O2; `check_gla_remat_policy.sh` now proves
default-ON on a firing TU and that every off token silences the pass.
Validation: `cargo test --lib` 2645 passed / 0 failed / 6 ignored
(`location_alloc` 30/30), rustfmt + clippy `-D warnings` clean,
`ci_local.sh --fast` 31/0/3; φ-CFG and m32 differential fuzzing 1000+1000
with the gate forced on, zero mismatches; gate on/off equivalence was
re-run with the final binary on both targets.

## RA-GLA-02 (2026-09-13) — source-less rematerialization default ON:
## hardening, four-target evidence, structural verifier, Godbolt validation

Supersedes RA-GLA-01 for the source-less rematerialization vocabulary
only: GlobalAddr and const-Copy clones graduate from gated-OFF to
default ON. Spill gaps, intra-block gaps and the Size tier stay OFF,
and `CCC_RA_GLOBAL_LOCATION` becomes the emergency kill switch
(0/off/no/false/empty disables; unset ⇒ ON). This record is the
graduation evidence and the hardening delta; the addendum below is the
second line-by-line red-team pass.

**Unconditional post-rewrite verification with byte-for-byte rollback.**
The structural verifier now runs for EVERY applied GLA rewrite with no
environment gate (the graduation rule: a default-on feature cannot rely on
an opt-in checker). Before the first mutation the materializer snapshots
`blocks`, `next_value_id` and `next_label`; on any violation it restores
all three verbatim, prints `[GLA] … rewrite failed structural verification
… aborting plan with zero edits`, and reports zero edits — exact gate-off
behavior. `CCC_VERIFY_REGALLOC=1` upgrades the warning to a panic for
development backtraces. The seven invariants are: (1) unique definition
sites; (2) every capture-slot reload dominated by its store (including the
in-block ordering); (3) fresh ids bounded by `next_value_id`; (4) φ nodes
remain a contiguous block prefix; (5) unique block labels and every static
terminator edge (Branch/CondBranch/Switch/IndirectBranch) resolving to a
real block; (6) each φ incoming names a real, distinct CFG predecessor;
(7) the φ incoming predecessor SET equals the block's CFG predecessor set
(set comparison, because `build_cfg` records both arms of a CondBranch
that targets one block). Empirically zero rollbacks over the complete
compiler corpus × {-O0..-O3,-Os} × all four targets (the fire census
`aborts` column is 0 everywhere below).

**Retarget fixes (red-team findings).** `retarget_edge` moved only one
CondBranch arm (`if/else if`) and stopped at a matching Switch `default`
without walking cases. A CondBranch whose two arms name the same block —
or duplicate Switch cases — are ONE unique CFG edge (`build_cfg`
deduplicates successor labels) with one shared φ incoming; a fan-out
trampoline must reroute every occurrence or the un-rewired arm enters the
merge expecting trampoline-defined names. Both arms / every matching case
are now moved together; the Switch/default ordering is also corrected.
Pinned by `retarget_edge_moves_both_equal_cond_arms_together`,
`retarget_edge_moves_all_duplicate_switch_cases`, and
`remat_on_switch_fanout_phi_edge_isolates_edge_with_trampoline` (full IR
shape: case edge isolated, default edge untouched, φ rewired, jump-table
emission references the appended block purely by symbolic label, which
the block-iteration emission at `generation.rs` resolves like any other
block). `clone_remat`'s silent catch-all (which would have cloned a
template with the OLD destination) is now an `unreachable!` documenting
that only `GlobalAddr`/const-`Copy` are ever certified — re-reading memory
is not a sound rematerialization (the store may have happened since).

**Knob vocabulary harmonized.** The experimental shipped-OFF switches
`CCC_GLA_SPILL_GAPS`/`CCC_GLA_ALLOW_INTRA` parsed only `0`/`false`; an
emergency `=off`/`=no`/`=FALSE`/empty would have ENABLED them. All boolean
knobs now parse through one `is_off_token` helper with the master gate's
full vocabulary (unit: `token_tests`).

**Harness defects fixed.** `check_gla_remat_policy.sh`'s negative
assertions were written as `! grep …` under `set -e`: negated commands are
exempt from errexit, so every "must not apply" assertion was vacuous
(including the default-policy nbody rejection). They are explicit
`must_not_grep` helpers with diagnostics; the off-token sweep now covers
`OFF/NO/False` as well, and the master-gate proof uses a known-firing TU
for every token. The equiv runner's ELF32 smoke probe entered `int $0x80`
with undefined registers and could never observe exit 42 (it segfaulted,
which hid the decision between native and qemu-i386); the probe now sets
eax=1/ebx=42. The fuzz harness passed multi-word runners
(`qemu-riscv64 -L …`) as one argv element, making every cross synthetic
case die with SIGHUP and get mis-filed as GEN-BUG; runners are now
`shlex`-split (this unlocked real RISC-V fuzzing). GCC include paths are
resolved via `-print-file-name=include` everywhere (fire census, callgrind
A/B) instead of a hardcoded gcc-14 directory.

**Module-wide cleanup coupling (the i686 positive-insn cases, root
cause).** When ANY splitter edits ANY function, the pipeline's
`did_split` flag runs copy propagation + DCE over every function in the
TU. GLA merely became a new trigger for this pre-existing, sound cleanup.
On i686 matmul -O2, GLA edits only the standalone `matmul` kernel; the
`main` +6 insn / frame 60→44 change is that cleanup re-materializing two
cold double constants around printf setup, not a GLA rewrite — proven by
`CCC_RA_GLOBAL_LOCATION=1 CCC_GLA_REACH=0` (zero GLA edits) assembling
byte-identical to gate-off. Runtime measured neutral-to-positive
(matmul 0.992× in RA-GLA-01's paired run); the pre-existing coupling is
intentionally left untouched (refactoring it changes all three splitters,
out of scope for graduation).

**Four-target fire census (rebase a361f26, 794 TUs × 5 opts, gate ON):**

| target | firing TUs | edits | remat | capture slots | trampolines | rollbacks | gate-induced build failures |
|---|---:|---:|---:|---:|---:|---:|---:|
| x86-64 | 106 | 586 | 586 | 0 | 3 | 0 | 0 |
| i686 | 335 | 1170 | 1170 | 0 | 4 | 0 | 0 |
| aarch64 | 128 | 496 | 496 | 0 | 3 | 0 | 0 |
| riscv64 | 158 | 780 | 780 | 0 | 3 | 0 | 0 |

**Godbolt oracle cross-validation** (live Compiler Explorer audit
`scripts/godbolt.py audit`: gcc 16.2 / clang 23.1 / icc 2021.10 / icx
latest all current; local gcc 14.2 is NOT used for oracle claims). Static
per-function stats via `scripts/codegen_oracle.py`:

* x86-64 zlib_ng_adler32 `main`, -O2: gate-off 242 insn / 40 loads / 6
  stores / **19 spills** → gate-on 242 / 40 / 4 / **7 spills**; oracles
  gcc16.2 156/20/3/2, clang23 148/30/0/0, icc 73/8/5/9, icx 147/23/4/3.
  GLA closes part of the spill gap with zero added instructions; the
  residual 7 are computed (non-source-less) values — out of the remat
  vocabulary, P0-B territory; the trace confirms the remaining
  global/const candidates fail the calibrated use-weight/segment gates
  for good reason (v57 50 uses, v819 220 uses/2 segments).
* aarch64 zlib_ng_adler32 `main`, -O2: 388/66/35/**80 spills** →
  364/46/20/**45 spills** vs ARM64 gcc 16.1 126/20/5/6.
* riscv64 same TU: 711/71/37/6 → 707/58/25/6 vs RISC-V gcc 16.1
  151/22/7/10.
* x86-64 strlen_bench: on/off text-identical (three cold setup remats,
  Callgrind Ir 1.00000).
* aarch64 sha256_transform: on/off function text identical (the one
  remat is a relocation of a 1-use globaladdr; recurrence-carried φ
  accumulators remain protected).

The inherited x86-64-derived budget/band on the 30+ GPR targets was
therefore validated as fail-safe in the right direction: it fires only at
genuine pressure extremes and the measured changes are large spill
reductions or neutral. The follow-up per-ISA calibration that this
paragraph left open has since been done (RA-GLA-03 below): AArch64 moves
to the ABI-derived band 10 with static and oracle-confirmed cold-code
wins and zero benchmark-program changes, while RISC-V stays at 6 because
the band-9+ frontier demonstrably loses to the oracle (hot-loop constant
remats).

**Runtime / performance evidence:** Callgrind -O2 31-program fast corpus
rerun on the rebase: geomean Ir mine/ref **0.99952**, zlib_ng_adler32
**0.98548 (−1.45 %)**, every other program 1.00000 or within ±0.002 %
with I1/LLi/branch events unchanged (`results/callgrind-a361f26-o2.md`, wiped: ephemeral worktree table, never preserved).
Valgrind clean on the compiler while firing and on the resulting
trampoline/adler binaries. Differential fuzz with the gate forced on:
phi_cfg 1000/1000, x86-64 differential 600/600, i686 m32 1000 seeds
(~3000 case-arms), aarch64 400 cases, riscv64 synthetic 400/400 — zero
mismatches. The equiv runner now compiles/runs an independent gcc oracle
best-effort in EVERY mode (host gcc, gcc -m32 under qemu-i386, cross gcc
under qemu-user); cases where both gates agree but neither matches gcc
are classified as pre-existing backend gaps and reported separately, so
they never masquerade as gate divergences.

**Test/CI state:** `location_alloc` 40 unit tests (was 30 at graduation);
`cargo test --lib` 2716/0/6; clippy `-D warnings` and rustfmt clean;
`ci_local.sh --fast` 31/0/3 with the codegen golden gate within tolerance.

### Addendum — second red-team pass on the final binary (2026-09-13)

A full line-by-line re-audit of planner/policy/materializer/verifier
plus the definitive four-target matrix on the final binaries produced
five additional hardenings, with no production behavior change for the
shipped (remat-only) policy:

1. **Indirect-branch edges, exact fail-closed semantics.** The edge
   planner already refused to trampoline an `IndirectBranch` predecessor
   (the jump target is a runtime blockaddress). Two refinements: (a) when
   such an edge also carries a rename that IS fully defined before the
   terminator (a name already emitted in the predecessor), that rename is
   now applied in place instead of being discarded with the deferred set;
   (b) `retarget_edge` no longer has an arm that rewrites
   `IndirectBranch.possible_targets` — the edge planner never creates a
   trampoline there, and rewriting the table would not redirect the
   computed jump. Pinned by
   `retarget_edge_never_rewrites_indirect_branch_targets` and the existing
   `remat_through_indirect_branch_edge_is_refused` (which asserts both the
   φ incoming and the target table are untouched).

2. **Deterministic emission order, no dependence on FxHash table layout.**
   The split value worklist (`gap_ids ∪ remat_ids`) and the capture-slot
   alloca insertion iterated `FxHashMap`/`FxHashSet` collections; the
   multi-definition fan-out trampoline body and the entry alloca order
   therefore depended on table layout. Both are now sorted by value id
   before emission, matching the deterministic ordering every other event
   stream already uses (events sort by point/vid; candidate selection
   tie-breaks on vid; blocks and tramps are built in index order). Pinned
   by `trampoline_multiple_defs_are_sorted_by_value_id` (two globals
   deferred onto one fan-out edge: one shared trampoline, clones in
   ascending id order).

3. **Equivalence harness: target-incompatible TUs are SKIPped and
   argv[0] is fully controlled.** The four-target matrix found two
   classes of false "output divergence" on byte-identical gate binaries:
   (a) `builtin_avx256_raw` under qemu-aarch64 carries an
   `-mavx -mavx2` sidecar that aarch64 cross-gcc rejects outright (the
   x86-only vector ABI test is meaningless on aarch64); the program also
   reads uninitialized stack and its garbage output under qemu-user
   varies with the **argv[0] layout** (proven: copies of one binary named
   `n2` vs `aaaa` print different denormals). Such TUs are now SKIPped
   when the cross oracle cannot compile them rather than comparing
   undefined-behavior output. (b) `arm_f128_param_preserves_gp_param`
   on riscv64: both gate binaries are byte-identical and both fail to
   run because lccc's riscv64 link lacks compiler-rt (`undefined symbol:
   __trunctfdf2`), and the loader prints the program path TWICE in its
   diagnostic — so differing artifact names manufacture a diff even at
   equal basename lengths. The harness now runs every binary (both
   gates and the oracle) with one fixed argv[0] (`lccc-equiv-prog`, via
   qemu-user `-0`, native via `exec -a`) and scrubs the work directory
   from captured output; equal-length artifact names remain as a third
   layer. The loader-failure case then classifies correctly as a
   gate-independent pre-existing oracle gap rather than a divergence.
   Host and `-m32` modes keep the oracle-best-effort semantics;
   shellcheck and `bash -n` clean.

4. **DWARF source-span 1:1 discipline in the block sweep.** Materializing
   events rebuilds each touched block's instruction vector; previously
   `source_spans` was left untouched, so a spanful block (one entry per
   instruction, which `generation.rs` walks when emitting `.debug_line`)
   desynchronized after the first inserted clone — every following
   instruction could be attributed to the wrong source line. The sweep
   now mirrors the established `split_ranges::insert_instruction`
   precedent: when a block was spanful it stays exactly spanful, each
   inserted event clones the span of the instruction it executes before
   (the last span for terminator-boundary events), originals keep their
   own spans, and a `debug_assert!` pins the 1:1 correspondence. Snapshot
   rollback already restores spans wholesale (they live in `blocks`).
   Pinned by `inserted_remats_keep_source_spans_one_to_one`.

5. **Doc-comment hygiene:** the shared `is_off_token` doc comment was
   attached to the wrong item after the earlier insertion reorder; fixed.

The i686 `loop_patterns` calibration site was re-checked on the final
binary: GLA still fires there (one `globaladdr` remat in `main`) but the
gate-on/gate-off emitted text is identical — the band-2 calibration keeps
excluding the historical counter-spill edit; oracle A/B remains
209/51/49/70 spills under both gates.

**Rebase note (2026-09-13, upstream main `0ad4633`).** The addendum was
rebased across PR #522 (S38: class-agnostic GEP CSE keys, x86-backend
GlobalAddr rematerialization through `Copy`, static-chain call marker
protocol, call-liveness ABI refinements). S38 changes only GVN and
post-regalloc x86 codegen — all strictly DOWNSTREAM of GLA's pre-regalloc
IR rewrites — and touches no `location_alloc/` file. Its backend
Copy-remat is deliberately NOT mirrored into the IR-level planner: GLA
is target-independent across all four targets, while the S38 remat is
x86 text/MachInst specific; admitting `Copy <- GlobalAddr` to GLA's
template set would be a new four-target policy needing its own census.
The rebase changed no GLA fire site (see census below), and the
equiv matrices remain failed=0. The rebase also repaired a documentation
defect from the graduation commit: the `## ALIGN-01` section header had
been overwritten by the RA-GLA-02 insertion, orphaning the whole
tight-loop record under the wrong section; it is restored, and the
RA-GLA-02 / addendum heading levels are fixed.

**Test/CI state after addendum:** `location_alloc` **45** unit tests;
`cargo test --lib` **2722/0/6** (one upstream-added test); clippy
`-D warnings` and rustfmt clean. Rebased fastbuild binaries md5:
x86-64 `aa78c076…`, lccc-i686 `f5da2fa2…`, lccc-arm `5507f212…`,
lccc-riscv `bdabce5d…` (all four targets rebuilt via
`scripts/build_lccc_fast.sh`).

**Definitive gate on/off matrices (rebased binaries, five opt levels
each), all `failed=0`:**

| target | ran | oracle-confirmed | gate-indep. gaps | skipped | rc |
|---|---|---|---|---|---|
| x86-64 | 3499 | 3408 | 7 | 21 | 0 |
| i686 | 3337 | 3103 | 70 | 48 | 0 |
| aarch64 | 2687 | 2647 | 40 | 216 | 0 |
| riscv64 | 2698 | 2584 | 114 | 522 | 0 |

The i686 counts move by 2 ran / +4 gaps / −2 skips vs the pre-rebase run
(S38 text-path changes flip two unstable/skippable cases into
gate-independent gaps); all other targets are identical. The seven
x86-64 gate-independent gaps are the same hand-verified set as before
graduation (asm_alternative_length_template -O1, bitop_nonneg_zext -O0,
bitops_builtins -O0/-O1/-Os, deflate_setparams_cmp_cast -O1,
glibc_const_p_strlen -O1); cross-target gaps are cross-ABI / missing
compiler-rt link shapes present under BOTH gates.
Logs: `results/rerun-rebase-20260913/equiv-{x64,m32,arm64,riscv64}.log`.

**Fire census on the rebased tree (gate forced on, 794 TUs × 5 opts,
`aborts=0`, capture slots 0 everywhere):** x86-64 106 TUs / 586 remats /
3 trampolines; i686 335 / 1170 / 4; aarch64 128 / 496 / 3; riscv64
158 / 780 / 3 — bit-identical to the pre-rebase census, proving S38 did
not move any GLA decision (`results/rerun-rebase-20260913/fire-*.json`).

**Whole-corpus static emitted-text delta (gate OFF→ON, rebased
binaries):** x86-64 **−450 instructions / −342 frame references** (S38
improves the pre-rebase −436/−341 by another 14 insn/1 stkref; the lone
+insn row is the documented `-m32` TU at -O0 trading one compare for a
56→40-byte frame); i686 **−570 / −1645** unchanged (the matmul `main`
+6/-2 row remains the documented, runtime-neutral copy-prop coupling).
Logs `results/rerun-rebase-20260913/static-{x64,m32}.log`.

**Gate-forced-ON fuzzing on rebased binaries, zero mismatches:**
phi_cfg 1000/1000 (363 s), differential 600/600 (203 s), m32 forwarded
engine 1000 seeds × 3 opts ≈ 3000 case-arms (267 s), aarch64 400 seeds ×
{O2,Os} ≈ 800 case-arms (69 s), riscv64 synthetic 400 × -O2 under
qemu-riscv64 vs riscv64 cross-gcc (48 s). Note the synthetic engine
requires an explicit `--runner "qemu-riscv64 -L
/usr/riscv64-linux-gnu" --refs riscv64-linux-gnu-gcc`; without them the
lccc binaries are not executable on the host and every case reports a
spurious rc=−1 "mismatch" — a harness-config failure, not a compiler one.

Valgrind (`--error-exitcode=99 --leak-check=full`) on gate-on and
gate-off zlib_ng_adler32 binaries is clean; lccc-ld linker fuzz gates
(128 linker mutants + 64×7 ELF grammar links) report zero defects.

## RA-GLA-03 (2026-09-13) — per-target reach bands: aarch64 6 → 10, RISC-V kept at 6 with evidence

**Question.** RA-GLA-02 shipped every 64-bit target the x86-64-derived
Speed reach band of 6. The band proxies residual register capacity the
production colorer can still bring to bear beyond the GPR scan budget
(12). Both non-x86 64-bit targets have materially larger register files
(~27 usable AArch64 GPRs, 31 RISC-V GPRs with s1–s11 callee-saved), so
the question is whether their bands should widen to match the actual
callee-saved GPR sets.

**Method.** Static screening of every TU in benchmark/{programs,
kernel_corpus,patterns} + regression at -O0..-O3,-Os with gate off vs
gate on for reach bands 6, 7, 8, 9, 10, 11, 12, 14, 22
(`work/band_sweep.py`, `work/band_detail.py`, `work/band_knee.py` — uncommitted scratch scripts, wiped; <!-- dl-skip -->
`.flags` sidecars honored as the real harness and census do), Godbolt
per-function oracle comparisons for every TU whose deltas move with the
band, and per-function assembly attribution of each marginal change.

**AArch64: widen to 10 (x19–x28).** The marginal blocks bands 7–10
admit are *frame-setup / callee-save-home* pressure, not hot-loop work.
Representative case, the N=17 double matmul TU at -O2: band 8+ removes
the const slot round-trip (`mov x0,#136; str x0,[sp]; …; ldr x0,[sp]`
→ rematerialized `mov` at the single use), shrinks the frame
128 → 112, −1 load/−2 stores; the hot FP FMA loops are byte-identical.
Godbolt rank (`codegen_oracle.py --arch aarch64 --function main`,
carm64g1610/carm64gtrunk/cclang2210) moves 252 → 246 insns (2.02× →
1.97× of the 125-insn GCC leader), spills 52 → 50: same direction as
the oracle, zero counter-regression. Sidecar-aware whole-corpus census
(off → on), numbers after the PR #526 block-layout rebase (main
5ef3fe5): band 6 gives -O1 −122/−93, -O2 −691/−1096; band 10 gives
-O1 −163/−123, -O2 −721/−1120 (same at -O3), i.e. a marginal
**−71 insns / −52 sp-refs at -O2/-O3 and −41/−30 at -O1** over 12 more
touched rows (the pre-rebase build measured −96/−76 aggregate; PR #526
shifted the baselines slightly, the conclusion does not move). The only
new positive rows are +1 insn/opt in
`gep_chain_fold_root_liveness.c`, the dedicated long-GEP-chain root
liveness torture TU (one extra reload among duplicated adrp+add
materializations, characterized, no sp-ref change). -O0/−Os are flat
(−165/−95 at -O0, −4 at -Os). The empirical knee is sharp and was
re-confirmed after that rebase: band 11 starts rematting the REAL
benchmark `conv_u8_3x3` for **+9 insns/+5 sp-refs** (a cold-setup
cascade, the same failure shape RISC-V hits at band 9), and band 12
trades `loop_memset_fill_basic`'s stack savings away; the ABI-derived
10 is therefore also the measured maximum. The marginal
benchmark-program rows at band 10 (sha256_transform −71/−22,
sqlite_varint −77/−56, zlib_ng_adler32 −24/−35, strlen −10/−19,
binary_search −10/−7) all already fire at band 6, and NO
benchmark-program TU textually changes between bands 6 and 10 at any
opt level (verified over programs/kernel_corpus/patterns).

**RISC-V: keep 6, against the raw ABI count.** The SysV ABI preserves
s0–s11 (11 usable s-GPRs, s0 being the frame pointer), so a mechanical
ABI derivation suggests 11. The data rejects it: (1) the blocks bands
9+ admit are dominated by *loop-invariant constants*, which on this
backend the production allocator keeps resident in s-registers for
free — band 9 remats the LCG generator constants 1664525/1013904223
*inside* the hot loop of `double_reduction.c` (two 32-bit `li` pairs
re-emitted per iteration; band 6 keeps them resident in s9/s10 with a
single hoisted sd/ld pair), +5 static insns / −3 one-time sp-refs;
Godbolt whole-TU comparison moves 299 → 304 insns, i.e. 4.04× → 4.11×
of GCC's 74-insn solution — **away from the oracle**. (2) The only
sizeable static win beyond 6 (`outer_loop_shapes` −12/−12) enters at
band 11 in the same step as the hot-loop regression; bands 7–8 move
only synthetic TUs by ±2 insns with no benchmark-program row at all.
The conservative band therefore stays; the planner's weighted-use gate
(which would ideally refuse these hot constant remats directly)
undercounts these const-feeding-φ shapes, and the reach band is the
load-bearing backstop — recorded as a known limitation, not worked
around. A deeper red-team pass proved there is no planner-visible
discriminator that would harvest the band-11 wins safely:
`outer_loop_shapes` at band 11 applies exactly TWO cold weighted-1
single-segment GlobalAddrs (b0..b2, setup) for −12 insns/−12 sp-refs,
while `double_reduction` at band 9 applies the SAME class (THREE cold
weighted-1 single-segment GlobalAddrs in b0..b2) and the downstream
colorer's global reassignment then evicts the LCG constants for +5
insns. The candidate plans are near-identical at the point GLA decides;
the outcome difference lives entirely in allocator coloring the
pre-pass cannot observe (the same structural reason pre-alloc spill
gaps measured net-negative, RA-06). Any rule separating the two would
have to name those programs — a forbidden benchmark-specific rule.
The empirical band boundary is therefore the honest general backstop;
post-allocation feedback (the P0-B gap substrate) is the recorded
proper path to ever harvesting the outer-loop wins.

**Fail-closed accounting.** The band is selected at runtime from
`target_elf_machine()` (set by the driver pipeline for every compile,
before any codegen): AArch64=183 → 10, ptr-32 (i686=3) → 2, everything
else (x86-64=62, RISC-V=243) → 6. The thread-local machine defaults to
62, so any path that forgot to initialise it would read the x86 band
and *under-fire* on AArch64 — the wide band can never be selected
off-target. `CCC_GLA_REACH` overrides for A/B exactly as before; the
Debug tier remains unbanded (64) and the Size tier inert.

**Validation (re-validated on the PR #526 block-layout rebase, main
5ef3fe5, 2026-09-14).** Rust unit coverage extends
`reach_band_equals_buyable_callee_saved_gprs` to a per-ELF-machine table
(x86-64 6, aarch64 10, riscv64 6, i686 2, Debug 64, Size 0), and
`tests/regression/check_gla_cross_reach_band.sh` (a new `ci_local.sh
--fast` gate, hermetic under an ambient `CCC_GLA_REACH`) pins the arm
frame win, the riscv band-11 cliff, and the x86 default end to end.
Gate-on/gate-off/cross-gcc equivalence matrices under qemu-user:
aarch64 ran=2687 (2647 oracle-confirmed, 40 known pre-existing gaps)
failed=0; riscv64 ran=2698 (2585 oracle-confirmed, 113 gaps) failed=0;
x86-64 ran=3499 failed=0; i686 ran=3338 failed=0. Differential synthetic
fuzz 500 seeds × {-O1,-O2,-Os} on each cross target (seed 20260914):
zero mismatches. A second, host-side round (same seed) ran the
`phi_cfg` engine across 1200 seeds × {-O0,-O1,-O2,-O3,-Os} and the
`differential` engine across 800 seeds × {-O1,-O2,-O3}: 2000/2000
PASS, 0 mismatches. A dedicated structural gate
(`check_gla_backedge_no_trampoline.sh`, also a `ci_local --fast`
gate) constructs a loop-latch φ fed by the global base on the
looping edge and a distinct p-derived incoming on the forward edge
under heavy pressure, forces every admitting knob open (reach 999,
segment cap 1024, weight cap 64), and requires **0 back-edge
trampolines** on x86-64 (-O0..-O3), aarch64, riscv64 and i686 plus
byte-identical gate-off/forced-wide output — the permanent encoding
of the red-team result that the latch-edge remat/trampoline shape is
unreachable by construction (φ-feeding weight ≥ 10 > cap 3, and the
one-live-segment cap rejects the weaving form). Static off→on census (`.flags`-aware): the only new
positive rows aarch64 bands 6→10 introduce are +1 instruction/opt in
`gep_chain_fold_root_liveness.c`; x86-64 totals −455 insn/−342 stkref
(31 rows, one +insn row), i686 −572/−1627 (211 rows, same pre-existing
trade population as RA-GLA-02). The aarch64 fire census at the new
default is bit-for-bit the pre-rebase plan: 544 remats / 0 capture
slots / 0 aborts; the one non-harness forward-edge trampoline site
(`narrow_shift_count_ge_width.c` -O1, a ten-remat depth-0 switch-arm
function) emits no extra asm block or branch after layout, leaves
frame/insn/sp-traffic counts identical, and is oracle-confirmed.
A remat-weight-cap sweep on AArch64 (caps 3..8, reach 10) is a negative
result: caps ≥4 introduce synthetic +5/+9 counter-rows and shrink
existing wins while moving ZERO benchmark-program TUs, so the cap
stays at 3 on all targets. The arm regression runner reports 528 pass
with 56 failures that are byte-identical gate-off (x86-only inline asm
and documented backend gaps); GLA is exonerated. Host kernel runtime
A/B (`scripts/bench_kernels.py`, -O2 v3, 15 best-of reps, taskset, idle
machine): all nine `bench_run` codegen hashes are identical gate
on/off (hot bodies untouched; geomean 0.997×, every ratio 0.99–1.02,
the single sub-0.99 row carries an identical body hash and is the
harness's documented layout/timing noise). Valgrind on the compiler
while firing: 0 invalid accesses, 0 definite leaks; the 140 "possibly
lost" reports are gate-identical LazyLock builtin-table teardown
false positives. The linker suite (`tests/linker/run_linker_tests.py`
via the driver) is 206 pass / 0 fail / 1 environment skip.

## ALIGN-01 (2026-09-12) — structural hot-loop alignment ("tight loops"): two-layer GCC 16.2 mirror with exact encoded spans

**Code:** structural audit in `src/passes/loop_align.rs`
(`audit_tight_loop_inner`, `tight_bucket_log2`, `TightLoopMode`),
private marker `.lccc_tight_loop` parsed in
`src/backend/x86/assembler/parser.rs`, emitted in
`src/backend/generation.rs`, resolved by the branch-relaxation fixed
point in `src/backend/elf_writer_common.rs`; the integrated-assembler
gate lives in `src/driver/pipeline.rs`. Research tooling:
`scripts/tight_loop_oracle.py` (GCC 16.2 / Clang / ICX census on Godbolt),
`scripts/align_size_calibrate.py` (ground-truth GNU-as span validator),
`scripts/callgrind_ab.py` (deterministic A/B), gate
`tests/regression/check_tight_loop_align.sh`.

### What the oracle actually does (gcc/config/i386/i386-features.cc,
`ix86_align_loops`, read in full)

The pass bails unless `TARGET_ALIGN_TIGHT_LOOPS && optimize &&
optimize_function_for_speed_p` and the tuning cost has a non-zero
`prefetch_block`. Static-guess mode qualifies a loop by predicted
iteration counts (header fallthrough vs. taken counts against
`align-threshold`=100 and `align-loop-iterations`=4). It then walks the
basic blocks of ONE loop contiguously from the header, requiring the
same `loop_father` throughout, summing `ix86_min_insn_size` (the
**minimum** encodable instruction length), rejecting inline asm and
real calls (size −1), stopping at the first edge back to the header,
rejecting an unconditional transfer before that edge and any second
conditional branch. Success emits, immediately before the header label,
`gen_max_skip_align(ceil_log2(size), 0)` — an UNCONDITIONAL
`.p2align K` — and the normal `.p2align 4,,10` / `.p2align 3` cascade
follows. The RTL comment states the goal: the whole loop fits one
instruction-cache line / fewer DSB misses.

### lccc's two layers

1. **Structural audit (IR, post-layout).** Mirrors the RTL structural
   half: innermost loop in the natural-loop nest; header physically
   first and the body contiguous in emission order; last body block is
   the latch; no call / call-indirect / inline asm / trampoline init /
   nonlocal transfer; at most one conditional branch before the latch;
   a header compare proving ≤4 trips excludes the loop (the same
   `-param=align-loop-iterations` bound). Only x86-64/i686 builds going
   through the integrated assembler are eligible; `-S` and the optional
   external-GAS toolchain never see the private marker and keep the
   portable bounded cascade, as do `-Os/-Oz` and `-O0`.
2. **Exact-span bucket (assembler).** The marker is resolved inside the
   existing jump-relaxation / marker-fixup fixed point: span = header
   label → end of the first backward branch to it, measured in the exact
   encoded bytes of the settled layout. Bucket =
   `ceil(log2(span))` clamped to 8/16/32/64 (`tight_bucket_log2`, the
   single shared table); an empty body or a span > 64 B fails closed to
   the cascade. Re-derived on every sweep, so a deferred `.skip` that
   grows the body past 64 B after an earlier acceptance revokes BOTH the
   padding and the section-alignment raise
   (`reconcile_section_alignments`), which earlier prototypes leaked.

**Deliberate divergence from GCC, same hardware goal.** GCC's size is a
*minimum-encodable* estimate; lccc buckets the *actual* encoded span.
The stated purpose is "the whole loop fits one cache line", and the
minimum-size estimate demonstrably under-measures real encodings (the
old in-tree linear opcode estimator mis-bucketed ≈25 % of corpus
loops). `align_size_calibrate.py` builds marker-free assembly, assembles
with GNU as, measures true spans, and compares with every writer
decision: **0 bucket mismatches, 0 span disagreements, 0 cache-line
leaks, 0 missed promotions** on the full 51-program corpus for both
x86-64 (132 candidates) and i686 (131). We do NOT switch to minimum-size
estimates to chase per-label GCC equality: generated CFGs differ
between the compilers, and the corpus-wide rule behavior is what
matches.

### Full-corpus oracle census (2026-09-12, -O2, -march=raptorlake)

Strongest unconditional `.p2align` log2 per backward-branch header over
`tests/benchmark/programs/*.c`: GCC 16.2: 2^3:83, 2^4:12, 2^5:49,
2^6:38 (plus 167 bounded-cascade groups); Clang 23.1: 2^4:269; ICX
2^4-heavy; lccc: 2^3:248 (cascade tail), 2^4:22, 2^5:51, 2^6:47, with
31 over-64-B bodies refused to the cascade. Strong-tier coverage is
therefore GCC 87 (49@32 + 38@64) vs lccc 98 (51@32 + 47@64): the same
shape at the same frequencies; exact sizing moves near-boundary bodies
up one tier (the faithful decision for the one-cacheline goal) and the
writer refuses bodies GCC's estimator under-sizes. sqlite_varint is the
worked label-mismatch example: GCC aligns three loops the lccc CFG
marks for documented reasons (call/inline-asm, unconditional transfer
before the latch, outer loop) while lccc promotes the one inner
digit loop; same rules on different CFG shapes, not a policy bug.

### Deterministic A/B (Callgrind, DEFAULT_FAST corpus vs pristine 0e4cf54)

Geomean executed instructions mine/ref = **1.00037 at -O2 and -O3**
(identical tables), **0.99999 at -Os** (fully inert; sub-1e-5 noise on
118k-Ir micro-benchmarks). Five benches show small Ir INCREASES
(tls_seg_access 1.0055, linux_find_bit 1.0031, expat_xml_scan 1.0017,
sqlite_varint 1.0010, sha256_transform 1.0004): the unconditional pad
can sit on the loop-guard FALLTHROUGH executed once per entry, and
Callgrind Ir counts the NOPs while being blind to the DSB/decode win the
alignment buys. Every one of the five loops is tight-aligned by GCC 16.2
with the same (or stronger — tls_seg_access is an exact `.p2align 6`
match) unconditional directive; padding is amortized over each loop's
trip count (tls: 63 inner iterations/entry). sqlite_varint additionally
cuts conditional-branch-mispredict counts 6.06 M → 5.22 M (confirmed again
in an isolated policy-on vs `CCC_LOOP_ALIGN_HOT=off` run, which reproduces
the exact same table). The pre-feature baseline geomean is 0.99322 with
worst-case sha256 0.824; no benchmark shows an Ir regression beyond the
oracle-matched padding.

### Knobs and fail-closed contract

`CCC_LOOP_ALIGN_HOT` (A/B ladder, default `gcc`): `gcc` = exact-span
policy; `off` = bounded cascade only; `5`/`6` = force unconditional
32/64 + cascade (resolved at IR level, visible under `-S`);
`5skip` = bounded 32 with a 16 fallback. An unrecognized value warns
and falls back to `gcc` (never pads more). `CCC_DEBUG_TIGHT` prints the
span/bucket/reject per fixup sweep; `CCC_DUMP_ALIGN` prints structural
verdicts; `CCC_NO_LOOP_ALIGN` disables the whole pass. The marker parser
rejects a malformed directive with a hard error (a private directive
that silently no-ops would just lose alignment). Relaxation interaction
is covered by `tight_loop_padding_and_relaxation_reach_fixed_point`
(marker padding can re-grow an outer short backedge to long form and
the fixed point must converge).

### Validation matrix (2026-09-12)

Full library `cargo test --lib`: 2627 passed / 0 failed / 6 ignored
(incl. 13 new `loop_align` policy tests and 8 tight writer/parser
tests); `location_alloc` 28/28; rustfmt clean; clippy
`-D warnings` clean for lib and `--tests`; `scripts/ci_local.sh --fast`
green incl. the new gate. Deterministic fuzzing: phi-CFG 1000/1000 and
m32 differential 1000/1000 programs, zero mismatches. Output
equivalence vs the pristine compiler: 51/51 programs byte-identical on
both x86-64 and i686. Debug-assertion-enabled build (fastbuild with
`-C debug-assertions=on`): 510 corpus compiles (51 programs × 5 opt
levels × both targets) and 707 regression/bug compiles assert-clean;
this sweep also found and fixed one pre-existing, unrelated
`1u64 << 64` overflow in `loop_memset.rs`'s Ne-trip-count gate
(64-bit compare type). GNU-as byte-equivalence suites: i686 20/20,
x86-64 failure set identical to pristine main (zero regressions).

## S25 (2026-09-24) — F32/F64-typed ternary merge slots + VEX 4-operand blend retarget

**Code:** `src/ir/lowering/expr_ops.rs` (`ternary_merge_slot`, `emit_ternary_merge_store`),
`src/backend/x86/codegen/float_ops.rs` (S05 blend, `retarget_vex_result`), `src/ir/constants.rs`
(`narrowed_to` now handles F32↔F64 cross).

**Rationale.** LP64 lowered every scalar ternary through an I64 slot, so
`float`/`double` selects became I64 selects: GPR-homed by RA, starving the
S05 FP-select blend which needs an XMM dest or XMM false arm, degrading to
GPR cmov + movq round-trips. Exact-typed slots keep FP merges XMM-homed end
to end; blends/cmovs select bit patterns, never arithmetic values, so NaN
payloads survive.

**Soundness.** `ternary_merge_slot` returns exact type only for F32/F64
(and int widths that already match target int). `emit_ternary_merge_store`
now converts float constants via `coerce_to` (F64→F32, F32→F64) so the
Store's value type matches its slot type — previously it only narrowed
integers, leaving a type-incorrect Store when a constant of the other
float width reached the slot. `narrowed_to` extended similarly for mem2reg.
`retarget_vex_result` admits 4-operand variable blends (`vblendvps`): VEX
reads all sources before writing, so retargeting dest onto mask is safe;
upper-half protection via existing full-width reader check.

**Ordering / knobs.** No new knob; existing S05 blend gate benefits.
`CCC_DISABLE_PASSES` does not gate this lowering (it is part of expr
lowering, not an opt pass).

**Measured evidence (from S25 commit, no invented numbers).**
`trunc` 32→26, `floor` 16→12 on -O2 -march=x86-64-v3 (GCC 16.2 19/15,
Clang 23.1 15/13, ICX 2025 18/14 on same flags in our oracle runs —
numbers are per our lab runs, not universal claims). `rint` holds 7
(blendv path). Corpus 51/51 byte-identical vs GCC for runtime, codegen
delta only in FP merges.

**Tests.** `ternary_float_merge.c` pins F64 const into F32 slot, int 0
into double, F32 const into F64 slot, and NaN payload preservation via
volatile/noinline runtime conditions (no constant-folded `1 ? x : y`).

## S26 (2026-09-24) — CVP correlated-select use rewriting + edge_facts Copy lookthrough

**Code:** `src/passes/cvp.rs` (`BOOL_BITS=64`, `edge_facts`, `decided_select_arm`,
select triple map, use rewriting before def folding).

**Rationale.** `S = select C,A,B` dominating `if (C)` is invisible to jump
threading (select dominates branch, no predecessor edge predicts it). When
the enclosing branch decides C, every use of S in that arm can be rewritten
to the selected arm (pure rename, no code motion: arm dominates select which
dominates use). Lets DCE + sinking delete selects duplicated across
correlated diamonds (libm `trunc`: inlined round-to-even select feeds both
arms of `if (x >= 0)`).

**Soundness.** BOOL_BITS is 64, so \"true\" means value in [1,2^64-1], not
exactly 1 — matches C bool. Chosen arm dominates select, select dominates
use, so no code moves. Phis (uses on edges) and `IsConstant` skipped.
`edge_facts` looks through `Copy` chains (GVN residue) so branch conditions
through copies are still decided.

**Ordering.** Runs in CVP, before GVN, after mem2reg. No new disable switch
(covered by `cvp`).

**Measured evidence (lab runs, -O2 -march=x86-64-v3).** `trunc` 32→17
(S26 final) vs GCC 16.2 19, ICC 21, Clang 15 15 in our oracle runs — gap
to Clang is 2 tail movsds, queued. 16/16 new `cvp` unit tests. No claim
of universal “beats all oracles”; numbers are per our Godbolt/mold
oracle runs on 14700KF target.

## S28 (2026-09-24) — union-punned copysign through memory → intrinsic

**Code:** `src/passes/bit_idioms.rs` (`recognize_copysign_mem`), `src/ir/intrinsics.rs`
(`CopysignF32/F64`), x86 backend lowering to `andps/orps` / `andpd/orpd`.

**Rationale.** musl `copysign`, glibc `s_copysign`, corpus `copysign` spell
`r.u64 = (a.u64 & ABS) | (b.u64 & SIGN)` via unions and memory. GCC/Clang/ICX
recognize this idiom and emit 2-3 bitwise ops (or single `andps/orps`);
lccc previously emitted 9 insns (stack traffic, reloads). Rewriting the
three-store + reload pattern to a `Copysign` intrinsic lets the backend emit
optimal code and enables further folding.

**Soundness (fail-closed).** Checks every use of both memory slots, requires
single-use chains, exact mask pairing (0x7fff_ffff / 0x8000_0000 for F32,
0x7fff_ffff_ffff_ffff / 0x8000_0000_0000_0000 for F64), swapped masks
rejected (some other computation), no calls/fences/unknown memory in window,
volatile init bails, unrelated allocas in window OK. Reuses DCE's own
side-effect check (hence `pub(crate)`).

**Ordering.** Runs in `bit_idioms` after SROA, before DCE. No new knob.

**Measured evidence (lab runs).** `copysign` 9→4 insns on -O2
-march=x86-64-v3, matches GCC 16.2 / Clang 23.1 / ICX output shape in our
oracle runs (andps/orps). All other TUs bit-identical (no over-fire). 8
new unit tests. No universal “ties all oracles” claim — per our measured
corpus on 14700KF.

**Tests.** `copysign_union_mem.c` covers ±0.0, NaN with payload, ±inf,
printing result bits.

## S29/S30 (2026-09-25) — TU dead-global-store elimination (Phase 11a) + evidence ordering

**Code:** `src/passes/dead_statics.rs` (`eliminate_dead_global_stores`),
`src/passes/mod.rs` Phase 11a/11 ordering, `src/passes/README.md`.

**Rationale.** After GVN's store-to-load forwarding, a `static` output
buffer that the TU never reads keeps only its stores (`out[i]=...` in
`round_family_pass`: same-iteration reload forwarded, `main` never touches
`out`). Stores are unobservable — no loads exist anywhere in TU — yet
backend emitted them plus address math. Every oracle deletes them
(whole-program DSE); this pass is lccc's TU-closed equivalent.

**Soundness (fail-closed list).**
- TU-local linkage only: static, not extern/common/weak/used/custom-section.
- No name-level escape: name appears in no toplevel-asm blob, no global
  initializer (init naming publishes address), no inline-asm input_symbols,
  no alias target/name (`__attribute__((alias))` publishes cross-TU), no
  inline-asm template string (textual mention like `movl g(%%rip), %0`).
- No value-level escape or read: each `GlobalAddr` naming global seeds
  address set propagated only through GEP bases, Copy, bitwise-identical
  Casts (single source of truth `cast_is_bitidentical_nop`), Phi
  (optimistic). Single-source chains so deleted-store sets for distinct
  globals disjoint. Every use of every set member must be non-volatile
  default-AS Store THROUGH address. Load through address = reader, address
  as stored VALUE, call arg, cmp/binop, memcpy endpoint, asm operand,
  terminator use → bail. Pure calls NOT exempted.
- All definitions derive: after fixpoint EVERY definition of EVERY set
  member must derive from global's address (GlobalAddr of this global,
  in-set GEP base with non-address offset, in-set Copy/Cast source, Phi
  whose EVERY arm in set). Makes optimistic phi sound (phi merging foreign
  value fails here, stores discarded), covers redefinition.
- At least one store recorded.

Deletion mirrors DCE sweep (spans compacted), DCE chaser on touched funcs
so orphaned value/address chains disappear in same pass (pipeline runs no
DCE after Phase 11).

**Ordering constraint.** Phase 11a (dead global stores) + Phase 11 (dead
static funcs) moved AFTER 11e (const-array promotion). Evidence-consuming
passes run before evidence-deleting ones: a dead `keep = a` escape store
is constarr's proof that address escapes; deleting it first would let
constarr promote an escaping array (the `const-array-promote escape-shape`
gate pins rejection). Deleting after constarr has refused keeps both.

**Performance (F6).** Pre-collects GlobalAddr defs once into
`FxHashMap<name, Vec<(func,value)>>` and groups by func, instead of
rescanning every function for every global (O(G*F*I) → O(F*I + G*refs)).
Avoids cloning use lists (iterates slice directly).

**Disable switch.** `CCC_DISABLE_PASSES=globaldse` (Phase 11a).

**Measured evidence (lab runs, -O2).** `round_family_pass` 56→50 (-6 = 2
stores + addr math + DCE) on x86-64, `out` BSS gone, `buf` path intact in
our corpus run. Corpus 51/51, 3431 lib tests, 68/68 ci_local --fast green
after ordering fix on our CI image.

**Tests.** 14 unit tests (incl. `phi_cycle_fires`, `hostile_phi_bails`,
`latch_cycle_fires`, `hostile_redefinition_bails`, `alias_target_bails`,
`alias_name_bails`, `inline_asm_template_bails`), plus C regressions:
`globaldse_unread_static.c` (unread buffer, runtime only),
`globaldse_alias_kept.c` (alias escape must keep stores),
`globaldse_asm_template.c` (inline-asm template mention must keep).

**Follow-up fixes (2026-09-25 audit, first Review AI).**
- F1 alias: built `alias_named` set from `module.aliases` (both alias and
  target) and skip globals in set; added unit test.
- F2 float ternary: `emit_ternary_merge_store` now converts F32/F64 consts
  via `coerce_to`; `narrowed_to` handles F32↔F64 cross.
- F5 inline-asm template: both dead-static passes now scan inline-asm
  templates with word-boundary matching (`asm_mentions_symbol`), not just
  input_symbols.
- F6 performance: GlobalAddr map + no-clone use iteration.
- F7 unroll-gate script: `jumps_in_fn` regex `j[a-z]+` covers all jcc
  variants (js/jns/jp/jnp/jo/jno/jb/jc etc.).
- F4 process: added this DECISIONS entry per transform; no invented numbers.

**Follow-up fixes (2026-09-25 audit, second Review AI — F1..F7).**
- F1 high $g matcher: `asm_mentions_symbol` now handles AT&T `$g`
  immediate prefix (`movabsq $g, %rax`, `movl $g+4`) while preserving
  embedded `$` safety (`foo$bar` must not match `bar` or `foo`). Impl:
  two-char lookbehind: `$` preceded by non-ident or start => immediate,
  preceded by ident => embedded. Added 6 unit tests:
  `asm_mentions_symbol_basic`, `immediate_prefix`, `immediate_bails`,
  `static_function_via_asm_template_survives` (transitive helper),
  `unrelated_static_still_removed`, `symbol_attrs_template_only_survives`.
- F2 float regressions no-merge: rewrote `ternary_float_merge.c` to avoid
  literal `1 ? x : y` (which folds before merge slots). All selections via
  `noinline` + volatile-driven runtime conditions, NaN payload preservation
  via `cond_nan_f32/f64`. Added Rust unit tests for `narrowed_to`/`coerce_to`
  covering F64→F32, F32→F64, rounding, subnormals, overflow→inf, signed zero,
  infinities, NaN payload same-width.
- F3 fail-open assert: replaced `.expect("pre-validated: ...")` in
  `bool_thread.rs`, `loop_idiom.rs`, `vectorize.rs` with fail-closed bail
  (continue / return false) so unexpected IR does not panic the compiler
  (`ccc: internal error`) but keeps the transform conservative.
- F4 tab jumps: `jumps_in_fn` now uses `[[:blank:]]` instead of `[ \t]`
  for portability; added self-test comment that both space and tab indent
  must be counted.
- F5 missing reachability/attr tests: covered by new dead_statics tests
  above (transitive reachability, unrelated removal, template-only attr).
- F6 docs overstated: toned down “beats all oracles” to lab-run numbers
  with explicit flags and target (14700KF), no invented universal claims.
- F7 alloc: confirmed no-clone iteration over `use_locs` slice, GlobalAddr
  map O(F*I+G*refs) not O(G*F*I), and no unnecessary `clone()` of templates
  beyond required owned string for map key.

## PERF-ARX-LAT (2026-09-26) — ARX lane frames chosen by critical-path latency

**Decision.** `vec_arx` and `arx_vectorize` choose which roles of the
lane-rotated ARX group are shuffled by minimising the round loop's critical
path under one shared, microarchitecture-neutral latency model
(`arx_vec_op_latency`: every emitted vector op 1 cycle, the shift-pair rotate
2, `vprold` 1).  `vec_arx` decides per op→op component, greedily in program
order (ties: fewer instructions, then the historical frame); `arx_vectorize`
ranks its four anchor offsets by `kernel_critical_path`.

**Why.** The previous frame shuffled the diagonal group's b role, whose value
comes from `rotl(b, 7)` and is consumed by `a += b` immediately: two `pshufd`
on the critical path per double round.  The loop is a serial recurrence, so
this cost 2 of 30 cycles on every model (llvm-mca znver4/5, raptorlake,
sapphirerapids), and the op-count cost model of the original design could
not see it.  Anchoring b is derived, not hard-coded: the same search keeps
any frame-insensitive kernel byte-identical.

**Evidence.** chacha20_block: recurrence 30 → 28 (v3), 34 → 32 (SSE2),
26 → 24 (AVX-512VL), both source forms; identical instruction count and
output; paired wall-clock after/before median 0.941 (18/21 faster, Xeon host);
lccc/gcc-v3 0.950.  Gate: `tests/regression/check_arx_frame_latency.sh`
(fails on the old compiler in all six configurations by exactly 2 cycles).

**Rejected.** Hard-coding "never shuffle b" (cipher-specific, and wrong for
schedules whose last op writes another role); a per-`-mtune` latency table
(the relevant latencies are identical on Zen 3–5 and Intel P-cores); counting
shuffles (the old design's error: all candidates have the same count).

## PERF-REASSOC-LAT (2026-09-26) — reassociate integer trees by recurrence latency

**Decision.** A new last-scalar-IR pass (`reassoc_latency.rs`,
`CCC_DISABLE_PASSES=reassoc_lat`) rebuilds single-use Add/And/Or/Xor trees
(i32/u32/i64/u64) that lie on a loop-carried recurrence.  Availability is
`(r, l)`: `r` = time along the innermost loop's recurrences (non-IV header
phis start at 0; invariants, IVs and IV-addressed loads are free), `l` =
local depth.  Free leaves are left-folded in source order; the partial sum
and the carried leaves are merged Huffman-style (two earliest first).  A
tree is rewritten only if its root's `r` strictly drops.  Leaves' pure
single-use chains are sunk to their new consumer.

**Why.** SHA-256 (benchmark ratio 1.30 vs GCC on EPYC) builds `t1` in source
order, putting four serial adds after Σ1 on the e→e recurrence.  Measured
round loop, llvm-mca 19 cycles/round: lccc 13.35 → **7.05** (znver4/5),
15.01 → **10.02** (raptorlake/SPR); GCC 16.2 9.69/12.01, Clang 23.1
12.03/14.02.  Recurrence bound (loop_latency.py --loads): 8 → **5** at v3
(the floor: Σ1 3 + 1 + 1), 8 → 6 at x86-64; GCC/Clang/ICX all 7.  Wall
clock (Xeon host, paired): sha256 0.930, sqlite_varint 0.959,
switch_dispatch 0.973, i686_alu_chains 0.983; 44 of 51 benchmark programs
byte-identical; outputs identical in all 7 that change.

**Why these rules (each from a measured failure).**
* *Recurrence-only:* the first version also balanced off-recurrence trees
  by local depth; conv_u8_3x3's 9-product sum then spilled (+24% dynamic
  instructions).  An off-recurrence tree is overlapped across iterations by
  the out-of-order core, so depth there buys nothing.
* *Sinking:* without it Σ1 and Ch stayed live across `K[i] + W[i] + h` and
  the round loop spilled one GPR.
* *`Not`→`And` costs 0:* BMI1 `andn` (default v3) makes Ch 2 deep; charging
  the `Not` tied Ch with Σ1 and gave 6 instead of 5.
* *Signed sums are legal:* IR integer Add wraps (no nsw flags) and every
  overflow-reasoning pass (IV widening, CVP, SCCP) runs before this one.

**Rejected.** GCC's rank-sorted linear chain (orders by def depth, not by
recurrence: its SHA round is 7, not 5); balancing every tree (register
pressure, above); a per-`-mtune` latency table (only relative order
matters, and the rule set is target-neutral except the `andn` fold).

**Known cost.** sqlite_varint executes +3.8% instructions (reg-reg copies
from coalescing around the reshaped trees; no spills) while running 4%
faster — a copy-coalescing follow-up, not a reason to narrow the pass.
