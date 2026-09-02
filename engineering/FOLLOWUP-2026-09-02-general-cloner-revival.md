# Follow-up 2026-09-02 — general CFG cloner revival + unroll arithmetic hardening

Supersedes the open question left by the `c72ff2c` review ("was the
general-cloner stride fix ever exercised?"). Snapshot `S10`, commit
`62e064f`, base `a726b5b` (latest main at the time of writing).

## 1. Audit verdict on the review claim

**Claim:** `c72ff2c` added IV-stride scaling to the general CFG cloner
(`iv_init + t * iv_step` per clone, `iv_init + trip * iv_step` post-loop),
which only the two-block cloner had; without it a stride-4 multi-block
loop would clone indices 0,1,2,… instead of 0,4,8,….  The reviewer also
noted the shipped tests exercise only single-block bodies.

**Verdict: correct on both counts — and understated.**  The scaling is
present and textually correct.  But the general cloner could not fire on
*any* standard loop at all:

* `inst_ok` (general cloner) rejects any header `Cmp` whose operand has
  "pointer origin" (`operand_has_pointer_origin`, added in PR #302).
* That walk failed **closed** at recursion depth > 8.  Every counted
  loop's IV is a cycle — `phi <- Add(phi, step)` in the latch — so the
  walk spun `phi -> Add -> phi -> …` and returned "pointer" at depth 9.
* The exit `Cmp` always compares the IV phi, so **every** loop died at
  that check.  Instrumented evidence: 20/20 general-cloner attempts on
  seven stride kernels rejected at the header-`Cmp` gate, printing e.g.
  `Cmp { op: Slt, lhs: Value(40), rhs: Const(I32(16)) }`.

So the `c72ff2c` scaling was dead code as shipped, and the guard's own
comment ("Numeric and FP nested loops … are unaffected") had been false
since the day it was written.  Timeline confirms the interaction:
guard predates the fix (`git log -S` → #302 vs `c72ff2c`).

## 2. Fix

* **`operand_has_pointer_origin`**: depth cap replaced by an iterative
  visited-set def-web walk.  Each distinct value expands once (linear),
  pointer producers (`Alloca/DynAlloca/GlobalAddr/LabelAddr/GEP`) are
  found at *any* depth, and numeric cycles terminate as non-pointer —
  the guard's documented intent.  Pointer-state loops (the #302
  motivation) are still rejected: the pointer node itself is reached
  through the web regardless of cycles.
* **`complete_unroll_trip`**: fully checked arithmetic.
  `limit - iv_init` wrapped for extreme pairs (limit = `i64::MAX`,
  init = -1); `-iv_step` wrapped for `iv_step = i64::MIN`.  Now
  `checked_sub` / `checked_abs` / overflow-free ceil
  (`(span-1)/step + 1`) / `checked_add`; any wrap ⇒ `None` (stay
  rolled).  Unsigned cmp ops keep their conservative i64 reading.
* **Post-loop IV, both cloners**: `init + trip*step` checked
  (`checked_mul`+`checked_add`) before substitution; e.g.
  init 0, limit `i64::MAX`, step 2^62 gives trip 2 but final 2^63 ⇒
  refuse to clone.  Per-clone constants (`t < trip`) are provably
  between init and final, so the single final-IV check bounds them all.

## 3. Evidence

| check | result |
|---|---|
| new regression `unroll_stride_general_cloner.c` (7 multi-block stride kernels: diamonds up/down, carried-phi diamond, nested, post-loop IV uses, residue trip) vs `gcc -O3 -march=x86-64-v3` | exact match `504 17 256 100 1157 42 53` |
| red/green: revert only the two scaling lines (cloner revived) | `7 17 21 10 1157 14 15` — 5/7 kernels miscompile ⇒ fix is live and load-bearing |
| header-`Cmp` rejections after visited-set fix | 20 → 0 on the same kernels |
| unit tests | 1543 pass / 0 fail (new: `unroll_trip_extremes_never_wrap`) |
| regression suite | 571 / 0 / 15 skip |
| benchmark outputs | 152 / 0 / 0 |
| `ir_verify_sweep.py` | 1142 configs, 0 violations |
| fresh-worktree verification | patch applies clean on `origin/main` (`a726b5b`), builds, 1543/0 units, 571/0/15 regression, oracle match |

## 4. Remaining backlog (unchanged ranking, minus this item)

1. Typed `Call` lowering in MachInst isel (522 silent rejects).
2. Vectorizer: global-array map-loop bailout (missed vec on `matmul`-ish
   kernels writing globals).
3. dot8 unroll+FMA end-to-end gap vs gcc — re-measure on godbolt now
   that the general cloner actually fires (nested numeric loops were its
   stated target; expect change).
4. FCmp scratch xmm0/xmm1 staging (PhysReg 18/19 idle).
5. Memcpy(92)/Store-other(79) typed lowering.
6. Loop-gate `loop_insts ≤ 32` raise experiment.
7. Linker-oracle script for LTO/whole-program comparisons.
