# Follow-up — 2026-09-05 (session 4) — RA-06 built and measured; five more miscompiles closed

Base: `ms178/lccc` main @ `5537286` (contains PRs #412, #413, #414).
Deliverable: `ms178-1.patch`, verified `APPLIES-CLEAN` on that commit.

---

## 1. Five live miscompiles found and fixed

Every one was reproduced on this base *before* being fixed, and every one is
silent — wrong values or wrong addresses, no diagnostic. Four of the five are
wrong-address bugs in the same two functions; the fifth deletes an observable
access.

| # | Defect | Measured on the unfixed compiler |
|---|--------|----------------------------------|
| 1 | `canonical_addr_key_impl` treated every widening cast as value-preserving, so **sext and zext of the same source shared an address key** | `signed char i = -1`: `d[i]` (→ `d-1`) and `d[(unsigned char) i]` (→ `d+255`) collapsed. `ext_diamond` returned 11 for the zext arm, want 22 |
| 2 | The same collision let `rewrite_covered_arm_loads` forward a pred load to an arm load **256 bytes away** | `ext_covered` → 11, want 22 |
| 3 | …and let `sink_conditional_stores` merge two stores to **different addresses** — a wrong-address WRITE | `ext_store`'s false arm wrote `d[-1]` instead of `d[255]` |
| 4 | The backend's SIB index **cast peel** walked `I32 → U32 → I64` down to the I32 root and then SIGN-extended it | `t[(unsigned char) c]` with `c = -1` addressed `t[-1]`: `peel_idx` returned 0, want 7255. That shape is every character/CRC/tolower table lookup |
| 5 | The covered-arm load rewrite matched `Load { .. }`, **blind to the volatile flag** | `volatile int g; if (g > 0) return g;` emitted ONE `g(%rip)` read; C11 5.1.2.3 and GCC require two |

Fixes, in the terms the code now states:

* **Cast in the address key.** A cast may be descended through only when the
  composite root→offset function it contributes is *recorded* in the key.
  Same-size integer casts are transparent; widening casts record the
  extension kind and both widths (`sx4>8` / `zx4>8`); truncating casts stop
  the walk; anything touching a floating-point type stops the walk, because
  `(long)(float) i` rounds for `|i| > 2^24` with no UB.
* **Shift in the address key.** A shift wraps at its own width, so
  `(long)(u << 1)` and `((long) u) << 1` differ for `u >= 2^31` on the
  unsigned form. The width joins the constant in the key.
* **Volatile.** Excluded from both the covering set and the rewrite
  candidates. A volatile access is an observable event that neither
  substitutes for another nor may be substituted by one.
* **Cast peel.** The peeled index is later extended by
  `ensure_sib_index_form` according to *its own* type, so the peel is sound
  only when that extension reproduces the chain: `from_ty` must really be the
  source value's type, and the step must be same-signedness or an
  unsigned→signed *widening* (a zero-extended value is non-negative in the
  wider type, so any later extension of it equals `zext` of the root).

Regression test: `tests/regression/ifconv_key_injectivity_and_volatile.c`,
green at `-O0/-O2/-O3/-Os` on x86-64 and i686 and matching GCC. Every index
pair used lives inside one allocation, so a wrong-address access reads or
writes a *defined* neighbour rather than faulting — which is precisely what
made these five silent.

---

## 2. RA-06: built, proven correct, measured negative, shipped off

`split_ranges::split_high_pressure_ranges` is decoupled spill-then-color in
the Braun & Hack sense (CGO 2009), as an IR pre-pass:

* per-block program-point pressure over the GPR-eligible values;
* Belady MIN at each over-subscribed point — evict the value whose **next use
  is farthest**;
* materialise the eviction as a store at the free point and a reload as a
  **fresh SSA name** before the next use, renaming from there on plus the
  successor phi operands from that block. The fresh name is what buys two
  locations for one logical value inside a backend whose assignment result is
  a single `value → location` map — the same reason LLVM's SplitKit works at
  MIR level.

Three bugs were found and fixed *during* development, each by measurement
rather than inspection:

1. **Index invalidation.** Applying splits with repeated `insert` calls
   shifted the planned indices of every later split in the block; one split
   ended up reloading before its own store. Rewritten as a single linear
   rebuild with all planning indices in original coordinates.
2. **Stores before phis.** A live-in value's store was placed at index 0,
   ahead of the block's phi prefix — invalid IR. Now clamped past it, and the
   split is dropped if that clamp shrinks the gap below the profitability
   floor.
3. **Vector values through a GPR slot.** `is_simple_gpr_type` is not
   sufficient: a 256-bit intrinsic result can carry a scalar-looking IR type,
   and storing one through an I64 slot truncates it to its low lane. Fixed
   with an explicit eligibility set that excludes intrinsic and inline-asm
   operands and results. The `vectorize_*` oracle tests caught this.

**Correctness: 633 PASS / 0 FAIL / 0 A/B diffs and torture-clean with the
pass forced on.**

**Profitability: negative, in every configuration.**

| budget / min-gap | kernel stkref | kernel insns |
|---|---|---|
| 12 / 4  | +119 | +121 |
| 13 / 12 | +87  | +98  |
| 14 / 20 | +87  | +98  |
| 16 / 30 | +57  | +58  |

Monotone toward zero, and **not one function in the corpus improves** — the
"best three" are three regressions. `arith_loop` goes 165 → 367 stack refs at
the aggressive setting.

This is a mechanism, not a tuning failure. Braun–Hack pays because its spill
phase *guarantees* MAXLIVE ≤ k at **every** point, so the colorer never spills
again and the store/reload traffic is the entire cost. An **intra-block**
splitter cannot provide that guarantee here: the dominant candidates are
loop-header phi values consumed in a *different* block, and renaming those
needs real cross-block SSA repair. Instrumented at `arith_loop`'s peak, 32 of
the live values are rejected for exactly that reason. Residual pressure stays
above k, the colorer demotes anyway, and the program pays for **both**.

So the pass ships **off by default** (`CCC_PRESSURE_SPLIT=1`), with the sweep
and the mechanism recorded in `engineering/DECISIONS.md` — the repo's
negative-results ledger — so the next attempt starts from the guarantee
instead of from the heuristic. Turning it on as-is would be a measured
regression, which the project's own rules forbid.

Four unit tests pin the fail-closed renaming invariants (outside non-phi
consumer, successor phi from this block, phi operand from *another*
predecessor, knob clamping).

**This also invalidates RA-28b's hypothesis** ("mode 6 loses only because of
lifetime demotion; splitting will flip its sign"): it cannot be tested with
this pass, because the pass does not remove the demotions it was meant to
remove.

---

## 3. Validation

| gate | result |
|------|--------|
| `run_regression_suite.sh` (default) | **633 PASS / 0 FAIL / 7 SKIP**, AB-diff 0 |
| `run_regression_suite.sh` (`CCC_PRESSURE_SPLIT=1`) | **633 PASS / 0 FAIL**, AB-diff 0 |
| `gcc.c-torture/execute` x86-64 `-O0..-Os` | 0 run-fail |
| `gcc.c-torture/execute` i686 `-O0..-Os` | 0 run-fail |
| `cargo test --lib` | 4 new RA-06 invariant tests |

---

## 4. The one honest to-do

**Cross-block SSA repair for the spill phase.** Everything else about RA-06 is
now in the tree and tested; the missing piece is the ability to rename a
value's consumers in other blocks (dominance frontiers, phi insertion), which
is what turns the pass from "partial spilling that pays twice" into the
MAXLIVE ≤ k guarantee that makes spill-then-color a win. The acceptance
number is unchanged: `arith_loop` 165 → ≤ 100 stack refs (GCC is at 92), with
`scripts/ra_ab_census.py` as the gate.
