# Follow-up: session 59 — worst-15 waves (range-fold, const-array), the constarr provenance audit, and the rebase onto PR #560

Date: 2026-09-19/20 (session 59, continuation)
Base: `db3f2adc` (= upstream main after PR #560 merged the arena-applied
re-measurement commit; the session started at `56858cbc` and rebased).
Series: `1835a839` range_fold · `a8139636` constarr · `dc470d74` constarr
provenance fix. Deliverable: `ms178-1.patch` (109,262 bytes, APPLIES-CLEAN
vs `db3f2adc`, verified by the snapshot pipeline).

## 1. What this session shipped

### Wave 1 — range_fold: the branch, And and Or forms (1835a839)

GCC folds `(x >= lo) && (x <= hi)` into the unsigned-bias test
`(unsigned)(x - lo) <= hi - lo` on the FIRST branch; lccc had the transform
only for one-constant-armed Selects and Phi diamonds. The wave added:

- **`fold_branch_chains`** — the BRANCH form: `Bcond`'s CondBranch chains
  into a single-compare `Bcheck` whose 'no' edge shares `Bcond`'s exit. The
  rewrite fuses the test in `Bcond`, deletes `Bcheck`, retargets `Bbody`'s
  phi arms to `Bcond` (rejecting a conflicting existing arm; dropping a
  duplicate with an identical value) and drops `Bexit`'s dead `Bcheck` arms.
- **`try_fold_bool_op`** — the BITWISE form if-conversion lowers small
  guarded bodies to: `And`/`Or` of two 0/1 compares of the same value fold
  to the range test (bitwise == logical on Cmp results), widening the
  boolean to the consumer's type when needed.
- **`resolve_bool_cmp` widening fix** — the boolean-widening transparency
  set stopped at `I16|I32|U32`, silently excluding the `U8 → I64` widening
  the frontend emits for `char`-typed predicates; the fold never fired on
  the very idiom it was written for (Expat's `xml_name_continue`).

Runtime: expat 1.453 → 1.328, strlen 1.061 → 1.022 (vs GCC, paired).
`csv_field_sum`'s digit test now emits GCC's exact shape
(`leaq -48,%r9; cmpb $9,%r9b; ja`).

### Wave 2 — constarr: constant local arrays → .rodata (a8139636 + dc470d74)

A local array initialized entirely by constants and only ever read is a
C-level constant, but the pipeline materialized it as an alloca plus one
store per element (vecreg_new_ops' three 16-byte arrays cost 48 scalar
`movb`s where GCC/Clang emit the bytes once in `.rodata`). The new pass
(Phase 11e, `CCC_DISABLE_PASSES=constarr`) audits the post-unroll shape
fail-closed and rewrites the survivor to an `is_const` global (`.LCA_n`)
with rip-relative references. Measured: vecreg_new_ops main 87 → 36
instructions (GCC 34), output bit-identical; no other corpus kernel
changes shape.

**The in-flight snapshot's phi extension and what it exposed.** The
session's snapshot S28 preserved an uncommitted extension: pointer-phi
derivation in the callee write-through proof (a loop strength-reduced to
pointer induction — the canonical `-O2` reader shape — merges the
preheader pointer with the backedge GEP; without the phi arm every such
callee rejected). The red-team audit of that extension is §2.

## 2. The red-team audit: two shipped holes, found and closed (dc470d74)

Auditing my own in-flight suggestion instead of trusting it reproduced the
gate failure the snapshot was saved mid-debug on, then traced it past the
immediate symptom to two latent defects in the SHIPPED wave-2 commit
(reproduced on the pristine `38ae2c4a` worktree):

1. **Miscompile (P0): variable-offset stores were invisible.** The store
   classifier resolved pointers only through const-offset GEP chains
   (`gep_roots`), so `a[k & 3] = v` — a runtime store — matched no
   candidate, and its GEP.base mention was whitelisted. An array whose
   other uses were all provably read-only promoted with the runtime write
   intact: `movl %esi, .LCA_0(%rip,%r11,4)` — a store into `.rodata`
   (SIGSEGV; silent corruption in any build that links the section
   writable). The gate's keep-shape only passed because `sum`'s pointer
   phi made the callee proof fail — coincidence, not protection: with a
   phi-free reader (`return p[0] + p[1];`) the pristine tree promoted and
   segfaulted. The same invisibility held for `Memcpy` dest,
   `AtomicStore`/`AtomicRmw`/`AtomicCmpxchg` pointers, `InlineAsm`
   outputs, and `VaCopy` dests — the catch-all's root walk could not see
   through variable offsets, casts, copies, phis or selects.
2. **ICE (P0): terminators were never audited.** `return a;` promoted,
   `apply_one` deleted the alloca, and the backend refused to fabricate
   the dangling reference: `operand_to_rax: value 0 in function 'leak'
   cannot be materialised`.

The phi extension itself was sound (a phi merging a derived pointer with
anything else *may* carry the object on some path — the rejection
direction for writes, exactly like the caller-side rule), but it was
load-bearing in the worst way: by making loop-reader callees provable
read-only it unmasked hole 1.

**Fix: classification by provenance, not spelling.** `pointer_touches()`
computes by fixed point the set of candidate allocas each value may point
into, through ANY chain of GEPs (const or variable offset), casts, copies,
phis and selects. The Store/Load/GEP/Call arms, the catch-all, and a NEW
terminator scan all resolve through it. A store touching a candidate with
a non-`Const` value or an unresolvable offset rejects; every
variable-offset write position rejects; any terminator mention rejects.
The positive unlock survives and is now exercised by its own battery
shape (`p8`: const array + pointer-induction reader promotes — verified
the pre-fix compiler rejects it).

Alignment was also threaded end-to-end in the same commit: the promoted
global keeps the alloca's alignment (`int[2]` at `.align 4`, not 1;
`_Alignas(64)` at `.align 64`), with `effective_align`'s size ≥ 16 → 16
promotion still applying (GCC/Clang parity; the committed version's
hard-coded `align: 1` would have placed sub-16-byte typed arrays at
align 1).

