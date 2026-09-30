# FOLLOWUP 2026-09-30 — the affine exit-compare win, and the exact blocker

**Status:** diagnosed, prototyped, soundness-argued, **deliberately not landed**.
**Priority:** high (systematic, per-iteration, every loop of one shape).

## The gap, in hard numbers

For the canonical countdown shape `for (int i = 0; i + C < N; i++)`, lccc
emits an `lea` to materialise `i + C` **on every iteration** and compares the
result against `N`. GCC strength-reduces the same test to a comparison of the
IV against `N - C`. Measured on a minimal loop (5 instructions per iteration,
deterministic `Ir` from Callgrind):

| | loop body | insn/iter |
| --- | --- | --- |
| **lccc -O2** | `movzbl (%rdi,%rdx),%eax; addq %rax,%rsi; addq $1,%rdx;` **`leaq 4(%rdx),%r10;`** `cmpq $2048,%r10; jl` | **5** |
| **gcc -O2** | `movzbl (%rax),%ecx; addq $1,%rax; addq %rcx,%rdx; cmpq %rsi,%rax; jne` | **4** |

The `leaq` is 1 of 5 instructions in the hottest loop of the program, on
**every** loop of this shape. It is not shape-specific: the same `leaq C(%p)`
+ `cmpq` pair is emitted for `for (const unsigned char *p = v; p + C < v + N; p++)`
as well, so this is systematic, not a one-off.

## The fix (prototyped, then reverted)

Fold the offset out of the comparison. In `loop_rotate`, the latch test is a
clone of the header's guard, and that guard is `Cmp(Add(i, C), N)`:

```text
  addq  $1, %rdx             ; i += 1
  cmpq  $2044, %rdx          ; i < N - C     <-- was: leaq 4(%rdx),%r10; cmpq $2048,%r10
  jl    .LBB2
```

**Soundness.** `Slt(iv + c, k) == Slt(iv, k - c)` whenever `iv + c` does not
overflow. Here `c` is a small positive literal and `iv` counts toward a bound,
so an overflow of `iv + c` is a signed overflow in the *source* expression
`i + C` — undefined behaviour, hence outside the set of executions the C
program admits. This is the same argument `iv_widen` already uses for its
derived `Add`/`Sub` chain. **The rewrite must be restricted to signed
comparisons**: for `ult(iv + c, k)` the wrap is *defined* and the identity
genuinely fails. The bound is computed with `checked_sub` and the rewrite is
skipped when `k - c` would overflow.

Implemented as a post-clone canonicalisation in `loop_rotate` (the pass that
*mints* the `Add`), operating only on freshly cloned temporaries whose sole
user is the `Cmp` being rewritten. Verified to fire: `cmpq $2044, %rdx`.

## Why it was NOT landed

`loop_rotate` is **opt-in** (`CCC_LOOP_ROTATE=1`). `src/passes/loop_rotate.rs:114`
records that a v16 attempt to default-enable it was **reverted**. So at
default `-O2` the fold is unreachable — shipping it would be dead code, and
the deliverable explicitly rejects shipping unreachable work.

Enabling the pass is blocked by a **backend** gap, and this is the part worth
recording. With rotation on, the boolean is materialised and the compare-branch
fusion refuses:

```asm
    cmpq $2044, %rdx
    setl %r8b
    movzbl %r8b, %r8d
    testb %r8b, %r8b
jne .LBB1
```

5 instructions where `cmpq $2044, %rdx; jl .LBB1` is 2. Rotation therefore
*loses* 2 instructions per iteration unless the fusion fires — and it does not
fire, because of a liveness-precision bug, not a pattern-matching gap:

* `compare_branch::fuse_compare_and_branch` **does** recognise this shape. Its
  own comment names `setl %r9b` / `movzbl %r9b, %r9d` as a validated shape,
  `parse_self_test` accepts `testb %r8b, %r8b`, and
  `parse_bool_extension` returns `Ok(8)`.
* The **same** sequence with a **forward** branch fuses cleanly in the unit
  harness. With the **backward** branch (a real loop) it does not.
* `CCC_DEBUG_CMP_FUSE=1` names the exact refusal:

  ```
  [CMPFUSE] refusing non-legacy fusion
             (setcc_fam=8 setcc_live_after=Some(true) relay_live_after=Some(Some(true)))
  ```

  `FileLiveness::live_after` returns "live" for `%r8b` at the `jne`, even
  though `%r8b` is defined by `setl` immediately above the branch and is
  therefore dead on every path out of the block, including the back edge.

