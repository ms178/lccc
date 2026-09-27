# Follow-up — PR #645 red-team audit, comparative adjudication, and the combined follow-up patch

**Session date:** 2026-09-27 (UTC)
**Base:** `e265e05` (merge PR #645: `1d3349b` reassoc rotation lags + register
residency + register-addend lea fold + CI gate mirror)
**Deliverable:** `/home/user/ms178-1.patch` (this session's delta on `e265e05`)
**Artifacts:** `/home/user/artifacts/godbolt-rot{,-combined}/`, snapshot ledger

---

## 1. Verdict on PR #645 (Agent B): accepted — and beaten on its own follow-up

The audit covered the full 1743-line diff: `reassoc_latency.rs` (rotation
lags, register residency, fixpoint rounds, load-leaf operand placement), the
register-addend extension of `fold_copy_add_into_lea`, the three retargeted
test pins, `check_reassoc_latency.sh` legs 5–6, and the CI-mirror machinery
(`ci_ubuntu_chroot.sh`, bidirectional `check_ci_gate_parity.py`, `--slow`
stamp composition, debug-assertions gates).

**Agreed (with evidence):**

1. **The rotation-lag model is the correct root-cause fix, and it is better
   than this session's previous size-only approach.** Measured head-to-head
   on the CI contract kernel (best-of-21 interleaved, bitwise-identical
   outputs): their 55-insn shape runs **3.98 ms** vs **4.7–4.9 ms** for the
   previous tree's 54-insn shape — the old fold set compensated the broken
   schedule's *size* symptom while leaving the 4-cycle recurrence in place.
   Latency beats size here; the size metric alone was misleading both
   directions.
2. **The register-residency guard is data-disciplined engineering** — the
   rejected discriminators (block peak, pressure integral, total span
   pressure) are each refuted by a measured case, and the surviving rule
   (recurrent values vs allocatable GPRs over the span) separates every
   measured one. Reproduced here: i686 rot is byte-identical to pass-off on
   their tree AND on the combined tree (`cmp` clean).
3. **The swap of a foldable load to the second operand is semantically
   safe:** `is_reassociable` admits only `Add/And/Or/Xor` over
   `I32/U32/I64/U64` (reassoc_latency.rs:198) — all commutative; `Sub` never
   forms trees.
4. **Refusing to weaken the "beats gcc" check was right**, and the
   Ubuntu-24.04 chroot mirror is the correct way to close the
   local-green/CI-red divergence that let PR #638 ship.

**Criticised (all fixed here, see §2):**

1. **The local gate contract broke for non-Ubuntu userlands:** on Debian
   (gcc 13.3 counts 55) their rot()=55 makes `check_phi_acyclic_order`
   FAIL (`55 vs 55`, strict `<`); only the chroot's gcc-13.2=56 passes. A
   contract that holds in exactly one userland is fragile; margin should
   cover the GCC range (13.2: 56, 13.3: 55, 14.2: 71).
2. **The +1 instruction was accepted without attempting their own
   FOLLOWUP** ("scaling lags by the loop's II would let the load join last
   at no model cost"). This patch takes the instruction back without
   touching the schedule — see §2.1 — landing at **53 insns at identical
   wall clock**, margin 2 vs gcc-13.3 and margin 3 vs the hosted runner's
   gcc-13.2.
3. **Spelling inconsistency:** their register-addend fold emits
   `leal (%r12,%rdi), %ebp` (no SIB space) while every other SIB-producing
   fold in the tree (incl. their own accepted SIB test pins) emits
   `(%rbx, %r14)`. Normalised to the spaced convention here, their three
   test pins adjusted.
4. **The register-addend fold's %rbp policy was a shape heuristic**
   (`dst==5 && src==4` refuses only the frame-setup copy). Replaced by the
   proof-based `rbp_is_gpr_in_function` gate (function-scoped: no
   `movq %rsp,%rbp`, no `(%rbp)` traffic, no rbp-rooted CFA, no opaque asm)
   — the same policy `retarget_producer_into_copy` already uses.

## 2. This session's delta on top of PR #645

### 2.1 `fuse_load_lea_into_base` (new) — the FOLLOWUP instruction, taken back

Their lag model deliberately merges the free `k[i]` leaf into the earliest
carried word (merging a free leaf *propagates earliness*; merging it last
adds +1 to an already-late partial — k-first is model-optimal, k-last is not
implementable without worsening the bound). The remaining +1 instruction was
the *staging*: isel emits `movl MEM,%X; leal (%B,%X),%D` for `(h + k)`, and
no existing pass folds it — x86 cannot fold a memory operand into a LEA
index, and the LEA→RMW direction needs the base to die plus a rename:

```text
    movl MEM, %X             (mov deleted)
    leal (%B, %X), %D   →    addl MEM, %B     (D's single use renamed to B)
```

`fuse_load_lea_into_base` implements exactly this (skip key `load_lea_base`,
phase 1 + late re-run), with every guard proven rather than assumed: X, B
provably dead at the LEA (`FileLiveness`, keep-set = the two deleted
instructions), D dead after the renamed use, D absent from MEM and from the
use's destination side, the delimiter-exact rename consuming every family
reference on the use line (a leftover `%edi` beside `%rdi` would read a
deleted definition), scale-1 only, MEM mentioning none of X/B/D, and
`flags_dead_after` (the LEA wrote no flags; the ADD does). On rot() the fold
fires on the t1 head and the pipeline then folds further: **55 → 53 insns,
wall clock unchanged (3.99 vs 3.98 ms best-of-21), outputs bitwise
identical, recurrence bound still 2.00** — `check_reassoc_latency.sh`
passes unchanged.

Eleven unit tests pin the fold and its refusal classes (live base, flags
read, result-as-destination, result-live-after-use, index-live-after-LEA,
base==index, scaled index, narrower-family spelling, wide form renaming
inside a memory operand, plus two positive fold pins). Two harness lessons
are encoded in the fixtures: hand-written fragments need the `# LCCC_RET_*`
prologue markers or `%rdx` is conservatively live at `ret` (real output
carries them; the oracle is correct, the fixtures were wrong), and
callee-saved fixtures must establish their frame or the same conservatism
refuses.

#### 2.1.1 The rename-width defect the mutation sweep caught (fixed here)

The first committed rename substituted only 64-bit names (`%rcx`), so the
delimiter-exact rename could never fire on a 32-bit-spelled use line —
`addl %ecx, %esi`, the dominant spelling in real `-O2` 32-bit streams —
and the pass silently disabled itself on exactly the SHA-rotation-shaped
code it was written for. rot() kept folding only because its use line
happened to spell `%rdi`. Found by attempting to mutation-validate the
negative fixtures: every guard mutation "survived" because the rename
guard refused before any targeted guard was reached. The fix tries the
folded width's spelling first, then the 64-bit spelling, and the residual
check refuses any other-width family spelling left behind (mixed `%ecx`/
`%rcx` lines still refuse). Byte-identical codegen on every audited shape
(rot() x86-64 and i686, sha256 142/9, the recurrence legs), so the fix is
pure latent coverage on this tree's measured kernels; quantifying its
yield across the 39-corpus is queued (§4).

#### 2.1.3 The imm32→disp32 canonicalization defect (found by the slow corpus, fixed)

`fuse_staged_add_and_relay`'s immediate branch carried the addend text
*verbatim* into the synthesized LEA's displacement. An ALU `imm32` is a
**bit pattern** — a 32-bit add computes mod 2^32, so `addl $2882400001`
(0xABCDEF01) is legal — while a LEA `disp32` is **sign-extended**. Splicing
the unsigned numeral into a displacement both refuses to encode
(> i32::MAX, the integrated assembler's loud error) and, were it accepted,
would compute `base − 1412567295` instead of `base + 2882400001`. The bug
was introduced with this pass (§2.1's commit) and surfaced only when the
slow differential corpus ran: `phi_latch_absorb` and
`widen_i32_i64_sse2_reduction` failed to compile; every audited kernel was
unaffected (all in-range imms fold verbatim — byte-identity discipline).

Fix: for 32-bit adds canonicalize the displacement into signed range
(`0xABCDEF01` folds as `-1412567295`, the same residue mod 2^32 — identical
32-bit result, encodable, GAS-valid, and the 3→1-instruction win is KEPT
rather than refused); values that are not even imm32 bit patterns refuse;
64-bit adds have no modular freedom (the disp is the real 64-bit addend) so
only signed-i32 constants fold there. The mirror site in
`fuse_copy_and_operation` (`addq $imm` → `leaq`) gained the missing signed
range guard. `fuse_load_lea_add` already enforced the full discipline
(i64 parse, checked_neg, i32::MIN..=i32::MAX). Five new unit tests pin the
canonicalization both directions, the verbatim in-range spelling, the
64-bit refusal, and the beyond-imm32-window refusal; the guards'
mutations are all discriminating (canonicalize→verbatim fails 2 tests,
refuse-window-off fails the window test, 64-bit-refuse-off fails its test).

Root-cause lesson recorded: the defect class was *repeatable* at
audit time — `fold_copy_add_into_lea` and `fold_inplace_add_copy_into_lea`
both already carried exactly the range guards this pass lacked. Any
immediate-to-displacement splice must parse, range-check, and canonicalize;
verbatim text splicing between operand classes is a bug.

#### 2.1.4 Post-merge review (PR #649 audit): H1–H3 adjudicated, fixed, proven

The post-merge review AI found three miscompile-class defects, all in
`fuse_load_lea_add`. Adjudication with empirical verification (the review
could not run code; every claim was re-derived and then tested here):

* **H1 (missing flags guard) — AGREED, PROVEN ON HARDWARE.** The rewrite
  keeps the ADD opcode but changes its operands ((B+C)+M vs (M+B)+C), so
  CF/OF/AF belong to a different final pair; only ZF/SF/PF are
  result-derived. The doc's "flag semantics BIT-IDENTICAL" lemma was false
  and — worse — `flags_consumer_still_folds` pinned the bug with a live
  `jl`. Both retracted. Native experiment (the exact triple, `jl` after
  the final ADD, M=-1 B=2 C=0x7fffffff): original association falls
  through, folded association takes the branch. Fix:
  `flags_dead_after(store, infos, k + 1)` (the ADD at k is itself a flags
  writer; the scan must start at k+1).
* **H2 (missing D==C refusal) — AGREED.** `lea (X,B),C; add C,C` computes
  2(M+B) but the rewritten chain computes B+C_old+M. Fix: refuse
  `c_fam == Some(d_fam)` (imm addends have no family — no change there).
* **H3 (dropped SIB displacement) — AGREED, FIXED BETTER THAN REFUSAL.**
  The `[a, b]` decomposition discarded `disp_prefix`, and the imm arm
  replaced the disp with the addend. The review prescribed refusal; this
  fix instead CARRIES the displacement: `disp(B,C)` is encodable (disp8/32
  + SIB), and the imm arm sums `checked_add(disp, imm)` with an explicit
  disp32 bound — so every disp shape keeps its 3→1 fold and only true
  encoding overflow refuses. In-range hex inputs now emit canonical
  decimal (closes review P3b for this pass's output).
* **P1a — AGREED, FIXED.** `rbp_is_gpr_in_function` now also honors
  `movl %esp,%ebp` / `movl %ebp,%esp`, `enter`, CFA register 5 (i386
  numbering) and `%ebp`-spelled `.cfi_def_cfa`, matching the three
  sibling passes' both-spellings convention. Test added (32-bit-spelled
  frame pins rbp).
* **P1b — AGREED, FIXED with an honesty note.** `fuse_load_lea_into_base`
  refuses `fam == 4` explicitly (sibling parity). The accompanying test
  pins the observable CONTRACT (an rsp-involving triple never folds);
  today the visible refusal comes from the liveness oracle — the guard
  exists so the contract survives oracle changes — and the test comment
  says exactly that (a mutation-killed guard test is impossible here
  because FileLiveness never reports rsp dead).
* **P2/P3/N — DEFERRED** (per the review's own instruction): per-candidate
  `FileLiveness` hoisting, immediate-parser unification, and the
  fold_staged/fold_copy_add overlap are backlog items, not silently
  dropped. Two review nits were wrong on inspection: `op_w` is used (it
  is `fuse_staged_add_and_relay`'s), and the claimed misnomered test does
  not exist.

Acceptance evidence: suite 3670 passed / 0 failed; the flipped
`flags_consumer_refuses_fusion`, `lea_result_as_add_source_refuses`,
`sib_disp_is_carried_onto_rewritten_lea`,
`sib_disp_imm_addend_sums_displacement`,
`sib_disp_imm_overflow_refuses`, and the P1a/P1b tests all green;
mutation matrix 5/5 discriminating (flags off / c==d off / disp-carry
dropped / sum→imm / overflow bound off each fail exactly their tests);
rot byte-identical (56 raw, 53 gate-count, driver output = gcc);
sha256_transform 142/9 (its t1 site is clean on all three axes, as the
review predicted); check_reassoc_latency 12/12 legs; check_phi_acyclic
contract holds; corpus pair differential = gcc.

#### 2.1.2 `flags_dead_after` argument: contract conformance, not a fix

An earlier draft of this document called the `flags_dead_after(store,
infos, j)` → `j + 1` change a miscompile-class off-by-one. That was wrong,
and the red-team pass here corrects it: `flags_dead_after` scans *from*
its argument (it examines `infos[from]` itself), and at call time line j
is still the flag-neutral LEA, so `j` and `j + 1` walk identical lines and
answer identically. The change lands anyway as contract conformance — the
function's documented contract is "the line after the flags-writing
instruction", and relying on the scanned line's own neutrality is a trap
the moment the rewrite order changes. No behavioral difference, no known
fired case, and the M1 mutation (guard disabled) is caught by
`flags_read_refuses` (the `stc` + `adcl` fixture).

### 2.2 The `%rsp`-as-index defects this audit caught in the *combined* tree

Wiring the previous session's `fuse_staged_add_and_relay` next to their new
fold exposed a latent defect in **both** of this session's passes: the
addend register is re-emitted as the LEA's SIB **index**, and `%rsp` is not
representable there (SIB index 100 encodes "no index") — GAS rejects
`leaq (%rbx,%rsp), %r8` outright. Their own `copy_plus_register_add_refusals`
test caught the shape the moment both passes were live. All three
load/staged-add folds now refuse `%rsp` in any emitted index slot (and
`fuse_load_lea_add` additionally refuses a `%rsp` B: as a base it would let
the rewritten ADD alias the stack pointer with a dying staged value). This
is a compile-failure class, not a miscompile class — the assembler refuses —
but it is a defect all the same, and the guard is cheaper than the failure.

### 2.3 Still-additive from the previous session (rebased, measured)

* **`flags_effect`: width-suffixed register push/pop is flag-neutral.**
  Still missing from main (PR #645 does not touch it). Effect, measured on
  this tree: sha256_transform default arm **144 → 142**, legacy
  175 → 173; rot legacy arm 72 → 71 (the blocked flag-discarding folds
  across callee-save restores now fire).
* **`fuse_load_lea_add` (disp32-hardened) + wiring** — still additive:
  it owns the `mov + lea disp-form + add` shape this session's new pass
  deliberately leaves alone; sha256_transform carries its two-instruction win.
* **`retarget_producer_into_copy` for %rbp under `rbp_is_gpr_in_function`**
  + the narrow-copy frame fixture fixes — unchanged by PR #645, rebased.
* **`th.s` removal** — the EVEX smoke-test leftover is still not referenced
  by anything; removed.
* **Test pins:** PR #645's pins for `frame_compact_tests` / `mod.rs` SIB /
  `vector_copy` are kept (better anchored than the previous session's:
  theirs pins `movb $0, ` and forbids a `(%rax)` survivor), adjusted to the
  spaced SIB convention; the previous session's competing versions of those
  three hunks are dropped in favour of theirs.

### 2.4 Oracle + measurement summary (this tree)

Static counts (`godbolt.py`, local = this tree, IDs resolved via `audit`;
manifests in `/home/user/artifacts/godbolt-rot-combined/`):

rot(), `-O2`: **LCCC 56** raw (53 gate-count) < GCC 16.2 60 < ICC 2021.10 79
< Clang 23.1 85 < ICX latest 136 — LCCC first on both metrics.
Gate-count: 53 < gcc-13.3 55 (Debian) and < gcc-13.2 56 (Ubuntu runner);
gcc-14.2 measures 71.

sha256_transform, `-O2`: **142** vs GCC 16.2 154 (Clang 126 leads; that gap
is PF-SCHEDULER-1, unchanged). chacha20_core, `-O2`: 118 vs GCC 180 /
Clang 176 (ICX 74 leads; PF-CHACHA-1, unchanged).

Wall clock (best-of-21, 400×4096-iteration rot, bitwise-identical outputs):
combined **3.990 ms** ≈ PR #645 3.981 ms ≪ pass-off 5.15 ms ≪ gcc-13.3
5.10 ms.

## 3. Validation

* `cargo test --profile fastbuild --lib` → **3663 passed, 0 failed**
  (PR #645's suite + 31 tests from the previous session + 11 new
  `load_lea_base_tests` + 5 imm32→disp32 canonicalization tests + 1
  `fuse_copy_and_operation` guard via the shared suite).
* **Mutation matrix, all eight discriminating** (each guard flipped off,
  exactly its fixture fails, then restored): flags guard →
  `flags_read_refuses`; X-liveness → `index_live_after_lea_refuses`;
  B-liveness → `live_base_refuses`; D-liveness →
  `result_live_after_use_refuses`; destination-side →
  `result_used_as_destination_refuses`; rename guard (whole) →
  `narrower_family_spelling_refuses`; dual-spelling fallback →
  `staged_load_folds_with_32bit_spelled_use`; alias (B==X/D==B/D==X) →
  `base_equals_index_refuses`. Method notes: mutations must be anchored
  *inside the target pass* (sibling passes contain textually identical
  guard lines — an unscoped `replace` once "mutated"
  `fuse_load_lea_add` instead), and the mutation script must check the
  edit's exit status or it silently tests the unmutated file.
* `check_reassoc_latency.sh`: all legs PASS incl. rot recurrence 2.00 (≤2)
  and the -m32 residency controls.
* `check_phi_acyclic_order.sh` (gcc-13.3 parity PATH): **contract holds** —
  rot 53/2, legacy 71/34, escape-off 53/3, gcc 55/0; differential
  correctness across 65 trip counts in both policies; sha256 known-answer
  both policies (142/9, 173/54).
* `rustfmt` clean; `-D warnings` build clean; `ci_local.sh --fast` + `--slow`
  on the final tree (the new mode=full stamp composition) — see the snapshot
  ledger for the recorded result.
* i686 rot byte-identical to PR #645's output (residency guard untouched);
  re-verified byte-identical again *after* the rename-width fix, x86-64
  and i686, together with both gate scripts re-run on the rebuilt binary
  with unchanged numbers (142/9, 516/510, recurrence 2.00).

## 4. Follow-up queue (deltas to PR #645's §6)

1. **sha256 remains the biggest honest static-count loss** (142 vs Clang
   126): the message-schedule vectorization epic (PF-SCHEDULER-1) is the
   lever; the v3 shift-form gap (~17 insns vs GCC 16.2's 137) is the quick
   half.
2. **chacha ICX gap** (118 vs 74/63): PF-CHACHA-1 numbers refreshed
   (backlog); the v3 ARX-lane schedule is the lever.
3. **Chroot the slow gates on release deliveries:** `ci_ubuntu_chroot.sh`
   exists; the snapshot gate now warns off-Ubuntu — the next step is a
   release checklist entry that runs the full mirror in the chroot so the
   "green locally, red hosted" class is structurally closed.
4. **Quantify the rename-width fix's yield:** the audited kernels are
   byte-identical, so its win is latent on this tree; survey the 39-corpus
   benchmark shapes for programs whose 32-bit-spelled use lines now fold
   (expect wins in integer streams that spell `addl %ecx, %esi`).
5. PR #645's remaining items (map64 unroll epic, Raptor-Lake hardware runs)
   stand unchanged.
