# Red-team audit of upstream PR #363 + rebase of PR #358/#359 fixes onto it

Context: upstream merged PR #363 (`2c027cb1` → `e2d5ef66`) on top of
`da964ed4`, the base of this audit branch (`wip/audit358-359`). #363 is the
*next upstream work touching the same code area* — it fixes the same
wide-immediate-store bug as our `b10ef6c0`, plus adjacent FP-peephole and
typed-lowering work. This branch was rebased onto `e2d5ef66` (all three prior
commits replayed and reconciled) and #363 itself was red-teamed.

## What #363 contains (summary)

1. **Wide-immediate memory relay honors operand width** (`machinst_emit.rs`).
   Independent fix for the same VLA over-wide-store miscompile our `b10ef6c0`
   fixes, using a different technique: relay through `%rax` at the operand's
   own width (`movabsq $imm,%rax` + `movl %eax,mem`) instead of a single
   direct sized move.
2. **`is_pure_xmm_overwrite` merge-write rule** (`memory_fold.rs`). A reg-reg
   scalar `movsd`/`movss` preserves the destination's upper bits (SDM merge),
   so it is not a full overwrite; only memory-source scalar forms define the
   whole register. Real, previously-unfound bug in the same PR #359 machinery
   we audited.
3. **Hoist quality fixes**: dedupe same-line double uses; `movss`
   materialization for single-precision-only sites.
4. **Float-constant stores take immediate forms** (isel/emit typed path).
5. **Indirect calls join the typed contract**; **wide-imm call arguments**
   stage via movabsq.

## Verdicts

### 1. Wide-immediate relay fix — CORRECT but not optimal; ours is strictly better

The two fixes agree on the width contract (a store whose immediate is outside
the imm32 window must still write `size` bytes) and both close the VLA
miscompile. Differences, measured on the exact reproducer
(`tests/bugs/o2_vla_fill.c`, the unrolled `b[]` 4× `u32` fill of 3041712678u+i):

| | #363 sized relay | this branch (b10ef6c0 + #363) |
|---|---|---|
| `b[]` fill instructions | 8 (`movabsq`+`movl` ×4) | **4** (`movl $imm` ×4) |
| `%rax` clobbers | 4 | **0** |
| register-dest narrow | `movabsq` (10 B) | `movl` (5 B, zero-extends) |
| rationale | relay is a 2-instruction idiom | mov{b,w,l} imm field is a RAW {8,16,32}-bit value; the truncated constant stores directly |

The direct form is sound because the immediate field of `mov` (unlike the
ALU/compare imm32 forms) is not sign-extended into the store — the memory
destination takes exactly `size` bytes of the raw field, and a narrow register
destination takes the sized `movl` (zero-extending) / `movb`/`movw` forms.
Upstream's own `vla_narrow_const_store_family.c` passes with our path active;
their regression is a compatible second gate. Both are kept in the merged tree
(narrow fast path first; the sized relay remains the S64>i32 path and a
defense-in-depth net), and the emitter comment now records both.

### 2. Merge-write rule — AGREE, adopt (it is in the base now)

The counterexample is airtight: a deleted memory-source `movsd` zeroed bits
[127:64]; a later reg-reg `vmovsd` merges (preserves) those zeroed bits from
the *destination*, and a packed `vmovapd` then observes them. Our control-flow
barrier (`is_cf_transfer`, 9236df3d) and their dataflow rule (merge-writes are
not kills) are orthogonal necessary conditions for the same "the loaded value
is dead" proof; both are required and both are now in force. Six tests pin the
accept side (mem-source scalar, straight-line reuse) and the reject sides
(reg-reg merge, rmw, store, partial write, branch-skipped kill).

### 3. Hoist dedupe + movss width — AGREE (quality fixes)

The same-line double-use threshold bug is exactly the in-place-rmw case our
own audit flagged; the adjacent-dedup fix is minimal and correct (sites are
pushed in ascending line order). The `movss` materialization reads the op
width, which is the correct invariant (an op reads only its own width from the
pool address).

### 4. Float-constant immediate stores — mostly right; ONE defect found

Immediate forms are a genuine improvement (no rodata entry, no xmm load, no
partial-register dependency) and the `as i32` half-spelling correctly stays
inside the imm32 window. **Defect (fixed in this branch, `5ef2ea30`):** an
F64/D64 constant that needs the two-`movl`-half split was applied to C
`volatile` stores. A volatile store is one abstract access (C11 5.1.2.3); two
4-byte stores expose a torn state to signal handlers/concurrent readers that
the abstract machine forbids — gcc/clang/icc emit a single pool+movsd for a
volatile wide double. The typed lowering now refuses the split when the Store
is `volatile` (single-access immediate forms for F32 and for fitting F64 bit
patterns remain allowed). Demonstrated before/after on
`volatile double t; t = 1.5e300;` (was `movl`+`movl`, now one `movsd`), pinned
by 3 isel unit tests + `tests/regression/volatile_const_stores.c`.

### 5. Indirect calls / wide call args — out of the audited area; spot-checked

Mechanism (single atomic CallTyped, r10/r11 staging with topological
reader-before-writer protection) is consistent with the architecture this
branch audited earlier; left for the area owner. No defect found in the
read-through.

## A/B isolation (how the comparison above was measured)

`b10ef6c0`'s direct-immediate block was temporarily removed from the merged
tree (upstream relay alone active), `o2_vla_fill.c` compiled, and the `b[]`
fill counted: 8 instructions with 4 `%rax` clobbers. Restoring the block: 4
single `movl`s. The final tree keeps the fast path and passes every oracle
(`vla_largeconst_u32_fill.c`, `vla_narrow_const_store_family.c`,
`o2_vla_fill.c`) at -O2/-O3.

## Combined branch state after the rebase

```
e2d5ef66 (upstream main; merged PR #363)
  f016149e machinst: fix narrow stores of large immediates (b10ef6c0 replayed)
  9236df3d peephole: close control-flow gap in the FP fold liveness refinement
  f766a4da docs: resolve VLA store-width BUG doc; PR #359 red-team verdict
  5ef2ea30 isel: never split a volatile F64/D64 const store into two movl halves
```
