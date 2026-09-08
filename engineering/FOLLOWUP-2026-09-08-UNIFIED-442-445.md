# FOLLOWUP 2026-09-08 — unified series: PR #442 + PR #445 (ported) + red-team + ChaCha20 diagnosis

Base: upstream `main` `b6d6e7bd` (merge of #444). Deliverable
`ms178-1.patch` = `git diff b6d6e7bd..unified`: 442 in full, 445
ported per-area (adopted vs dropped below), three 442 root-cause
fixes, `__attribute__((cold))` wiring, loop-alignment refinements,
identical-blocks fixes + tests, abs-fold miscompile fix + tests,
`CCC_NO_SPAN_VALVE` RA diagnostic, bench.yml → fastbuild. All gates
green (§7).

## 1. PR #445 red-team verdict (per area)

Method: full diff review of 445 tip `1aba0bd8` vs its base, plus a
local rebuild + full gate battery of the 445 tree (§3). Verdict
summary — adopted insights, dropped risks:

ADOPTED (ported onto `unified`):
- `vectorize.rs` conditional-map work (integer lane compares, lane-mask
  selects, min/max folds, FP selects, remainder mirrors) + its
  `run_correctness.py` differential tests — real vectorizer coverage.
  One confirmed miscompile fixed during the port (§4).
- `regalloc.rs` delta (445's allocator changes outside 442's +51
  `loop_span_reserve` lines, which are disjoint and retained).
- Intrinsics (IR + x86 + ARM), `lane_const`, `stack_layout` ×3 —
  reviewed, coherent, adopted.
- `loop_align`: trip-bound ≤4 skip + unlikely-section skip — adopted
  as two surgical insertions into main's pass (445's whole-file
  rewrite dropped, see below). Made fully live via §5 cold wiring.
- `identical_blocks`: the *problems* 445 addressed are real, but its
  rewrite is not taken; instead four surgical fixes P1–P4 (§5) +
  three unit tests, two red-validated.

DROPPED (audited, rejected with reason):
- ELF `AlignChain` writer changes — highest-risk, unneeded: main's
  GAS `.p2align` emission is already correct.
- Parser changes — 445's parser delta is churn with no test-demanded
  behavior; unified takes only the `cold` attribute it needs (§5).
- Backend `loop_align` rewrite + latch-check deletion — main's
  `passes/loop_align.rs` (natural loops, PGO gate, `-falign` flags)
  is strictly more capable; deleting the latch check removes a
  soundness guard.
- Optimization deletions — 445 removes passes/patterns main relies
  on; the corpus + bench gate pin current behavior, and no
  measurement justified the removals.
- `loop_unroll` test deletions — 445 DELETES
  `escaping_latch_def_blocks_partial_unroll` (and kin) instead of
  fixing the underlying failure. Unified fixes the root cause (§4)
  and keeps the test. Deleting a failing test is never a fix.

Net: #445 is not better as a whole — it is a mix of valuable
vectorizer/allocator work and high-risk rewrites/deletions. Every
valid insight was adopted; every unjustified deletion/rewrite was
left out. The unified tree is the constructive merge 445 should
have been.

## 2. PR #445 CI forensics (Test Suite failure NOT reproducible)

445 CI at `1aba0bd8`: Clippy job failed, Test Suite failed,
LCCC-vs-GCC succeeded. Annotations: missing regression JSON/log/env
artifacts + exit 101. Job logs need auth (API 403), so the failure
was re-run locally instead: a clean worktree at the 445 tip,
`build_lccc_fast.sh`, then EVERY Test Suite step verbatim —
cargo test (7 suites ok), regression corpus 684/700 pass 0 fail
(14 skipped_compare, 2 skipped_run), check-engines 13/13,
synthetic fuzz 4/4, inline-asm utf8, recip + machinst oracles,
linker fuzz ×2 — ALL GREEN. The 445 CI failure is
environment-specific (prime suspects: newer-stable lints under
`-D warnings` since rust-toolchain tracks stable, or
multilib/CPU-dependent exec tests — the local VM has AVX512 and no
multilib, so i686 exec tests skip here). Mitigation in unified:
strict clippy + fmt are verified locally (§7), and 445's
highest-risk parts were not ported — but the residual risk is
reported honestly, not hand-waved.

