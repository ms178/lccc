# FOLLOWUP — independent review of the PR #578 audit, and the perfected follow-up

**Date**: 2026-09-22 · **Base**: `main` @ `1ff53e85` (PR #578 merge of `66c8687d`)
**Scope**: `src/passes/slp_vectorizer.rs`, `src/passes/if_convert.rs`,
`scripts/` (ci_local, parity checker, allowlist), `.github/workflows/ci.yml`,
`tests/regression/` (battery + new wide-lane select test).

This session took the Review-AI audit of PR #578 as input, **re-derived every
claim with tools** (the audit itself has no test capability), and either
confirmed, corrected, or refuted each item before acting. Agreement below is
evidenced, not assumed.

---

## 1. Verdict on the audit, item by item

| # | Audit claim | Independent finding |
|---|---|---|
| H1 | `narrow_const` mis-types I32/U32 lanes → latent miscompile | **Code reading correct, severity overstated.** The helper returned `IrConst::I16(cv as i16)` for every non-8-bit lane. But all four call sites sit behind the `matches!(ty, I8|U8|I16|U16)` gate at `build_pack`, so no 32/64-bit lane ever reached it. Proven: instrumented `narrow_const` across all 740 regression + 51 benchmark + probe files → **zero** calls with a wide lane. So not a live miscompile — but a non-total helper is a trap; fixed defensively (below). |
| H2 | codegen gate `check_bb_slp_nested_ifconv.sh` orphaned | **Confirmed and far larger.** Zero references in `.github/` or `scripts/`. But the systemic finding: **84 of 126** `tests/regression/check_*.sh` are referenced by nothing. Root cause: `check_ci_gate_parity.py` only validates scripts `ci_local.sh` already names; a script wired into neither is invisible. |
| M1 | coverage evidence not bounded at frees | **Code reading correct, consequence UB-only.** `block_deref_keys` iterates with `_ => continue`, so a `Call` neither clears the map nor stops the walk. An observable miscompile in *conforming* C could not be constructed (aliasing across calls is already refused — verified). The hazard is `free`/`munmap`/SEH-style programs (LLVM's `CanBeFreed`). Fixed as hardening. |
| M2 | ~180 lines of duplicated demotion | Partially addressed (Fold enum consolidated, N3). The large structural extraction was **deliberately not done**: the prior attempt regressed (`clamp_i8x16` 27→192) and is recorded as a dead end; duplication here is localized and commented, and correctness outranks it. |
| M3 | sibling coverage misses the region tail | **Sound but unreachable.** Implemented region-union, then proved across the whole corpus **0** cases where region-tail coverage differs from head-only (instrumented). Two purpose-built inside-out cases also did not isolate it. Reverted rather than ship unverifiable generality. |
| N1 | doc says "no unsigned word intrinsic"; suggests `(U16,16)` arm | **Doc wrong, but the suggested fix was itself wrong.** `VecMinU16x16/VecMaxU16x16` exist (`vpminuw/vpmaxuw`), yet all three fold tables accepted only *signed* predicates, so no unsigned lane ever reached `packed_int_minmax` — the existing `U8` arms were also dead. Simply adding the arm (as T6 proposed) would be dead code. The real fix unlocks the whole unsigned family (below). |
| N2 | unexplained `depth + 2` | Confirmed anomalous vs `depth + 1` everywhere else; normalized. Measured byte-identical. |
| N3 | three `enum Fold` | Confirmed; hoisted to one file-level type. Pure refactor. |
| N4 | docs cite work-branch hashes / stale base / snapshot names | Confirmed for the S61 follow-up header; replaced with the landed `main` commit. Journal rebase hashes retained (accurate history). |

## 2. What was delivered (each measured, each regression-checked)

### 2a. Total `narrow_const` (audit H1, defensive)
`narrow_const(cv, ty)` now returns `Option<IrConst>`, exact over
I8/U8/I16/U16/I32/U32 and fail-closed otherwise. A wider lane that forgets
to register becomes a *rejected seed*, not a miscompile. Pinned by two
**non-vacuous** unit tests (verified to FAIL against the old behaviour by
mutation) plus an end-to-end wide-lane truncated-select regression test
(`bb_slp_i64_to_i32_select.c`).

### 2b. Unsigned min/max folds — the real perf win (audit N1, extended)
`demote_cmp_pred` maps unsigned lanes to `Ult/Ule/Ugt/Uge`, but every fold
table admitted only the signed relational predicates, so the one-instruction
integer min/max was unreachable for **every** unsigned clamp (the `U8` arms
were dead). The tables now admit the unsigned predicates and thread their
signedness into `packed_int_minmax`, which requires predicate/lane signedness
to agree (fail-closed on a same-width reinterpret mismatch) and gains the AVX2
`U16x16` `vpminuw/vpmaxuw` arm.

This was only reachable because of a second fix: **copy-transparent
`same_source`**. C promotion emits the re-read arm of `a[i] > k ? k : a[i]`
as a `Copy` of the load; `same_source` stripped only bit-identity casts, so
the fold degraded to cmp+blendv. Stripping `Copy` (a pure value move) unlocks
it.

Codegen (lccc −O2 −march=x86-64-v3, before → after):
`clamp_lo_u8x16` 20→8 (`vpmaxub`), `clamp_hi_u8x16` 20→8 (`vpminub`),
`clamp_u8x16` 33→21, `clamp_tmp_u8x16` 33→15, `clamp_hi_u16x16` 12→7
(`vpminuw`), `clamp_i16x8` 26→17, `clamp_tmp_i16x8` 22→13,
`clamp_both_i16x16` 14→12, `slp_probe clamp_u8x16` 33→21.
**Net −91 instructions, 9 wins, 0 regressions, 0 new branches.**

### 2c. Free-barrier bound on load-speculation coverage (audit M1)
`may_free_or_unmap` names the invalidators — direct call, indirect call,
inline asm (pure ops, GEPs, atomics, `memcpy` are deliberately not barriers:
none releases a mapping). `block_deref_keys` clears at each barrier (only
post-last-barrier derefs survive); `dominating_deref_keys` stops the
unique-predecessor walk at a block containing a barrier. Proven reachable by
a new `must_branch after_release` control (out-of-line opaque
`asm volatile("":::"memory")` barrier): branchless with the barrier removed
(mutant fails the gate), 1 branch with it present; the no-call control stays
branchless.

### 2d. CI enforcement (audit H2, and the class behind it)
- `check_bb_slp_nested_ifconv.sh` wired as a fast gate into `ci_local.sh`
  and a step in `ci.yml`.
- `check_ci_gate_parity.py` now also fails on any `tests/regression/check_*.sh`
  executed by nothing unless it is in `scripts/ci_gate_allowlist.txt`, and
  fails on any allowlist entry that is no longer an orphan — so the 83-orphan
  debt is enumerated, can only shrink, and a newly added but unwired gate
  fails CI immediately. Each entry carries a reason (env dependency,
  gate-script defect, or "not yet wired"). Several were failing locally at
  audit time and are flagged for triage rather than blindly wired.

### 2e. Small cleanups
N2 depth-cap normalization (byte-identical); N3 single `Fold` home
(byte-identical); N4 doc provenance resolves from `main`.

## 3. Validation

`cargo fmt` clean; `cargo clippy --all-targets --profile fastbuild --locked
-j1 -- -D warnings` 0 diagnostics; `run_regression_suite.sh` **PASS=741
FAIL=0 SKIP=8, AB-diff 0**; `check_bb_slp_nested_ifconv.sh` PASS; runtime
clean at −O1/−O2/−O3, default and explicit −march, and with `slp` disabled;
differential vs gcc clean. Unit tests mutation-checked.

## 4. Top remaining lever — vector regalloc spill at ≥3 live uses

**Diagnosed, not fixed this session (correctness is a hard constraint and the
allocator is 14 K lines of interacting paths).** On every cmp+blendv kernel a
vector value with three live uses is spilled to the stack despite ~16 XMM
registers being available.

Reproducer (`abs_i16x8`, −O2 −march=x86-64-v3): **16 insns + `subq $72,%rsp`
+ 2 reloads** vs gcc's 7. The post-`slp` IR is clean (11 instrs, no spill):
the loaded vector feeds `VecSubI16x8`, `VecCmpI16x8`, `VecBlendvI16x8`. Two
**redundant zero broadcasts** are emitted — `VecBroadcastI16x8(Const(I16(0)))`
for the compare and `VecBroadcastI16x8(Const(Zero))` for the negate — which
are syntactically *different* constants for the same value, so nothing CSEs
them and they occupy two registers, pressuring the load to spill.

Two candidate directions, both needing careful validation: (a) IR-level —
deduplicate identical zero splats (normalize `Const(Zero)` vs
`Const(I16(0))`), removing a live value without touching the allocator; (b)
allocator-level — the linear-scan spill heuristic spills with ample registers
(an LRU-tie steal is already noted at regalloc.rs:6975). Worth ~5 insns on
every cmp+blendv kernel.

## 5. Deferred / disagreed

- **M2 full extraction**: deferred (prior attempt regressed; duplication is
  localized). Fold enum consolidated instead.
- **M3 sibling-tail coverage**: reverted after proving it unreachable on the
  corpus; the audit's "sound but incomplete" was confirmed sound but not
  demonstrably exercised.
- **84 orphaned gate scripts**: 1 wired this session; the rest are enumerated
  in the allowlist with reasons. Fixing the 12 locally-failing ones (env
  deps, GAS version, gate-script `.size`-awk defects) is its own task.
