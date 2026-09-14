# Red-team audit — S25-S29 (post-rebase session on main 5ef3fe568)

Date: 2026-09-14. Auditor: the author, adversarially. Scope: the five
engineering changes of this session (commits c45f112d2..77265d4a2) and the
process incidents. Upstream content since the S21/S22 audit record
(`s21-s22-redteam-audit-2026-09-13.md`) is exactly PR #526 — the squash merge
of my own S21-S23 chain-layout work — so there is no third-party engineering
content to re-audit; this record covers my own new deltas against that base.

Deliverable state at audit time: 34/34 CI gates green (full battery:
cargo-test 2772/0, regression corpus 716/0 with the inter-pass IR verifier
armed, benchmark-output oracle 204/204, differential correctness, clippy,
rustfmt, all x86/i686/cross/linker gates). CE rank 2967 → 2950.

---

## 1. W2 load→cast sext fold for I32→I64/U64 (c45f112d2)

**What it does:** extends the W2 fold (load redirected straight into the
consuming widening cast's register, cast skipped) from the zero-extension
types (U8/U16/U32) to the sign-extension pair I32→{I64,U64}, and wires the
same fold into `emit_load_with_const_offset_impl`'s three register-base
paths, which the original W2 hook (register-direct loads only) never
covered — the folded-GEP load route.

**Attack surface and verdicts:**

* *Could the fold skip a cast whose semantics are zero-extension?* The
  width_ok table adds only (I32,I64) and (I32,U64) — both are
  sign-extending in C (int→long / int→unsigned long). (I32,U32) and
  (I32,I32) are deliberately excluded: an I32→U32 relabel under the movl
  (zero-extension) convention would change the 64-bit half form.
  **Sound.**
* *Could the load emit `movl` (zero-extend) while the cast is skipped?*
  Belt-and-suspenders `i32_sext_ready` guard requires the load's dest in
  `needs_sext_values` at analysis time; the widening cast marks its source
  in that set (pass 1, `from_ty ∈ {I32,U32} ∧ is_64(to_ty)`), so the
  single-use adjacency guarantees membership. If the set is somehow empty
  the guard refuses (fail-closed). **Sound.**
* *Cast not adjacent / multiple uses?* Analysis requires `single_use &&
  adjacent_cast` (insts[ii+1]), matching the U8/U16/U32 contract. **Sound.**
* *Register-free-at-load guards:* unchanged (segment-aware + fat-envelope
  fallback), applied to the new pairs identically. **Sound.**
* *fold_skip_cast handshake in the const-offset paths:* set only on the
  paths that actually redirected the load into the cast's register; any
  non-fold path leaves it None and the cast emits normally (the old,
  correct, merely-unfused behavior). **Sound.**
* *Residual:* the `note_inplace_compute(fr, fold_dest)` accounting matches
  `emit_load_impl`'s W2 pattern; pinned/mark-nop machinery is not involved
  here (emission-time, not peephole). No known hole.

**Verdict: agree with the design.** The asymmetry I initially considered
(treating I32 like U32) would have been a miscompile; the belt guard makes
the class impossible.

## 2. Window-adjacent const-offset GEP folds (c45f112d2)

**What it does:** `adjacent_addr_producers` required the single-use address
producer to be the *immediately* preceding instruction of its Load/Store.
Now a same-block window of ≤8 pure instructions may sit between them, none
of which may write memory, be a Phi/Call/atomic/fence/stack-op, or redefine
the producer's operands.

**Attack surface and verdicts:**

* *Why was strict adjacency there?* Reading the history: the function's doc
  says "only the producer dest is proven stable — NOT aliases introduced by
  later copy-propagation". The stability question is about the BASE's
  availability at the access; the RA base-link
  (`collect_gep_fold_base_links`) extends the base's live interval to the
  access wherever they sit, so adjacency was a conservative proxy, not a
  soundness requirement. The phi-lowered loop bases (2 defs from the
  pre-phi copy lowering) already fold via this arm — with the *same*
  multi-def base the window now also accepts. The load/store-side guards
  (`can_const_addr_fold` → `const_offset_fold_reg_base_ok` at emission,
  `retain_ptr_only_uses` for unfoldable/volatile/segment accesses, the
  base-in-map safety net) all apply unchanged to window-marked candidates:
  the map entry shape is identical. **Sound.**
* *Redefinition guard:* intervening defs of the producer's dest (impossible
  under single-use) and of its base/offset operands are both refused
  (`addr_producer_guard_operands` + `d.0 == dest.0`). Under SSA the operand
  values cannot change identity; the guard covers the non-strict-SSA
  accumulator-forwarding shapes. **Sound.**