## 3. PR #442 root-cause fixes (it did not compile)

The committed 442 snapshot did not compile — its final assembly
lost edits to a read-modify-write race and was committed without a
rebuild. Three root causes fixed, all validated:

1. `vectorize.rs` remainder-planning `depth`/`done` threading:
   `plan_remainder_reference_inner` called
   `plan_invariant_chain(..., depth, done)` (7 args) but the callee
   still took 5 — 14 compile errors. Fix: signatures threaded AND
   `plan_operand_chain` now recurses into the INNER planner with
   `depth + 1` (the as-committed outer call would have reset the
   depth cap and the diamond memo per operand, silently disabling
   the cycle guard and the memoization the recursion exists for).
2. `live_range.rs:823` clippy `redundant_field_names` (`total_spans:
   total_spans`) — fails the `-D warnings` clippy gate.
3. `loop_unroll` test `escaping_latch_def_blocks_partial_unroll`:
   442's new verifier check 8 (def-dominates-use) correctly flags
   the test's hand-built fixture (a while-shape latch def used
   directly in the exit is inherently non-dominated — malformed SSA
   the real pipeline never produces, verified with `CCC_VERIFY_IR=1`
   on loop-heavy C: zero violations). Fix, not dumbing-down: the
   fixture now spells the escape in well-formed LCSSA form (latch
   def → header phi on the back edge → exit returns the phi) and
   additionally asserts `latch_value_escapes` directly, so the
   rejection assertion cannot pass vacuously. (A do-while reshape
   was considered and rejected: gate 2 requires an unconditional
   latch→header branch, so it would pass vacuously.)

Also: `cargo fmt` applied tree-wide (all flagged files are inside
the port set; the rest of main is clean under rustfmt 1.98.1).

## 4. Abs-fold miscompile (found in 445's vectorizer, fixed)

445's `fold_int_minmax` folded `l < 0 ? Sub(0, _) : l` to
`max(l, -l)` matching the true arm by its `Sub(0, ...)` LHS alone —
`v < 0 ? 0 - (v + 1) : v` misfolded to `abs(v)` (silent wrong
answer: v=-5 gives 5, correct is 4). Fix: `is_neg_of(e, target)`
checks the subtrahend is the compared value itself. Locked by new
test `vectorize_abs_fold_subtrahend` (slt/sle positive incl.
forced INT_MIN, nested, three anti-miscompile shapes, n=0..33
remainder sweeps) — RED-VALIDATED (fails on the reverted fold with
MISMATCH, passes with the fix). Full correctness suite: 53/53.

## 5. New work in unified

- `__attribute__((cold))` wiring (parser flag → `FunctionAttributes`
  / `DeclAttributes` → lowering defaults `cold` + no explicit
  section to `.text.unlikely`, GCC-compatible).
  `FunctionAttributes` flags widened u16→u32 (bit 16; contained to
  `ast.rs`). Validated: cold fn lands in `.text.unlikely`, `.text`
  restored after, link+run byte-identical to GCC, zero loop
  `.p2align` in the cold function while hot siblings keep theirs.
- `loop_align`: constant-trip-bound ≤4 skip (Auto only — explicit
  `-falign-loops=N` honored unconditionally) + unlikely-section
  skip. Asm-validated at -O1 (alignment on, unroll off): trip-4
  bare, trip-5 cascaded, explicit flag aligns both.
