# 2026-08-21 session 35 — GlobalAddr CSE must run before GVN

**Base:** `ms178/lccc` `00321224` (PR #158 merge).

## What the merge AI caught, and why the first fix was not enough

PR #158 declared `pub(crate) mod global_addr_cse` and wired `global_addr_cse::run`
after SROA / post-structural-inline. A sanity check on the served patch was
right that an *unwired* module is a dead pass.

The follow-up that only added that late call (and required
`[GADDR_CSE] fn=fannkuch_like merged>=6`) still failed empirically:

```
[GADDR_CSE] fn=fannkuch_like merged=0
```

`CCC_DUMP_EACH_PASS` on the fannkuch-like repro shows why:

* **lowering:** 9 `GlobalAddr` (3 symbols × 3 call sites) in the loop body.
* **pre-loop after inliner:** 3 `GlobalAddr` + 6 `Copy` of those values.
* **post-structural-inline (late CSE):** already unique per symbol.

The *first* GVN, inside `run_inline_phase` after canonicalize-simplify, CSEs
`GlobalAddr` into same-block Copies. DCE then drops the dead address insts.
A late-only class-aware pass never sees duplicates, so `merged=0` is honest
and a `merged>=6` gate against that wiring can never pass.

Lea-count gates are also weak: RA already emits one `leaq perm(%rip)` per
symbol even with a dead pass.

## The actual fix

1. **Run class-aware CSE before the first GVN** (after canonicalize-simplify
   in `run_inline_phase`). That is the IR the pass was written for.
2. **Keep the late run** after post-structural-inline + DCE: inlining can
   clone callers and re-duplicate addresses. A third, idempotent run after
   unroll/vectorize on iter 0 covers cloned loop bodies.
3. **Split GVN's `ExprKey::GlobalAddr` on `must_mat`.** Otherwise a later GVN
   re-merges a RIP-foldable use with a call-arg use of the same symbol and
   undoes RA-01 (`window(%rip)`). Kill-switch of `gaddrcse` still gets
   same-class GVN Copies; mixed classes stay split.
4. **Always print `[GADDR_CSE] fn=… merged=N`** under `CCC_DEBUG_GADDR_CSE`,
   including `merged=0` and `CCC_NO_GADDR_CSE`. The gate takes the **max**
   across runs.
5. **Mixed-class assembly gate:** `window[i]` + `sink(window)` in one
   function must still address `window(%rip)`.
6. **Entry-block hoist** (session 35 follow-up on `8fb1573b`): if class C of
   symbol S has no entry materialization, insert one after the Alloca/ParamRef
   prefix and rewrite every later same-class GlobalAddr onto it. Cross-block
   duplicates (two loop bodies that never dominate each other) and a *single*
   loop-body address now share one dominating SSA web. GNU alias chains are
   canonicalized like GVN. Gate: `merged>=9` on fannkuch_like (9 loop-body
   deletes + 3 entry inserts); same-block-only CSE without the hoist reports 6.

## What we did not do

* Did not disable GVN GlobalAddr CSE entirely (same-class Copies remain a
  fallback when `gaddrcse` is killed).
* Did not lower the merge threshold to paper over a dead/late pass.
* Did not take further levkropp AArch64 commits; still treat them as broken
  until proven on this RA.

## Validation

* `cargo test --lib --profile fastbuild` (incl. `global_addr_cse` + new GVN
  mixed-class test)
* `tests/regression/check_global_addr_cse.sh`
* `tests/regression/global_addr_cse_runtime.c`
* ARM `check_arm_fp_stack_reload.sh` / `check_arm_direct_select.sh` if time