* *Window bound:* 8 — covers the front-end's eager-address emission
  (RHS chains of 1-3 instructions) with margin; bounded for compile time.
  A window that is too large only costs scan time, never soundness (the
  guards are per-instruction).
* *Honesty note:* the rbtree main-loop case that motivated the S19 program
  does NOT benefit yet — its cursor base is *slot*-homed, so
  `const_offset_fold_reg_base_ok` refuses at emission regardless of the
  window. The window pays on register-homed bases (fill/loop shapes); the
  slot-homing question is Fix D's RA spill-cost work, not address-mode
  work. Recorded as follow-up, not silently claimed as fixed.

**Verdict: agree**, with the honesty note above made explicit.

## 3. Def-side notes on const-offset register-direct loads (c45f112d2)

The three register-direct arms of `emit_load_with_const_offset_impl` wrote
the dest's home register without `note_inplace_compute` — the documented
PR #487 defect class (loads are the most common def of a homed value; a
missing note lets a later stale-home read fire). The fix adds the note,
which also calls `note_reg_clobbered` — *more* accurate tracking, never
less. Risk assessed: a later pass relying on a stale "fresh" flag was
relying on wrong data; the full battery (incl. differential) is green.
**Agree.**

## 4. store_forwarding: immediate-backed mappings + pair fusion (3a90fb418)

**What it does:** three mechanisms (see the commit message). The pair
fusion deletes a load whose next instruction stores the loaded register to
another slot, rewriting the store to read the mapping's source register (or
the forwarded immediate).

**Attack surface and verdicts:**

* *The load's destination register may be read later.* This was a REAL
  miscompile on first attempt: diamond_soup -O0 (the `-O0` alloca
  ping-pong: `movq -8(%rbp),%rax; movq %rax,-32(%rbp); movq %rax,-8(%rbp)`)
  — the third instruction's read of %rax observed the pre-load value. The
  store-alu-cross-join differential gate caught it (exactly the gate's
  purpose). Fixed by the `FileLiveness::live_after(j, load_reg) ==
  Some(false)` gate — the same exact-dataflow contract copy_coalesce uses —
  plus `refresh_at` after every fusion. **The fix is the audit's most
  important outcome: the initial version was wrong, the gate caught it, the
  final version is provably restricted to dead-destination pairs.** Sound.
* *Immediate encodability:* `imm_fits_store` mirrors the emitter's
  `direct_store_imm` byte-for-byte contract: Q takes only the i32 range
  (movq imm32 *sign-extends* — allowing u32 would write 0xFFFFFFFFFFFFFFFF
  for $4294967295), L takes i32..=u32 (raw imm32 field), W/B their own
  ranges. Verified against the emitter source, not from memory. **Sound.**
* *`parse_imm_slot_store` shape check:* two operands exactly, mnemonic from
  the closed set {movq,movl,movw,movb}, destination text equal to the
  pre-parsed rbp_offset + `(%rbp)`/`(%rsp)` — no SIB, no symbol, no
  rip-relative. `rbp_is_frame` guard for rbp-based lines (rbp-as-data
  derefs never recorded). Multiple-different-offset lines are refused by
  `parse_rbp_offset` returning RBP_OFFSET_NONE. **Sound.**
* *`load_still_writes_reg` skip:* a nop'd load writes nothing; running the
  dest-register invalidation anyway kills mappings for a register whose
  value did not change. The same-register elision (load writes the value it
  already holds) is idempotent — deletion cannot change any register. The
  pair fusions' deletions are the ones that needed the liveness gate.
  **Sound.**
* *Pinned lines:* `mark_nop` refuses to delete a pinned load while the
  store was already rewritten — the result is correct (the store reads the
  right value; the load executes harmlessly), merely not minimal.
  Benign-degradation, not a miscompile. **Sound.**
* *Line-j/spanning:* `next_non_nop` between load and store accepts only a
  StoreRbp — a Label/CondJmp/Cmp/anything else refuses. Conservative.
  **Sound.**
