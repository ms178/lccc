# Decision & negative-results ledger

**Purpose.** Every load-bearing lesson from the point-in-time session journals
that used to live in `docs/history/` (121 files, 2026-03…2026-08),
`updates/` (10 files), `engineering/agent/SESSION_FOLLOWUP_KERNEL_BOOT.md`,
`engineering/agent/RA23_COMPLETION.md`,
`engineering/SESSION-20260825-torture-followup.md`,
`docs/linker/FOLLOWUP_2026-08-17{,_SESSION2}.md`, `hotspots/`, and
`docs/simd-audit.md` — all deleted; full text remains in git history. Nothing
here is a status snapshot. Each entry is a constraint, a measured negative, or
a root cause a future agent must not relearn the hard way. Entries cite the
source file they were distilled from.

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

- **[linker/FOLLOWUP_SESSION2 §0]** Stale **release** oracles produced two
  false failures (wild 0.7.0 blamed lccc; wild-git agreed lccc was right).
  Always build mold/wild from git before drawing conclusions. mold CMake
  pins: `-DMOLD_TARGETS='X86_64;I386'`, `-DMOLD_USE_MIMALLOC=OFF`,
  `-DMOLD_LTO=OFF`. Never compare Callgrind Ir across build profiles.
- **[linker/FOLLOWUP_SESSION2 §2]** Measured negatives: symtab **index
  sort** worse (57.8M vs 53.7M Ir); pre-sizing the FDE Vec with
  `count_eh_frame_fdes` 1.7 % slower; `push_strtab_name` as a tidy
  `extend(...)` iterator 5.8 % worse (defeats the vectorised copy — the
  reason that function carries `unsafe`); `Vec<u8> → Arc<[u8]>` **always
  copies** (removing the last whole-file copy needs mmap); the first
  zero-copy `SectionData` was 1.4M Ir *slower* because Deref re-derives
  bounds-checked subslices — bind `as_slice()` once. Removing a copy is not
  automatically a win.
- **[linker/FOLLOWUP_SESSION2 §2.2]** `parallel_reloc.rs` shipped real UB
  once: its doc claimed sorted-by-offset disjointness nothing established,
  and the bounds check was a `debug_assert!` (out-of-bounds **write** in
  release). Rewritten with high-water-mark partitions + checked writes; a
  randomised differential test caught an ordering bug **in the fix itself**
  (a stable sort only protects equal offsets).
- **[linker/FOLLOWUP_SESSION2 §2.3/§4.6]** Congruent segment packing
  (−59.5 % binaries): `p_offset ≡ p_vaddr (mod p_align)` is all the gABI
  requires. RELRO's boundary is an **address** boundary. `emit_shared.rs`
  had the identical defect (separate layout implementations drift
  silently). CORRECTION on record: the session-1 claim that
  `emit_script.rs` had the page-padding defect was **wrong** — an
  unverified claim in a follow-up doc is worse than no claim.
- **[linker/FOLLOWUP_SESSION2 §2.10+]** Linker correctness rules with named
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
- **[linker/FOLLOWUP_SESSION2 §13]** The mmap work shipped a
  `private_interfaces` warning for four sessions because the gate was
  `cargo build 2>&1 | grep -E "^error"` — blind to warnings by construction.
  Fix the gate, not the instance: `build_lccc_fast.sh` now exports
  `RUSTFLAGS=-D warnings` (`LCCC_ALLOW_WARNINGS=1` to opt out).
  Mutation-verify the gate.
- **[linker/FOLLOWUP_SESSION2 §14]** DSE deleted the stack-top write of an
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

- **[history/session18/24/64/84]** Environment protocol (harness wipes
  everything between sessions): swap first (`ensure_swap.sh`; some
  sandboxes cannot swapon — no `CAP_SYS_ADMIN`); Rust 1.98.0 pinned in
  `rust-toolchain.toml`, install under a persisted path; apt
  `gcc-multilib libc6-dev-i386` (m32 tests "cannot execute" ENOENT if
  absent — environment, not code), bison/flex/bc/cpio/libelf-dev/kmod;
  the kernel tree + 26 CachyMod patches is NOT persisted (re-extract each
  session); `/tmp` is wiped without warning (the LK-26 reproducers
  `/tmp/tlsinit.i` + `/tmp/offprobe.c` are gone — regen via the recorded
  `/tmp/tlspp.sh` recipe or reconstruct from `git show afa22485`).
- **[history/session84 / agent/SESSION_FOLLOWUP_KERNEL_BOOT]** Build recipe
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
  simply `scripts/build_lccc_o1_j2.sh` (fastbuild, `-O1`, `-j 2`,
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
- **[linker/FOLLOWUP_SESSION2 §6]** Testing rules that paid off: fuzz
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
  agent/SESSION_FOLLOWUP_KERNEL_BOOT.md + session 85): the
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
  `FOLLOWUP-2026-08-31-regehr-yarpgen-corpus.md`).
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
