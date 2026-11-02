# LOOP-PREHEADER-1 — dedicated loop preheaders, and what they are worth

**Status:** shipped (`src/passes/loop_preheader.rs`), **profitability-gated**,
zero cost in the default configuration. **Baseline:** `800b439f` + MINMAX-1.

---

## 1. The defect: LICM could never hoist a derived-pointer load

LICM refuses to hoist any load whose loop preheader is not *dedicated*:

```rust
// src/passes/licm.rs
let preheader_is_dedicated = match &func.blocks[preheader].terminator {
    Terminator::Branch(l) => *l == func.blocks[header].label,
    _ => false,
};
```

That gate is **correct and load-bearing** — the comment above it carries the
miscompile it prevents (SQLite 3.53.4 `jsonCacheSearch`: the `if (p == 0)
return 0;` block is the loop's unique outside predecessor, so it *is* the
preheader, and hoisting `p->nUsed` into it dereferences NULL whenever the
cache is empty).

**The defect was that LCCC never *creates* a dedicated preheader.** The
frontend lowers a counted loop as

```text
  entry:  t = 0; i = 0; cmp; CondBranch(body, exit)   <-- preheader, TWO succs
  body:   ...; i++; cmp; CondBranch(body, exit)
  exit:
```

and a preheader with two successors is by definition not dedicated. So the
gate refused essentially every invariant load through a parameter, a call
result or a GEP chain — the loads that can fault, which is to say the ones
worth hoisting:

```c
int f0(const int *c, int n) {
    int t = 0;
    for (int i = 0; i < n; i++) t += c[0];   // `addl (%rdi), %edx`, every iteration
    return t;
}
```

Adding `CCC_DEBUG_LICM=1` (shipped with this change) shows the refusal
directly:

```text
[LICM] load v14 in block 2 not hoisted: its block does not dominate every
       loop block (block is not the header, so the load is not must-execute)
```

Note that the *reported* reason is not the preheader — it is the must-execute
rule, because in the guard-at-top shape the load lives in the body, and LICM
requires the hoisted load's block to dominate **every** loop block (only the
header does). That rule is also correct: with `n == 0` the body never runs, so
hoisting `*c` into the preheader would fault on a pointer the program never
dereferences. **Both refusals are right. The loop shape is what is wrong.**

## 2. The fix

`src/passes/loop_preheader.rs` splices an empty block onto the single outside
edge:

```text
  entry:  t = 0; i = 0; cmp; CondBranch(pre, exit)
  pre:    Branch(body)                                <-- dedicated
  body:   ...; i++; cmp; CondBranch(body, exit)
  exit:
```

**Soundness.** `pre` is reached exactly when the loop is entered, and the
header is part of the loop body, so reaching `pre` implies the body runs at
least once. LICM additionally requires the hoisted load's block to dominate
every loop block, so the load was already must-execute. Nothing therefore
executes on a path that never entered the loop, and the SQLite shape is safe
because the `p == 0` branch now reaches `exit` *before* `pre` exists.