* *REG_NONE (255) indexing:* `deactivate_entry` guards the reg_offsets
  index for immediate-backed entries (255 would panic on the 16-entry
  array). **Sound.**

**Verdict: agree with the final version** — and the record of the caught
miscompile is the point: the process worked.

## 5. needs_sext-gated self-extension on immediate-mul/add (77265d4a2)

The LEA/imull strength-reduction and Add/Sub l-form paths emitted an
unconditional `cltq`; the general ALU path already had the
`emit_sext32_for_value` refinement (skip when the value has no 64-bit
consumer). The four sites now share that contract. The risk would be a
64-bit consumer reading the zero-extended slot value — impossible by the
needs_sext analysis's definition (every 64-bit consumer position marks its
operand, transitively through Copy/Phi). The knock-on (dead cltq had
blocked the peephole's LEA→mem fold) was measured: rank −12 across 10
functions. **Agree.**

## 6. Process incidents (recorded for the next session)

* **The false −32% dynamic "win" on memcmp.** First icount A/B showed
  87.7M→59.5M retired instructions on `glibc_memcmp_common_alignment`.
  Cross-checking the *static* code proved the function byte-identical
  between arms — impossible for a dynamic delta — which localized the
  error to the measurement: the qemu exec log (~1.9 GB per rep) had
  exhausted the disk mid-write and the truncated log parsed as a smaller
  count. The tool's determinism gate had flagged exactly this ("NON-
  DETERMINISTIC — measurement rejected") and the flag was initially
  misattributed to disk noise around the gate rather than the gate being
  *right*. Lesson recorded: **when a dynamic number moves implausibly,
  prove the static delta first; when a determinism gate fires, believe it
  before believing the measurement.** Honest re-measurement (per-invocation
  temp dirs, cleanup between, static-identity cross-check) shows the
  session's changes are dynamically neutral on the benchmark suite (all
  affected paths are cold or once-executed).
* **Disk discipline:** the baseline-binary worktree build (~3 GB) plus qemu
  logs (~2 GB per run) on a 9.9 GB disk caused the above. Future A/Bs:
  build the baseline once and *keep* it under /tmp with a marker, run
  icount one compiler per invocation with `rm -rf $TMPDIR/*` between.
* **The incremental-cache/OOM interaction:** deleting
  `target/fastbuild/incremental` for disk space raised the cold test-build
  memory peak beyond the 4 GB limit (SIGKILL). The ci_local cargo-test gate
  compiles with its own flags (different metadata hash → no reuse), and
  needed the dev server's 680 MB freed to pass. The dev server was stopped
  for the battery run and restarted after (preview verified 200 OK).
  Future: do not delete the incremental cache; trim rank artifacts instead
  (they were 2.3 MB per survey and are the actual disk hog).

## 7. Open follow-ups (priority order, unchanged plus refinements)

1. **Fix D** (rbtree +443 / csv +386): the structural program —
   select-arm branch specialization (if-conversion reversal) for
   Select-of-GEP in pointer-chasing loops, RA slot-homing of loop-carried
   cursors (the %rdx/PhysReg-16 base exclusion interacts: the RA keeps
   homing cursors in the fold's excluded scratch registers — an RA-side
   denial like i686's `collect_i686_scratch_denials` is the x86-64 shape),
   per-field GEP chains in the insert path.
2. SLP/vectorization epics: struct_copy +80 (clang 76-insn full SLP),
   chacha20_core +105, adler32 DO8 +168, nbody +212.
3. Kernel boot quest (RO-write ffffffff82001770, schedule_tail+0xec WARN,
   workqueue leaked-atomic); QEMU kernel harness rebuild — host-tools
   script with `--qemu` + the deb cache in my-project.
4. PGO fingerprint should include chain-layout state.
5. The ir-verify backlog from S20 is confirmed **cleared** (corpus 716/0
   with the verifier armed; the three named tests now pass — fixed
   somewhere in the upstream GLA hardening / S38 wave).

## 8. Summary verdict

All five changes are agreed with after adversarial review; one (the pair
fusion) shipped only after a differential gate caught a real miscompile
and the fix was proven with exact dataflow. The measurement process had
one false positive that was caught and documented rather than shipped as a
fake win. CI is green end-to-end on the full local battery. The gap
reduction this session (2967 → 2950, −17, with 22 functions ahead of
best-of-oracle) is honest, measured, and conservative.
