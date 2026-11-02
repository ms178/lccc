# PR #716 — audit adjudication and follow-up

The review filed four high-risk findings and a set of smaller items against the
matmul-FMA round (PR #716, 15 files, +1674/−35). This document adjudicates each
one against evidence reproduced with the compiler on this tree, not against the
report's text. The review ran no local tests; three of its five claims are
confirmed by building and running the shapes, one is confirmed as a defect but
downgraded from "miscompile" to "unreachable mistyping" with the measurement
that shows why, and one is confirmed as a **memory-safety miscompile whose
severity the report understated** — the previous compiler *segfaults*.

Base: `2f89b1af` (`origin/main`, after the PR #713 merge). Adjudicated tree:
`87a1d691` plus the fix commit this document ships with.

## 0. Verdict

| # | Finding | Verdict | Evidence |
|---|---------|---------|----------|
| 1 | Flags hazard: "full writer" list holds partial writers; taken edge never scanned | **AGREE** (defect is real; the report's location and example are imprecise) | 4 new unit tests pin the rule; 0 refusals over the 52-program corpus, so the stricter proof costs no reach |
| 2 | FMA matcher proves continuity, not polarity; inverted break runs the wrong trip count | **AGREE — reproduced** | `limit=0` printed `0.000000` vs oracle `10623.257143`; `limit=1` the two outputs were swapped; `limit=3` `31485.114286` vs `0.000000` |
| 3 | FMA legality is a blacklist; terminator wildcard; bound invariance unproven | **AGREE — reproduced** | `asm volatile` counted 16384 of the scalar 262144; switch/side-exit/indirect refused; bound proof replaced twice (see §3) |
| 4 | I32 remainder over I64 IV for `long` indices | **AGREE as a defect, DISAGREE as a miscompile** | pre-fix the arm already matched, emitted 4 packed FMAs and printed the oracle's `217645.914286`: mistyped IR, no reachable wrong result |
| 5 | Reduction exit comparison ignores the operator; `<=` can run an extra vector group | **AGREE, and STRONGER than filed** | guard-page probe at n=63: pre-fix **SIGSEGV (exit 139)**, post-fix `1953 1953 1953` |
| a–e | i32 wrap on the inclusive bound; volatile policy comment; awk escape; stale `loop_rotate` comment; audit-repro doc section | **AGREE, all five closed** | §6 |

## 1. F1 — the flags hazard: the mechanism is right, the report's locus is not

The report says the scan "trusts `flags_full_writer`, which lists partial
writers (`clc`, `stc`, `cmc`, `rol/ror`, `bt/bts/btr/btc`, `jmp`)". In this tree
`flags_full_writer` does not exist; the corresponding lists are
`flags_written_mask` (`flag_peepholes.rs:87`), `flags_effect` (`:160`) and
`F_ALL` (`:58`). The substantive claim is nonetheless correct, and the deeper
defect is the one the report only implies: the deleted check was a **bounded
linear window over the text after the jump** which never followed the jump's
target at all, so a reader that lives only in the target block was structurally
invisible. Two independent bugs, both real:

* **the walk's stop rule** treated "wrote some flags" as "flags dead", so a
  partial writer (`clc` writes CF, `jmp` writes nothing) ended the hazard scan
  while the flags under test survived it;
* **the taken edge** was never seeded, so `cmp; setl; movzx; test; jne .L; clc;
  setne` — the report's counterexample — would fuse and then read the producer
  `cmp`'s ZF through the `clc` where the deleted `test`'s ZF used to be. That
  example is valid *because* a `setne` follows the branch: `clc` alone is not a
  hazard, a *consumer* after a partial writer is.

Fixed by deleting both lists and routing the question through a CFG-aware
walker, `flag_peepholes::flags_reach_consumer_after_branch`, seeded with **both**
successors of the fused jump, stopping a path only at a writer that redefines
every flag still in question (`preserved = 0`: `test` and `cmp` disagree about
all six), at a call (flags are dead across the ABI boundary), at a `ret` or at
inline asm — which is charged as a reader *and* leaves the walk unproved.
Unresolvable targets, indirect jumps, and fragments with no `.cfi_startproc`
refuse. Failing closed is only meaningful if it is measured: across all 52
programs in `tests/benchmark/programs` the new guard refuses **zero** times, so
the corpus output is unchanged.

Four legacy cmp-fusion unit tests needed a real function frame and predicate-pair
assertions (`jl .LBB2` or its mirror `jge .LBB4`): they asserted one orientation
of a bare fragment, where the flags question is unanswerable by construction,
and later layout passes legitimately re-orient a provably-taken branch.

## 2. F2 — inverted-break polarity: reproduced, fixed, pinned

The old matcher counted in-loop successors but never asked *which* successor
continues the loop, while `insert_remainder_loop` unconditionally treats
`false_label` as the vector-exit edge. Reproduced before the fix, with `gcc -O0`
as the oracle:

| limit | pre-fix lccc | `gcc -O0` |
|-------|--------------|-----------|
| 0 | `0.000000` | `10623.257143` |
| 1 | `10623.257143` | `0.000000` |
| 3 | `31485.114286` | `0.000000` |
| 17 | `178896.657143` | `178896.657143` |

`limit=1` swapping the two outputs is what an off-by-one on the exit edge looks
like. The matcher now requires the canonical orientation (the true edge must
continue the loop) and refuses with *"matmul loop header is not in canonical
orientation (true edge must continue the loop)"*; all four limits above match
the oracle. Gate contract 1 pins them.

## 3. F3 — legality: blacklist → allowlist, and a real bound proof

`asm volatile` in the body was the reachable case: the per-iteration side
effect rode along on one lane of each vector group, so a 64³ kernel counted
**16384** asm executions where the scalar loop runs **262144**. `Switch`,
`IndirectBranch` and `Memcpy` fell through the wildcard arm; the loop bound was
moved and recomputed with no proof it was invariant. All three sub-claims
accepted.

The fix is structural rather than a longer blacklist: `fma_body_instruction_is_legal`
is an **allowlist** — anything not named legal refuses — the terminator must be
the canonical conditional, an escape scan covers values live after the loop,
and `limit_is_loop_invariant` proves the bound before the remainder is built.

Two honest notes on process, because both cost reach and only measurement found
them:

1. The first bound check was the crude `defs.contains(&lim.0)` ("defined in the
   loop ⇒ refuse"). It over-refused a reloaded global or parameter, which *is*
   invariant: `long_iv_rem`, `p4_long_iv_rt` and `p3_asm_counter` stopped
   matching (0 matches). It was replaced by the recursive proof — constants and
   values defined outside the loop are invariant by SSA; a `Phi` refuses; casts,
   unary ops, binops, comparisons and GEPs recurse; a `Load` is invariant only
   when every in-loop `Store` root is *proven distinct* (`proven_object_root` +
   `roots_proven_distinct`), unknown roots and volatiles refuse; depth cap 8;
   everything else refuses. The three shapes match again, and
   `long_iv_rem` is oracle-correct at `217645.914286`.
2. `p3_asm_counter` legitimately prints 0 packed FMAs: the allowlist refuses
   the shape. A "fix" that made it vectorize again would be the bug.

## 4. F4 — `long` index: a defect, not a miscompile (measured)

The report claims invalid IR / truncation / wrong code for `long` indices. The
typing defect is real: the matcher accepted I32 and I64 IVs, and the inserted
remainder was built from `pattern.iv` / `pattern.limit` with hardcoded I32
constants, phi and compare. But the *runtime* claim does not hold, and this is
the measurement that settles it — the true pre-fix compiler (`src` at
`87a1d691`) on a runtime-bound `long`-indexed matmul:

```
[VEC] Matmul pattern matched! Transforming to FmaF64x4 (AVX2, 4-wide)
packed FMAs: 4
runtime: 217645.914286   (gcc -O0 oracle: 217645.914286)
```

So the arm already fired, and the mistyped remainder produced correct machine
code for every representable trip count (the widths only diverge past 2³¹
elements — a 2-billion-element row). `CCC_VALIDATE_SSA=1` does not flag it
either: lccc's IR carries the type as a hint, not as a verifier-enforced
invariant. The correct framing is therefore *latent mistyping in emitted IR*,
not a shipped wrong answer.

Fixed anyway, and properly: the pattern carries `iv_ty`, `idx_const(iv_ty, v)`
builds every constant at that width, and the remainder's phi, latch and exit
compare all use `ty: iv_ty`. The fix is correct *by construction* — there is no
type-checking verifier in this compiler to lean on, so the argument cannot be
"the verifier would catch a regression"; it is that no remainder value is
built from an assumed type any more. What the gate adds is the orthogonal
half, and it now runs the structural verifiers (`CCC_VERIFY_IR=abort`, which
panics inside the pass loop naming the offending pass, and `CCC_VALIDATE_SSA=1`)
over every contract in the file, plus the reach control: the shape still
vectorizes (4 packed FMAs — a "fix" by disabling the arm fails here) and matches
the oracle.

## 5. F5 — the reduction exit comparison: understated, and it segfaults

`reduction_exit_is_canonical` was dead code: no call sites, and typed for
`VectorizablePattern` rather than `ReductionPattern`. Meanwhile the real
reduction analyzer chose its vector bound from the exit comparison's limit
while **ignoring its operator**, and the transform re-emitted the loop as the
canonical exclusive test. The report calls this a "pre-existing risk" that
"can process an extra vector group".

It is a miscompile, and it is memory-unsafe. Reproduced with a guard-page
probe: the array is placed so its last element ends exactly at a `PROT_NONE`
page, with `n = 63` (not a multiple of the vector width). An over-read then
faults instead of quietly reading a neighbouring byte:

| compiler | probe result |
|----------|--------------|
| pre-fix | **SIGSEGV, exit 139** |
| fixed | `1953 1953 1953` (scalar fallback for `<=`, vector for `<` and `!=`) |

Two things are worth stating plainly, because both were mistakes *I* made while
adjudicating this finding:

* The first sweep compared lccc against lccc (an invalid oracle). Comparing
  against `gcc -O0` is what the other four findings used; the same standard must
  apply here.
* The second sweep only tested trip counts that are multiples of the vector
  width, where the over-read is invisible by construction. `n = 63` is what
  makes it a fault, and a guard page is what makes it *stay* a fault instead of
  depending on the allocator.

The fix: the helper is retyped for `ReductionPattern` and wired into reduction
dispatch *before* any mutation; any exit comparison that is not the canonical
exclusive IV test (`<` with the IV on the left, `Ne` included) falls back to the
scalar loop, with the refusal visible under `LCCC_DEBUG_VECTORIZE=1` as
*"Reduction refused: exit comparison is not the canonical exclusive IV test"*.
This is a fail-closed refusal, not a rewrite: `<=`-shaped reductions lose the
vector path, and the reach that remains (`<`, `!=`) is asserted by contract 6
(8 packed adds still emitted on the same file).

## 6. The smaller items — all five accepted and closed

* **i32 wrap on an inclusive bound.** The exclusive bound was materialised at
  `INT32_MAX + 1` and cast `as i32`, violating the module's own fail-closed
  claim. Now `i32::try_from(plus)` with a refusal.
* **Volatile RMW policy comment.** "Stronger, never weaker" was not a contract
  for MMIO or interrupt observers. The policy in `memory_fold.rs` is narrowed to
  what the model actually holds: access count and order are preserved for
  ordinary memory in a single-threaded program; MMIO, ISR and signal observers
  are outside the model, and `_Atomic` is the supported spelling.
* **Non-POSIX `awk` escape.** `check_rmw_sib_folds.sh` now uses `[[:space:]]`.
* **Stale `loop_rotate.rs` comment** claiming the operator is mirrored. The
  implementation preserves it (`N op (i + C) == (N - C) op i`); the comment now
  says so.
* **The audit-reproduction section** in `docs/reviews/PR713-audit-response.md`
  pointed at `ms178-1.patch`, which is not in a checkout, and passed four
  filters to one `cargo test` invocation — libtest takes exactly one positional
  filter, so three of the four families were silently never run. §14 is now
  literally executable from a clean checkout, with one filter per invocation.
* **FMA oracle strength** (`check_fma_gating.sh`): the runtime comparison was a
  single `%.6f` of the total, which hides a wrong element that cancels — exactly
  the failure shape a vector remainder produces. It now compares a per-element
  weighted fold printed twice (forward and reversed) plus the exact bit
  patterns (`%a`) of the weighted total and two elements. The FMA counter also
  only knew `vfmadd`: `total()`/`packed()` now match every spelling
  (`vfnmadd`, `vfmsub`, `vfmaddsub`, `vfmsubadd`, the 4-digit forms like
  `vfmadd132sd`) with the mnemonic required to be the whole first token, so a
  label cannot be counted. Verified on synthetic asm: 5 of 5 spellings counted,
  3 packed, the label excluded.

## 7. What the review did not file, found here

* **An unwired gate, now wired.** `tests/regression/check_redundant_test_elimination.sh`
  (the `andl`/`orl` + `test` fold, which guards the same flag-reasoning domain
  this round reworked) passed locally but ran in no CI mode. It was a *tracked*
  gap rather than a silent one — `scripts/ci_gate_allowlist.txt` listed it with
  "passes locally; not yet wired" — and the parity checker failed the moment it
  became wired-but-still-allowlisted, which is the invariant working. It now
  runs in `ci_local.sh` (fast) and in the hosted workflow, with its allowlist
  entry deleted (parity: 126 commands). The remaining allowlisted gates are a
  standing, explicit list; expanding CI to cover them is a separate decision
  from this round's correctness work.
* **The reducer's reach ledger.** Refusing non-canonical exit comparisons is a
  behaviour change; it is measured, not assumed: `<` and `!=` still vectorize on
  the same source file (8 packed adds), and the corpus reach counter for the
  matmul arm is unchanged at 1 program (baseline vs current).
* **Probe hygiene.** `p3_asm_two_writes` could not build (`%rip` needs `%%` in
  the asm string, and the probe lacked the counter definition). Both fixed; the
  probe now reports `524288` — two asm writes per iteration, both preserved,
  because the shape is refused.

## 8. Reach ledger

| Guard added this round | Cost on the corpus | Control |
|------------------------|--------------------|---------|
| F1 flags walker | 0 refusals in 52 programs | `check_select_from_compare.sh` PASS; lib suite green |
| F2 orientation | 0 (only non-canonical loops) | contract 5: canonical `<=N-1` matmul keeps 4 packed FMAs |
| F3 allowlist + bound proof | 0 after the real proof | `long_iv_rem` / `p4_long_iv_rt` / `p3_asm_counter` re-match; `/tmp/reach.sh` total 1 == baseline 1 |
| F4 `iv_ty` | 0 (fixes typing) | contract 4: runtime-bound `long` matmul still emits 4 packed FMAs |
| F5 exit polarity | `<=`-shaped reductions only | contract 6: `<`/`!=` still emit 8 packed adds |

## 9. Verification state

* `cargo test --profile fastbuild -j2 --lib`: **3954 passed, 0 failed, 7
  ignored**.
* `tests/regression/check_fma_matcher_guards.sh`: 6 contracts PASS, and the
  pre-fix mutation of the same gate fails exactly where it should — `p2_0`
  `0.000000` vs `10623.257143`, `p2_1` swapped, `p2_3` `31485.114286` vs
  `0.000000`, the asm and carried counters `16384` vs `262144`, and the
  guard-page probe `lccc exited non-zero` (the segfault).
* The adversarial probe suite (39 cases: polarity, asm, switch, side exits,
  varying bounds, `memcpy`, long IVs, reduction clamps): **every case agrees
  with `gcc -O0`**, and none fails to build.
* `cargo fmt` and `cargo clippy --lib -D warnings` clean.
* Full `scripts/ci_local.sh` (all gates, including the three slow ones) is the
  stamp this revision ships with; gate parity between `ci_local.sh` and the
  hosted workflow was re-checked after registering `fma-matcher-guards`.

## 10. Reproduce

Every command runs from a clean checkout of this revision (build the compiler
first: `cargo build --profile fastbuild -j2`).

```sh
# the two runtime miscompiles, one contract each
CCC=target/fastbuild/lccc bash tests/regression/check_fma_matcher_guards.sh

# the FMA contract/target matrix, elementwise against gcc -O0
CCC=target/fastbuild/lccc bash tests/regression/check_fma_gating.sh

# the pins: the flags walker, the fusion, the orientation and the typing
for f in fusion_flags_flow flags_horizon compare_branch_fusion vectorize; do
    cargo test --profile fastbuild -j2 --lib -- "$f" || exit 1
done
```

The guard-page probe that reproduces finding 5 in isolation is contract 6 of
`check_fma_matcher_guards.sh`; it is deliberately a faulting probe, because an
over-read that stays inside a large array is invisible to a plain run — which
is precisely how this finding survived the first round of testing.

## 11. The design principle

Three of these five findings are the same mistake in different clothes: **a
proof replaced by a list**. A blacklist of "effects we know are bad" cannot be
complete (`asm volatile` proved that), a list of "writers that end a hazard"
cannot be complete when the hazard is about the flags a *deleted* instruction
set, and counting in-loop successors is not reasoning about the edge that
exits. The fix in every case is the same shape: name what is *proven* legal,
refuse everything else, and prove the properties the transform depends on
(orientation, invariance, distinctness, polarity) instead of assuming them.

And every guard has to carry its own reach measurement, because a refusal is
silent. The corpus counter, the packed-FMA counts and the probe suite exist so
that "fail closed" costs a number, not an argument.

---

## 12. Re-adjudication on latest main (2026-10-02)

The review re-filed the same five findings against "PR #716" a second time,
this time with a finding-by-finding instruction list and a 4/10 readiness
rating. Two facts had to be established before adjudicating, and both are
checkable in one command:

1. **The revision the review describes is not the current one.** `git log
   --oneline --stat -- tests/regression/check_fma_matcher_guards.sh` shows that
   gate entering history in `fc174f03` (upstream PR #717) with the same
   19-file, +2851/−119 diffstat this document's §9 recorded — and the file is
   byte-identical to the S08 deliverable (`md5 5b7766a8530d6af74ed128d87000c968`).
   The guards this document describes (§1–§6) are in `origin/main`, not merely
   in a patch: `flags_reach_consumer_after_branch`, `limit_is_loop_invariant`,
   `idx_const(iv_ty, …)`, the canonical-orientation refusal, and
   `reduction_exit_is_canonical` are all present in `origin/main`'s sources.

2. **Every finding is closed by that code, and this time it was measured on
   the current tree rather than argued.** The verdict table below records the
   commands; all of them were run against `origin/main` at `67a29473` (after
   the two later upstream commits `fc174f03`, `71e6d49c`) with a fresh
   `fastbuild`.

| # | Review's finding | Status on `67a29473` | Evidence |
|---|------------------|----------------------|----------|
| 1 | flags scan stops at a partial writer; taken edge unscanned | **Closed** | `fusion_flags_flow_tests`: 6 tests, each refusal paired with a live control (a `clc`+`setne` case must refuse; the same text with a full writer must fuse). Plus `refuses_when_the_reader_sits_only_in_the_taken_target` and its control. |
| 2 | FMA matcher does not prove the continuation edge | **Closed** | the matcher requires the canonical orientation, refusal string present at `vectorize.rs:1334`; `p2_ne_break` at limits 0/1/3/17 all match `gcc -O0`, 0 matmul matches on the inverted shape |
| 3 | legality check is an incomplete blacklist | **Closed** | allowlist + terminator refusal + `limit_is_loop_invariant`; the switch shape refuses with *"matmul loop has control flow the transform has not proven safe"*, the internal-branch shape with *"defines a value that is read outside it (unmodeled loop-carried state)"* — distinct diagnostics, not a wildcard |
| 4 | I64 IV accepted, I32 remainder generated | **Closed** | `idx_const(iv_ty, …)` at `vectorize.rs:17764`+; the guaranteed zero-trip shape was moved onto the width that can actually truncate (see §13) |
| 5 | the reduction guard is dead code | **Closed** | `reduction_exit_is_canonical` has 2 references in `origin/main`'s `vectorize.rs` (definition + the pre-mutation dispatch call); `check_fma_matcher_guards.sh` contract 6 asserts the refusal and the preserved reach |
| a | i32 inclusive-bound wrap | **Closed** | `i32::try_from(plus)` refusal |
| b | volatile RMW policy | **Closed, and now gated** | §13, contract 6 of `check_volatile_access_semantics.sh` |
| c | FMA oracle strength / spellings | **Closed** | elementwise fold compared forward and reversed plus `%a` bit patterns; `total()`/`packed()` match every FMA spelling |
| d | awk escape; audit-repro section | **Closed** | `[[:space:]]`; §14 runs with one libtest filter per invocation |
| e | stale `loop_rotate` comment | **Closed** | the comment states the preserved-operator algebra |

Also verified on the current tree, since "closed" is a claim about behaviour,
not about text: **39/39 adversarial probes agree with `gcc -O0`** (including the
guard-page reduction probe that segfaulted the pre-fix compiler and the
asm-effect counters), all four FMA/reduction/fusion gates PASS, and the 33
targeted library tests in the touched modules PASS (4 `fusion_flags_flow`, 3
`flags_horizon`, 24 `compare_branch_fusion`).

## 13. What the review did not file (this round)

Two residual holes were found by attacking the guards rather than re-reading
them; both are now closed, and both cost nothing on real output because that
cost was measured first.

* **A data directive was invisible to the flags walker.** The walker stepped
  over every `LineKind::Directive` as "a position, not an effect". That is
  right for alignment and CFI, and wrong for `.byte`/`.long`/`.quad`: a hand
  encoded sequence is an instruction the textual predicates cannot see —
  `adc`, `clc`, a branch — which is exactly the class inline asm is already
  charged for. The walker now classifies directives: the benign set is skipped,
  a data directive is charged as a reader of every flag *and* leaves the walk
  unproved (fail closed), and anything unrecognized is treated as data. The
  reach cost is zero by construction: **0 data directives inside function
  bodies across all 52 corpus programs** (measured). Pinned by
  `refuses_across_a_data_directive_that_can_encode_an_instruction` with a
  benign-directive control that must still fuse.
* **The volatile/`_Atomic` policy was documented but not gated where
  volatility is still visible** — the review's item (b), which asked for
  exactly this. Contract 6 of `check_volatile_access_semantics.sh` builds
  `_Atomic int` and `volatile int` accumulate loops, compares the values
  against `gcc -O0` (3000 3000), and asserts the *shape*: the object is still
  referenced inside a region bounded by a real back edge, so no pass collapsed
  N read+write pairs into a closed form. The contract states what it proves
  (shape + value) and what it does not (an access COUNT is not observable from
  stdout in a single-threaded program); the detector was checked against a
  hand-written closed form, where it fails as intended.

One further probe class came back **sound**, and is recorded because a refusal
is a boundary worth measuring: store indices that are affine in the IV but not
equal to it (`C[i][j+1]`, `C[i][2*j]`, a loop-carried `t`) are all **refused**
(0 matmul matches) and oracle-correct, so the matcher requires the bare IV
rather than accepting any IV-affine address.