**The gate is fail-closed and correct to be** — treating unknown liveness as
live is the right default. The defect is that the analysis *cannot prove*
liveness across a backward edge here, so it is permanently pessimistic for
every rotated loop. Fixing that is a precision improvement in
`FileLiveness`, and because every consumer of it gates fail-closed, a precision
improvement can only turn refusals into fusions — never the reverse.

**Minimal reproduction** (verbatim lccc output, backward branch):

```asm
.LBB1:
    movzbl (%rdi, %rdx), %eax
    addq %rax, %rsi
    addq $1, %rdx
    cmpq $2044, %rdx
    setl %r8b
    movzbl %r8b, %r8d
    testb %r8b, %r8b
jne .LBB1
```

`%r8b` is live-in to `.LBB1` only if the fixpoint is seeded wrong: the loop
header's live-in is `{rdi, rdx, rsi}`.

## Impact on default builds today

**None.** A default `-O2` compile of the same loop emits **zero** `setcc`-in-a-loop
shapes, so this gap costs nothing until rotation is default-enabled. That is
why it is a follow-up and not a regression.

## Order of work for whoever picks this up

1. Fix `FileLiveness` precision across backward edges; prove `%r8b` dead at
   the `jne` in the reproduction above.
2. Confirm the fusion fires end-to-end (not just in the peephole harness) —
   a unit test that passes while the real pipeline still refuses is the exact
   failure mode that produced the `incapable` class in this same series.
3. Land the affine exit-compare fold in `loop_rotate`.
4. Re-measure with Callgrind across the corpus; only then consider flipping
   `CCC_LOOP_ROTATE` to default. Do not flip it on a wall-clock hunch — the
   `setcc` regression above is invisible in aggregate cycle counts.

## Re-verified on the merged tree (2026-09-30, after PR #695)

The numbers above were measured on the pre-merge branch. Re-measured on the
merged main (`ec08e6a6`) plus the audit fixes, to confirm nothing in the
merged state changed the picture. It did not.

`unsigned long f(const unsigned char *v, int n) { for (i=0;i<n;i++) s+=v[0]; }`

| build | loop body | insn/iter |
| --- | --- | --- |
| `-O2` (default) | `movzbl (%rdi),%r9d; addq %r9,%rdx; cmpl %esi,%r8d; jl .LBB2` | **4** |
| `-O2 CCC_LOOP_ROTATE=1` | `movzbl (%rdi),%edx; addq %rdx,%r8; setl %dil; movzbl %dil,%edi; testb %dil,%dil; jne .LBB3` | **6** |

`CCC_DEBUG_CMP_FUSE=1` still names the same refusal:

```
[CMPFUSE] refusing non-legacy fusion (setcc_fam=7 setcc_live_after=Some(true)
                                      relay_live_after=Some(Some(true)))
```

So rotation is still a 2-instruction-per-iteration LOSS on this shape, which is
why it stays opt-in and why the fusion is the prerequisite rather than a
nice-to-have. Note `setcc_fam=7` (`%dil`): the earlier note's `%r8b` is the
same defect at a different register allocation, so the reproduction is
register-agnostic.

### Sharper root cause: it is the `ret_live` seed, not the fixpoint

The note above says the fixpoint "cannot prove liveness across a backward
edge" and suggests the loop header's live-in is seeded wrong. Having read
`passes/liveness.rs`, the mechanism is more specific and more fixable than
that:

The backward dataflow itself is correct — `inn = eff.reads | (out & !eff.writes)`
iterated to a fixpoint over reverse line order, converging in the round bound.
The pessimism enters through the **exit seed**, at

```rust
if succs[rel].is_empty() {
    out |= ret_live; // fell off the end: be conservative
}
```

`ret_live` is derived from return-value classification markers
(`ret_rdx_marker`, `ret_rax_marker`) and, when the marker is absent or
illegible, deliberately keeps the conservative bit. In the rotation shape the
loop-exit edge falls off the end of the analysed window, so the guard block's
live-out takes that conservative value, `r8`/`dil` is live-in at the `jne`,
and the fusion refuses — even though the register is defined by `setcc` two
instructions earlier and is dead on every path out of the block, the back edge
included.

So the fix is to make the exit seed **precise for a function that ends in a
real `ret` with a known return classification**, not to change the dataflow.
Two properties make that safe to do as its own change:

* every consumer of this analysis gates **fail-closed**, so a precision
  improvement can only turn refusals into transformations, never the reverse;
* `ret_rax_marker` already demonstrates the shape of the change — it drops the
  RAX bit when the signature says the function is pure-SSE/x87-returning.

The measurement that must gate it: every consumer's own gate, plus a
corpus-wide Callgrind comparison, because "more fusions" is not automatically
"better" and a mis-fused compare-branch is a miscompile.