- `identical_blocks` P1–P4: hash + text identity skip
  alignment-directive padding (blocks differing only in padding
  merge; deleted copy's directives stay verbatim); terminator scan
  + cleanliness skip `.loc`/`.file` (a trailing `.loc` no longer
  fakes a fall-through edge); debug markers stay hashed (no
  cross-line merges under `-g`). Three unit tests, two
  red-validated against reverted predicates.
- `loop_unroll.rs` + `verify/tests.rs`: fmt-only + the §3 fixture fix.
- `CCC_NO_SPAN_VALVE` RA diagnostic switch (+ config test). Default
  OFF (see §6: the valve is load-bearing; the switch is for A/B and
  the documented splitting project, never for default).
- `bench.yml`: release → fastbuild (build script + all three binary
  paths). Rationale documented in-workflow: bench measures emitted
  code, and lccc is deterministic, so fastbuild emits identical
  assembly while thin-LTO release only burns CI minutes. `ci.yml`
  already used fastbuild — now consistent.

## 6. ChaCha20: diagnosis, measurements, roadmap (stays 1.60×)

Official bench (`-O2` both, 9 reps, pinned): lccc 442.0ms vs gcc
276.5ms = **1.598×** (tight CI [1.593, 1.620]). The gap was
bisected to the backend with unusual precision; the fix is a
multi-session RA program, NOT attempted here. Record:

- Final IR is near-optimal: x[16] fully SROA-split + mem2reg'd to
  16 loop-carried phis, round body is pure Add/Xor/RotateLeft with
  ZERO memory ops, 32 rolls at parity with GCC. Pipeline diagnosis
  (per-pass dumps) shows SROA+mem2reg+DCE collapsing L=123→33 as
  designed. Nothing is wrong above codegen.
- The gap is 100% register allocation: lccc's rolled round loop
  carries all 16 state words in stack slots (~100 memrefs/iter);
  GCC's equally-rolled loop has 7 (register-resident, all 16 GPRs).
- Mechanism (via `CCC_RA_EXPLAIN` + phase dumps): chacha20_core is
  a leaf → P1 admits nothing (RA-23); all 16 webs + IV spill with
  `hazard-or-register-pressure`. The span-pressure valve
  (RA-PRESSURE-2) evicts loop-hot spans for single-use temps in
  EVERY wave: the static future-use count (≤2) ignores dynamic
  loop-trip weighting — 2 static in-loop uses = 20 dynamic reloads
  after eviction.
- Tried and REVERTED with measurements (no speculation shipped):
  `CCC_RA_LOOP_SPAN_RESERVE` 1..8 sweep: 174→172 spills (dead —
  the valve undoes the cap); forcing P1 hot-loop admission
  (`CCC_NO_LEAF_CALLER_HOME=1`): 174→185 spills (valve evicts in
  P1 too); 2c scan with valve off: 33→64 spills; global valve off:
  33→91 spills. The valve is load-bearing; both starvation
  directions were measured and the current one wins.
- Secondary gap (static): lccc runs the 4 independent QR chains
  serially (98-instr body in program order); GCC interleaves them.
  No scheduler project exists; unrolling is NOT the answer (GCC
  keeps the loop rolled, and ×2 would worsen the 12-reg pressure).
- Roadmap (ordered, for the splitting project): (1) live-range
  splitting at demotion (evict-and-requeue) — the documented fix,
  still correct; (2) pool enlargement — rax/rcx are unallocatable
  (caller pool is 6 of 11 caller-saved SysV regs); the rdx-wave
  hazard-filtered pattern is the template, needs implicit-use
  hazard points for mul/div/shift/returns; (3) valve↔cap
  coherence (cap-homed spans valve-immune) only as a bounded
  experiment with a full-corpus census; (4) QR-chain-aware
  scheduling, long-term. The `CCC_NO_SPAN_VALVE` switch and this
  record are the starting kit.

## 7. Validation record (final tree, this machine)

- `cargo test`: 2124 passed, 0 failed (6 ignored) — incl. 3 new
  identical_blocks tests and the LCSSA unroll fixture.
- `cargo clippy --all-targets --profile fastbuild --locked -j 2 --
  -D warnings` (exact CI invocation): clean.
- `cargo fmt --all -- --check`: clean. `bench.yml`: YAML-valid.
- Regression corpus: 686 passed, 0 failed (14 skipped_compare,
  2 skipped_run; 702 total).
- Benchmark output gate (`check_benchmark_outputs.sh`): 180/0/0.
- Correctness suite: 53/53 (incl. new abs-fold test, red-validated).
- Differential engines 13/13, synthetic fuzz 4/4, inline-asm utf8
  preserved, recip + machinst oracles, linker fuzz 128 + 64-iter
  grammar (0 defects), toolchain selector: all pass.
- Behavior proofs: cold section + skip, trip-4/5 boundary, explicit
  flag override, padding-merge + `.loc` unit tests (red-validated),
  abs-fold red/green — see §4/§5.
- `ms178-1.patch` apply-check: APPLIES-CLEAN (verified by the
  snapshot tooling off-path against base `b6d6e7bd`).

## 8. Patch

`ms178-1.patch` = `git diff b6d6e7bd..unified` (source + tests +
CI + this doc; no repro dumps, no evidence dirs). Snapshot series
in `/home/user/artifacts` (ledger, tarball, bundle).

## 9. S03 gate failure → remat widening → rbp-alloca → gate accounting (2026-09-08, this turn)

PR #447 (S03 supreme) failed Benchmarks step 10 (golden codegen gate):
`glibc_memcmp` stackmem 27 → 29. Root-caused empirically (main-vs-S03 A/B
in a detached worktree at `1a606ff`, `CCC_DUMP_IR` IR diff, asm diff):

1. S03's extra copyprop/folding lengthens live ranges (fewer, longer-lived
   values); the `glibc_left` address, previously homed in callee-saved r12,
   spills (store + reload = +2). The spill is a bare `GlobalAddr` used as a
   direct-call argument — a use shape the remat set rejected (`_ => false`),
   so the value kept a home and spilled instead of rebuilding by LEA.
