# Follow-up: executing MachInst, and the critical edge that blocks the
# classifier

Session date: 2026-09-01 (continuation)
Base: `ms178/lccc` main @ `e836c0ab`
Snapshot: `S27`

---

## 1. Accomplished — MachInst verified by execution

The suite already had four layers: register-table integrity, `IrType`→`OpSize`
mapping, golden per-variant emission, and an assembler differential (4000
randomized instructions through GAS 2.47). All of them answer one question —
*is this a valid instruction?* None answers the harder one: **is it the
instruction we meant?**

AT&T order is where that gap bites. `subq %rax, %rbx` means `rbx -= rax`. An
emitter that swapped the operands of a non-commutative op would produce text
GAS accepts, a disassembler prints without complaint, and a fuzz corpus
assembles cleanly — while every program computes the wrong answer.

The only way to close it is to **run the instruction**. Each case is assembled
with the pinned GAS 2.47, linked into a harness, executed on six input pairs
(including negative and boundary values), and compared against semantics
computed independently in Rust:

| test | what only execution can catch |
|---|---|
| all six ALU ops | `Sub`/`Imul` operand order |
| all three shifts | `sar` vs `shr` differ **only on negative inputs** |
| 32-bit ALU | x86-64 zero-extends bits 32..63; the 64-bit spelling assembles fine and silently keeps the upper half |
| `movzx` vs `movsx` on `0xFF` | 255 versus −1 |

**Proven non-vacuous.** Deliberately swapping the `Alu` operand order makes the
execution test fail with a precise diagnosis —

```
Add(dst=100, src=7) computed 100 but must be 107
  (an AT&T operand-order or width bug)
```

— where the accept-only differential fails merely *incidentally*, because
swapped immediates become invalid destinations. A pure register-register swap
would slip through it entirely.

32 MachInst tests, all green against GAS 2.47.

---

## 2. Attempted and reverted: the `namechars` classifier

The worst kernel (0.44x) still emits an **eleven-branch chain per byte** where
GCC 16.2 vectorises and ICX uses a binary search. The diagnosis from the
previous session was correct as far as it went, and this session took it two
steps further before hitting a wall worth recording.

### 2.1 What was built

* **`hoist_merge`** — a normalization that hoists a small, pure,
  multi-predecessor "then" block into its immediate dominator, emptying it so
  the hit edges can be retargeted straight at the join. This *worked*: the join
  phi went from two incomings to **one per test**, which is precisely the shape
  `set_membership`'s matcher documents.
* **`set_membership` hit-value generalization** — its matcher required
  `Const(1)` on every hit edge, matching only the boolean spelling
  `int f(char) { return a || b || c; }`. The counting spelling
  `if (a || b || c) n++;` contributes the same *incremented counter* on every
  hit edge. Nothing in the mask logic depends on the value being 1, only on it
  being the same across hits.

Together these got the target function from 11 branches / 41 instructions to
10 / 38.

### 2.2 Why it was reverted

`bool_return_materialization` began mismatching GCC. The cause is structural,
not a coding slip:

> The last test block branches to the hit block **and** falls through to the
> join. After retargeting, that block has **two edges to the join carrying
> different values** — `v58` on the miss, `v54` on the hit. A phi cannot
> represent one predecessor twice with two values.

That is a **critical edge**, and it is not incidental: the last member of any
`a || b || c` chain always has exactly this shape, so the transform can never
fire on the pattern it was written for without splitting that edge first.

Reverted rather than shipped. A partial 11→10 improvement is not worth a
miscompile, and guarding the critical-edge case would have made the pass fire
on nothing.

### 2.3 What the next attempt should do

1. **Split the critical edge first.** Insert a block on the last member's
   miss edge so each predecessor reaches the join exactly once. `split_ranges`
   already has edge-splitting machinery worth reusing.
2. Then `hoist_merge` + the hit-value generalization apply cleanly.
3. **`range_fold` still will not fire.** Even with the phi normalized, the
   `&&` pairs remain branches, so the members parse as `Skip` (plain `Cmp`)
   rather than `Range`, and there is no contiguous run to mask. The chain is
   `if_convert` → `range_fold` → `set_membership`, and it is starved at the
   first link: `if_convert` does not flatten the `&&` pairs in this branchy
   context.

So the real prerequisite is **if-conversion of the pure SESE region** spanning
the test chain. Every block in it holds a single `Cmp` and ends in a
`CondBranch`; the whole region is side-effect free. Converting it to predicate
arithmetic (`reach[B] = OR over preds of reach[P] AND edge_cond`) yields
`Select`s, after which `range_fold` and `set_membership` do the rest
unmodified. That is the supreme solution and it is a bounded, well-defined
algorithm — it is simply larger than the remaining budget of this session, and
shipping half of it is what produced the miscompile above.

---

## 3. Validation

| Gate | Result |
|---|---|
| `cargo test --lib` | **1472 pass / 0 fail / 6 ignored** |
| `./scripts/run_regression_suite.sh` | **PASS=561 FAIL=0 SKIP=15**, AB-diff 0 |
| `ir_verify_sweep.py` | **0 violations** |
| MachInst accept + fuzz + **execution** | green against **GAS 2.47.20260726** |

---

## 4. To do next, in priority order

1. **If-convert the pure SESE test region** (§2.3). Unblocks `range_fold` and
   `set_membership` and turns the 11-branch classifier into a handful of
   branchless instructions. Split the critical edge first.
2. **Vectorisation breadth.** GCC's 149/243-instruction `scan`/`match_len` are
   SIMD; lccc's scalar `memchr` ties it here but will not on wider data.
3. **Encoding-level differential.** Execution proves semantics for the cases
   written; comparing emitted bytes against GAS's own encoding would cover the
   rest. `insndiff.py`/`encdiff.py` exist.
4. **Def-dominates-use in the IR verifier**; **linker oracle** (lld 23.1,
   mold 2.42, bfd 2.47). Both unchanged.

---

## 5. Notes for whoever picks this up

- **"Does it assemble" is a weak oracle.** Two MachInst bugs were found by the
  accept differential, but an operand-order bug is invisible to it. Execute the
  instruction; the diagnosis is also far better.
- **Do not ship a partial region transform.** The `hoist_merge` work was
  correct in isolation and still produced a miscompile, because the pattern it
  targets always contains a critical edge. Normalize the edge first or not at
  all.
- **Check the whole chain before optimizing a link in it.** The phi shape was
  fixed and the classifier still did not improve, because `range_fold` upstream
  had never fired either. Verifying the *next* pass actually fires would have
  shown that before any code was written.
