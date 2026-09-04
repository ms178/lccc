# Audit: "ms178-ultimate" (Agent C) vs. this revision — and the re-based final tree

Date: 2026-09-04
Both revisions share the base `2d3d5d78` (tree of merge `82d70096`, PR #374).
This tree is the S02 snapshot **re-based on latest upstream main `4abb2aa3`**.

## 1. Provenance — exact identity of the competing revision

| Question | Evidence | Answer |
|---|---|---|
| What base is the uploaded patch against? | pre-image blob of `src/passes/loop_unroll.rs` is `16adaddb`, found in history only at `2d3d5d78` | **same base as ours** |
| Was it merged upstream? | main's `044bee74` ("Complete-unroll single-trip loops and widen vectorizable reduction shapes") merged as PR **#378**; `git diff 044bee74 4abb2aa3` for the two source files is empty | **yes — the audit target is current main `4abb2aa3`** |
| Is merged #378 byte-identical to the upload? | #378 tree vs. upload applied on the base: `git diff` over `loop_unroll.rs` + `vectorize.rs` = **empty**; the merge simply omitted their 4 test files | source identical; tests dropped upstream |

## 2. The assigned bugs on current main (= Agent C's revision)

Measured this session with the pristine `-O1/-j2` release build of `4abb2aa3`, compared to
`gcc -O3`:

| Repro | correct | main (Agent C) | verdict |
|---|---|---|---|
| cfg 1765 — runtime-trip `for (i=5; i<=lim; i+=7)` with a straight-line chain body and an early-exit `return` (`tests/regression/unroll_chain_body_exit_phi.c`) | 30 | **0** | still miscompiles |
| cfg 3169 — unsigned countdown `for (i=6; lim != i; i -= 1)` with chain body (`unroll_countdown_chain_body.c`) | 126 | **252** | still miscompiles (regression vs. base `82d7009`: 126) |
| guarded sum `if (a[i] > 0) s += a[i]` (`vector_guard_sum.c`, `nat=` and `rhs_rt=`) | 651 / 651 | **612 / 612** | still miscompiles (widening and swapped forms are handled) |

**Agent C's revision fixes none of the three assigned miscompiles.**

## 3. Verdict: it is not better — it is a strict subset of our revision

Three-way diff on the same base:

- Every hunk of their patch in `loop_unroll.rs` / `vectorize.rs` is present verbatim in ours;
  ours adds +348 / +65 lines that they do not have (their hunks ⊆ ours).
- Their content is missing, and is required by the assigned bugs:
  1. **do_unroll Step-5b terminator rewrite** (`subst_value_in_terminator` in the post-exit
     reader walk) — the actual fix for cfg 1765 (the early-exit `Return` kept the header
     phi's PREHEADER value: 0 instead of 30).
  2. **strict Add-only `find_iv_in_loop` for `analyze_loop`, with Sub-form recognition
     confined to `find_iv_in_loop_ext` for the complete unrollers** — fixes cfg 3169 (their
     Sub-accepting detector, feeding `analyze_loop`, is precisely what turned a rolled-but-
     correct countdown into the 252 miscompile) while keeping countdown recognition where
     the closed form can verify it.
  3. fail-closed `analyze_loop` body-connectivity + linear-chain guards (the exact
     precondition do_unroll's guard arithmetic is derived for).
  4. the **equal-width masked guarded-sum** transform (`VecMaskedAddI32x8`) + I32-only
     requirement — the actual fix for the `nat=`/`rhs_rt=` miscompile (their patch only
     rejects the *swapped* form, which was already correct-by-scalar).
  5. the intrinsic/regalloc pieces the transform needs, `scripts/unroll_stress.py` (the
     4000-config differential that found cfg 1765/3169), the pinned regression tests, and
     oracle/doc updates.
- Their content that ours lacks: **nothing**. Their 4 test files have the same names and
  same coverage we carry.
- Why they missed the bugs: their corpus contains no runtime-trip chain-body early-exit
  shape and no countdown chain shape — the two shapes live entirely in the *partial*
  unroller (`do_unroll`), which their revision never touches.

Where their work is genuinely good — and note this code is literally shared, since ours
contains it too — is the complete-unroller architecture: i128 closed-form trip arithmetic
in the comparison's own signedness domain with wrap refusal and final-value bounds, the
per-iteration `LoopPhiModel` SSA environments (fixing the whole carried-phi class: constant
backs, invariant backs, cross-phi swaps), canonical compare-normalisation (mirror/negate),
the persisting-inner-loop fail-closed gate, and the vectorizer's unhandled-loop-carried-phi
reject. These we adopt fully and verified as the shared core.

## 4. Root causes (evidence from the per-pass IR dump of main on cfg 1765/3169)

The inner `j`-loop is completely unrolled first (3 copies); the runtime-trip outer loop is
then handed to `do_unroll`. Step 5b inserts a proper exit phi for a value the exit block
reads directly (no pre-existing exit phi), then rewrites post-exit *readers* of the stale
header phi — but only in **instructions**. The `Return` terminator of the exit block kept
the header phi's SSA name, which on the early exit-check edges holds its **preheader**
value (the phi update rides the latch edge an early exit never takes) → cfg 1765 returned 0
instead of 30. The identical structural break, combined with the Sub-form IV acceptance,
doubles the accumulator in cfg 3169 (252 instead of 126); the strict/partial IV split keeps
that shape rolled and correct, and the extended detector remains where the closed-form trip
arithmetic re-verifies the step.

## 5. The re-based tree

`main @ 4abb2aa3` + this delta (`<Agent-C tree> → <S02 tree>`, 17 files, +1362/−84; the
harness-owned `scripts/lccc-harness-snapshot.sh` intentionally keeps main's version),
plus comment corrections so the 5b/5c guards state precisely what they do (they do NOT
trigger on cfg 1765/3169) and the `trip_range` doc matches the `1..=1`/`1..=16` call sites.

Per instruction, no further builds, tests or differentials were run after the re-base;
the validation of the base-deliverable state is recorded in the S02 snapshot artifacts.