The gate (`check_const_array_promote.sh`) grew the hole/leak/align/
promote-i32 emission+execution contracts, and — like the ISA gate before
it — was wired into `ci.yml` + `ci_local.sh` only by this audit (it was
in NEITHER).

## 3. Verification (final tree, at dc470d74 on db3f2adc)

- `check_const_array_promote.sh` + `check_range_fold_branch.sh`: PASS.
- `ci_local.sh --fast`: **46 passed, 0 failed, 3 skipped** (the new
  const-array-promote gate included).
- Full regression suite: 825 passed, 3 skipped, 25 failed — the failure
  list IDENTICAL to the pristine-base A/B (all 25 pre-existing
  environmental: i686 multilib headers, aarch64 cross-gcc, and the
  env-dependent asm checks).
- cargo test: 2918 passed, 0 failed, 6 ignored.
- Benchmark output oracle: 204/204 PASS.
- `cargo fmt --check` green; clippy `-D warnings` green (lib/tests/bins,
  per-target for the 4 GB box).
- Build policy honored throughout: fastbuild, foreground.

## 4. Follow-ups (priority order)

1. **TASK-RA-06A (existing, P0): the shared-latch slot detour.** Fresh
   evidence from this session's csv reading — `sum_fields`' loop-carried
   `col`/`cur` round-trip through `16/24(%rsp)` at the shared latch
   `.LBB12` while `total`/`s2`/`in` move register-to-register. The tell:
   the digit path stores `%r11` and the latch reloads `%r11` — a pure
   slot round-trip of an UNCHANGED register. The pattern fires when a
   phi's incoming homes differ across predecessors sharing a merge block
   (r11 vs rdx): the canonical merge location becomes the spill slot
   instead of a register-to-register parallel copy, costing 2 stores +
   2 loads per iteration where 2 movs (or nothing, coalesced) would do.
   Entry points: phi elimination + `copy_prop` post-phi
   (`src/passes/mod.rs` wiring), `web_congruence.rs` (the phi-latch
   copy-web cleanup, rule D), and the backend home assignment.
   linux_rbtree's reload-per-use (sp=63) is the same family, worse.
2. **csv phi-copy shuffling** (the range-fold wave's remaining gap):
   beyond the slot detour, the latch runs a chain of `movq` phi shuffles;
   coalescing the diamond's copies into the fused branch would remove
   them. Same files as follow-up 1.
3. **constarr: aggregate-init spellings.** `int a[4] = {1,2,3,4};` lowers
   through a `Memcpy` from an anonymous `.rodata` array (the tiling never
   sees per-element stores). Either teach the audit the memcpy-from-const
   form (resolve the source global's bytes as the image) or emit the
   array directly. This unlocks the dispatch-table spelling
   (i686_alu_chains' `const struct kernel ks[]` additionally needs
   `GlobalAddr`-typed initializers in the image — `GlobalInit` already
   supports them for file-scope statics).
4. **constarr: caller-side phi/select of the array address** currently
   rejects (the catch-all treats the dataflow mention as an escape). The
   callee-side rule (writes reject, reads allowed) applies verbatim to
   the caller; extending the whitelist + `apply_one` substitution to
   Phi/Select arms would promote inlined reader loops. Conservative
   today, sound.
5. **range_fold: load-CSE before the fold.** Two source-level loads of
   `*p` are not CSE'd before range_fold runs, so only the
   local-variable spelling folds (documented in the gate). A load-CSE
   step ahead of the pass (or extending the cast-chain identity to
   same-address loads) unlocks the `*p`-twice spellings.
6. **The remaining worst-15** (unchanged from the s59 re-measure):
   sha256's schedule loop, chacha20's state frame, struct_copy's SROA,
   matmul's k-unroll — see `s59-rank/` for the per-kernel oracle asm.

## 5. Environmental notes for the next session

- The harness wiped nothing this round; the session ran continuous from
  S28 (the user-ordered in-flight snapshot) through S30.
- PR #560 = the arena bot applying the S25 re-measure deliverable; the
  rebase onto `db3f2adc` cherry-picked the two code waves and squashed
  the WIP snapshot commit into the fix commit (3-commit series).
- cargo test still needs `CARGO_PROFILE_FASTBUILD_DEBUG=0
  CARGO_INCREMENTAL=0 -j1` on this box when the page cache is cold.
- The 25 suite failures remain the pre-existing environmental set
  (i686 multilib headers, aarch64 cross-gcc, env-dependent asm checks).