2. Fix ( principled, not tuned): admit call-argument uses into
   `build_rematerializable_global_addr_set_for` — direct calls (excluding
   inline-expanded memcpy/memset, which bypass the call emitter, plus
   unaudited sret/fastcall paths) and indirect-call args (target excluded).
   The consumer side already existed (`value_to_reg_inner` rebuilds
   home-less GlobalAddrs for every generic loader); the IntReg staging arm
   additionally stages the LEA directly into the argument register, since a
   LEA-into-%rax + mov relay cannot fold through the call barrier.
   memcmp stackmem 29 → 27 (gate green on that workload).
3. The widening freed two callee-saved regs in stencil5's `main`, flipping
   allocation (hash-order butterfly): `argv` homed in rbp(6) instead of
   r12(2), which exposed a pre-existing inconsistency — the x86 prologue
   passes the 5-reg `X86_CALLEE_SAVED` (no rbp) to the dead-param-alloca
   stability check while the FPO pool is `WITH_RBP`, so the dead argv slot +
   entry store were kept (+1 real). Fixed by passing the FPO-matching list
   (sole consumer of that slice is the stability check). stencil5 8 → 7.
4. The remaining +1 was a gate measurement artifact: `movq 8(%rbp),%r10`
   (argv deref through the rbp GPR — optimal encoding, no SIB) matched the
   `(%rbp|%rsp)` stackmem regex. Fixed the measurer, not the code:
   `ci-codegen-gate.py` now scores per-function and excludes `N(%rbp)` only
   on positive proof of GPR use (pure-write whitelist; `mov %rsp,%rbp`
   excluded as the fp setup). The proof can never fire on a true
   frame-pointer function, so no real spill can be masked; a missed writer
   merely keeps counting. Self-test `test_ci_codegen_gate.py` (6 cases)
   wired into bench.yml ahead of the gate.
5. Rebaselined honestly: `ci-codegen-baseline.json` regenerated from
   current main (`1a606ff`) under the corrected measurer (old baseline was
   also stale — main had drifted). S03+fixes vs new baseline: every metric
   of every workload better-or-equal (memcmp stackmem 28 → 22, insns
   226 → 206; sqlite insns 326 → 302; stencil5 6 = 6).

Validation (all green): unit 7/7 incl. 6 new `remat_call_arg_tests`;
regression+SSA 688/688; bench outputs 180/180; correctness 53/53; loop
alignment PASS; fuzz smoke 0 FAIL; inline-asm utf8 PASS; fib rec2iter PASS;
full benchmark corpus `--strict` (timing + output gate, §10); codegen gate
green on both the patched and the main binary.
