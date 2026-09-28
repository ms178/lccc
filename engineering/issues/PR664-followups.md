# Issue-ready follow-ups from PR #664 review

These are **local issue drafts**, not remotely filed GitHub issues. Base audit
and inventory: [review](../../docs/reviews/PR664-followup.md). Each semantic
consumer should be a separately reviewable change with a base-failing test,
not a wholesale pass rewrite. No authenticated issue-write interface was
available; do not invent issue numbers or closed statuses.

## P0 — Full-width Select identity in constant_fold

Evidence: `try_fold_with_map` compares resolved Select arms through `to_i64`.
Acceptance:
- Unknown condition with I128(0) and I128(2^64) must not fold to one arm, in
  either arm order. Test the transformation directly and a C integration case.
- Preserve exact float bit-key equality; signed-zero and distinct NaN payloads
  must not collapse.
- Compare before/after emitted code and pass fast CI; leave slow CI to hosted.

## P0 — SCCP Switch executable edges and rewrite must agree

Evidence: `evaluate_terminator` and `fold_terminator` both narrow to i64.
Acceptance:
- Share a type-width contract across executable-edge discovery and final
  rewriting. Test I128 high-half-only values, negative cases, narrow unsigned
  cases, and default destinations with phis.
- Include a base-failing discovery test, not only final assembly.
- Preserve unsupported-type conservatism and run SSA validation in hosted CI.

## P0 — IPCP interpreter truth and dispatch

Evidence: Select, CondBranch and Switch still use i64 extraction; Cmp/Cast
already delegate to typed constant-fold evaluation.
Acceptance:
- Pure-call evaluation with high-half-only condition and Switch inputs agrees
  with direct execution. Include both true/false and default/case paths.
- Unsupported floating/decimal carriers are either correctly typed or declined,
  never silently interpreted as an integer's low word.

## P0 — if_convert boolean-shape recognition

Evidence: low-word 0/1 recognition and same-value-or-both-zero predicates.
Acceptance:
- For every affected transformation family, demonstrate the input type/range
  precondition or add a base-failing I128 case. Do not blindly replace all
  extraction calls without reading the consumers.
- Preserve profitability decisions and record generated-code regressions.

## P1 — Canonical cast evaluator's narrow unsigned carriers

Evidence: `constant_fold::fold_cast_i128` zero-extends unsigned sources through
u64, without masking sub-64-bit source width.
Acceptance:
- Alternate valid carriers of U8/U16/U32 zero-extend to their declared widths,
  not to u64 sign-extended representations. Test signed source directions too.
- Reconcile the now-tested CFG helper contract with the canonical evaluator
  before moving more callers. Preserve decimal/Float128/pointer distinctions.

## P1 — Resolver work budgets and memoization

Acceptance:
- Shared-DAG, repeated-operand, cyclic and long-chain fixtures expose work
  counters, not just elapsed time. Depth limits alone do not bound total work.
- A tri-state memo table and explicit budget must remain deterministic, decline
  unresolved cycles safely, and invalidate after CFG/definition changes.
- Measure compile-time, peak memory, and unchanged/generated code separately.

## P1 — Asm-goto edge-aware Phi repair and fusion preconditions

Acceptance:
- Folding a terminator must not delete a Phi input when an asm-goto edge from
  that same predecessor survives.
- Reject or diagnose missing fusion inputs; never fabricate zero or pick an
  arbitrary unrelated incoming operand.
- Retain LabelAddr/static-initializer label identity and validate source spans.

## Closed within this patch, subject to CI

- CFG full-width Cmp/Cast/Select/Switch evaluation, typed comparison contract.
- Binary128 payload-based LongDouble truthiness.
- Real predecessor blocks and verifier assertion in the wide-Phi unit fixture.
- Fuzzer verdict discovery in both mirrors, with parity mutation protection.
- Reference/candidate/both execution failure buckets and runner-role assertions.
- Root-scoped scratch test ignore rule, no per-source-test exceptions.
- Archived operational report, durable review and pinned inventory.

## Evidence boundary

No claim of exhaustive correctness, completed backend audit, benchmark win,
PMU measurements or full hosted CI completion. The inventory's unreviewed
rows remain review candidates even if a neighboring function has a width guard.