**Scope.** Only loops with a *single* outside predecessor are rewritten —
several would need two phi entries for one incoming block, which SSA does not
allow, and LICM skips those loops anyway. An `IndirectBranch` into the header
cannot be routed through a new block (the computed address is the header's), so
those loops are left alone.

## 3. Why it is profitability-gated (the interesting part)

The first version inserted a preheader for every eligible loop. Measured over
the 51 benchmark programs at `-O3 -march=x86-64-v3`:

| configuration | static insns | preheaders inserted |
|---|---|---|
| pass off | 8088 | 0 |
| pass on, **ungated** | 8095 | 27 |
| pass on, **gated** | **8088** | **0** |

The ungated pass was **+7 instructions**: it paid for 27 blocks that unlocked
nothing, because in the guard-at-top shape the header is the guard block and
holds no loads, so LICM's must-execute rule refused every one of them anyway.

The shipped version therefore inserts a preheader only when it can actually
unlock a hoist: the loop must contain a non-volatile load, in a block that
satisfies LICM's must-execute rule, whose pointer is **not** an alloca and
**not** a `GlobalAddr`-derived value (those two classes are already hoistable
without a dedicated preheader). It mirrors LICM's rule instead of hard-coding
"the header", so it stays correct if that rule is ever relaxed, and it is a
cheap syntactic pre-filter rather than a second copy of the alias analysis.

Result in the default configuration: **0 insertions, byte-identical output,
zero risk.** The pass is infrastructure that costs nothing until something
makes it pay.

## 4. What it unlocks, and what still blocks it

With `CCC_LOOP_ROTATE=1` (which makes the body the header, and therefore
must-execute), the pass fires 18 times and the hoist is verifiable:

```c
int f1(const int *c, const int *a, int n) {
    int t = 0;
    for (int i = 0; i < n; i++) t += c[0] * a[i];
    return t;
}
```

* rotation, **no** preheader — the invariant load stays in the loop:

  ```asm
  .LBB2:
      movl (%rdi), %r10d          <-- c[0], reloaded every iteration
      imull (%rsi), %r10d
      ...
  ```

* rotation **+** preheader — it moves to the preheader:

  ```asm
  .LBB2:                          <-- the inserted dedicated preheader
      movl (%rdi), %r11d
      ...
  .LBB3:
      movl (%rsi), %edx           <-- only the genuinely varying load remains
      imull %r11d, %edx
      ...
  ```

The clearest single win is `fir_filter`: the 8-tap FIR loop reloads `c[0..7]`
from memory every iteration. With rotation the hoist lands:

| FIR hot loop | instructions |
|---|---|
| baseline | **56** |
| rotation + preheader | **38** (**−32 %**) |

**Rotation is not enabled by default, and this change does not enable it.**
Corpus-wide it is currently a net loss, and the reason is a separate defect in
`loop_rotate`: the rotated latch materialises the loop condition as an `i1`
value instead of keeping a comparison, so the backend emits

```asm
    setl %bl
    movzbl %bl, %ebx
    testb %bl, %bl
    jne .LBB3
```

— three instructions where one conditional jump belongs. Rotation measured
**+209 static instructions (+2.6 %)** across the corpus, which swamps the
hoisting win everywhere except the loops dominated by invariant loads. Fixing
that condition lowering is the follow-up that makes both rotation and this
pass pay; it is recorded as LOOP-PREHEADER-2 and is deliberately **not**
bundled here, because enabling a pass that is a measured net loss would be
the wrong trade.

## 4a. A pre-existing `loop_rotate` bug this turned up

Running the corpus under the SSA verifier (`CCC_VALIDATE_SSA=1`) with rotation
forced on aborts on two programs:

```text
SSA PHI-ARITY VIOLATION after phase 'after iter=0 loop_rotate': function
'main' block 18 φ incoming pred labels {23} violates the reachable-pred
contract (missing [39], not-a-CFG-pred [])          -- fir_filter.c

... function 'main' block 20 φ incoming pred labels {25} violates the
reachable-pred contract (missing [26], not-a-CFG-pred [])  -- moving_stats.c
```

This is **not** caused by this change, on two independent grounds:

1. It is attributed to phase `loop_rotate`, which runs at Phase 2b — long
   before `loop_preheader` (Phase 5-pre) executes at all.
2. It reproduces identically with `CCC_DISABLE_PASSES=loop_preheader`, i.e. with this
   pass fully disabled. With rotation off (this pass enabled), both programs
   verify clean.

So `loop_rotate` leaves phi nodes whose incoming-label set does not match the
rewritten CFG. That is a real latent miscompile in a pass that is currently
opt-in — which is very likely *why* it is opt-in — and it is worth fixing
before rotation is ever enabled by default. Recorded with LOOP-PREHEADER-2.

## 5. Diagnostics shipped

Both refusals were invisible before — "why is this invariant load still in the
loop?" cost an instrumented rebuild per hypothesis.

* `CCC_DEBUG_LICM=1` — prints the specific cause for every declined load
  (volatile, not must-execute, preheader not dedicated, alias engine off,
  calls in loop, alias info incomplete, no linear form).
* `CCC_DEBUG_LOOP_PREHEADER=1` — prints each loop's header, its preheader,
  whether it is dedicated, and whether a block was inserted.

## 6. Tests

* `src/passes/loop_preheader.rs` — 3 unit tests: the dedicated-shape
  predicate; edge retargeting across `Branch`/`CondBranch`/`Switch` (including
  that non-matching arms and non-matching switch cases are untouched); and
  refusal cases (`IndirectBranch` is never rerouted, and a non-matching target
  is reported as not rewritten). `cargo test --lib loop_preheader`: 4 pass.
* The transformation itself is covered by the corpus measurements above, which
  are the property that matters — a preheader insertion that is structurally
  right but hoists nothing is invisible to a Rust-side test.
