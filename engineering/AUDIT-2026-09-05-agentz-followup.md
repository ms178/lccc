# Red-team audit — Agent Z follow-up patch (`ms178-1-AgentZ.patch`)

Auditor: this session. Base for the audit: `ms178/lccc` main @ `1883599`
(which already contains PR #412, the five miscompile fixes + position-relative
RA cost model from the previous session).

**Verdict: ADOPT the engineering, REJECT the test-harness changes.**
Four of the six commits are sound and are genuine improvements over what is
upstream. One commit is redundant. One commit degrades the test suite and must
not be taken. Three residual soundness holes in the adopted work are closed
here.

---

## 1. Summary table

| # | Commit | Verdict | Rationale |
|---|--------|---------|-----------|
| 1/6 | `canonical_addr_key_impl` Shl-scale injectivity | **ADOPT** | Fixes a real address-aliasing miscompile |
| 1/6 | if-conversion speculation rule replacement | **ADOPT + harden** | Replaces an unsound gate with the correct one; 2 residual holes closed |
| 1/6 | `fp_copy_web_groups` FP copy-web coalescing | **ADOPT + harden** | Good idea, was unsound as first written; 1 residual hole closed |
| 1/6 | `vex_promote` BFS liveness, direct-emit, xmm2 | **ADOPT** | Behaviour-preserving; validated |
| 2/6 | phi-coalesce const-binop peel gating | **ADOPT** | Narrows an over-broad transform |
| 3/6 | `fp_copy_web_groups` interference validation | **ADOPT + harden** | Agent Z self-corrected 1/6; still fail-open |
| 4/6 | store-source home arms | **ADOPT** | Restores arms lost in their own 3-way merge |
| 5/6 | `VecMaskedAddI32x8` width | **REDUNDANT** | Already upstream (PR #412, RC-4) |
| 5/6 | 4 × `LCCC_NO_COMPARE=1` `.txt` sidecars | **REJECT** | Inert files; dead weight in the patch |
| 5/6 | `vectorize_map_expr_tree.flags` rewrite | **REJECT** | Destroys the test's purpose |
| 6/6 | docs | ADOPT (rewritten) | |

---

## 2. What is genuinely good

### 2.1 The address-key injectivity fix is a real miscompile fix

`canonical_addr_key_impl` walked through `Shl(v, k)` and **discarded `k`**:

```rust
| Some(Instruction::BinOp { op: IrBinOp::Shl, lhs: Operand::Value(v), .. }) => *v,
```

So `d[i]` (offset `i<<2`) and `d[2*i]` (offset `i<<3`) produced the **same key**.
Since the key is the identity used by `rewrite_covered_arm_loads` and
`sink_conditional_stores`, that is not a missed optimisation — it is a
wrong-address rewrite and a merge of two stores to different addresses.
Recording the shift constant in the key is the right fix. Clever and correct.

### 2.2 The speculation rule replacement is the single best change in the patch

The old gate said: speculate a load if it sits in a natural loop, its base is
loop-invariant, and its offset is derived from the loop's IV. That is a
*shape* heuristic dressed up as a safety argument, and it is unsound. For

```c
d[i] = a[i] > t ? 1.0f : c[i];
```

`c[i]` satisfies every clause, so the old code hoisted a load of `c` on
iterations where the source program never touches `c` at all — a fault the
program cannot produce (short/invalid `c` with an always-true condition).

Agent Z replaced it with the correct, target-independent rule: a load may be
speculated iff its address is dereferenced on **every** path pred→merge
(pred's own derefs, or the sibling arm's derefs for a diamond). This is
exactly LLVM's `isSafeToSpeculativelyExecute` +
`isDereferenceableAndAlignedPointer` discipline, and it matches what GCC
actually does (it masks the uncovered load with `vmaskmovps` instead of
speculating it). Strictly better than upstream, and better reasoned.

The claim that no memory operation is reordered within a path checks out:
`arm_is_speculatable` admits only loads and pure whitelisted ops, so arms
contain no stores, and `apply_diamond` appends arm instructions to the pred.

### 2.3 `fp_copy_web_groups` is a good idea

Coalescing copy-connected scalar-FP webs so a reduction's combine→carry
handoff stops bouncing through a stack slot is a real win and the right level
to fix it at. It is the FP analogue of the GPR copy-group coalescing that
already exists.

---

## 3. What was wrong, and what is now fixed

### 3.1 `fp_copy_web_groups` shipped unsound in 1/6

As written in commit 1/6 the function union-finds **any** copy-connected FP
values and hands the whole web one register, with **no interference test**.
A copy edge implies value equality *at the copy*, not over the members' live
ranges. The counter-example is ordinary:

```
%c = Copy(%r)
%n = Add(%c, %term)      // %n is in the web via the latch copy
%m = Mul(%c, %other)     // %c is STILL LIVE here
```

One register for `%c` and `%n` means `%n`'s definition clobbers the value
`%m` reads. Agent Z found this themselves and added interference validation in
commit 3/6 — which is the right fix and is why the series is adoptable — but
it means commit 1/6 in isolation is a miscompile, and any measurement taken
between 1/6 and 3/6 is untrustworthy.

### 3.2 Residual hole A — the interference check fails **open** *(fixed here)*

```rust
let Some(si) = seg_of.get(&members[i]) else { continue; };   // 3/6
```

A member with no recorded segment is skipped, i.e. "no data" is read as "no
interference". That is precisely backwards for a soundness gate: the members
this validation cannot see are the ones most likely to be wrong. Changed to
`return false` (drop the group), so a web only merges on positive evidence.
The fallback is the pre-existing always-correct behaviour: separate homes and
real copies.

### 3.3 Residual hole B — speculation coverage ignored access **extent** *(fixed here)*

The coverage test compared address *keys*. A key names an address;
dereferenceability is a statement about an extent. So

```c
if (c) x = *(long *)p;   else   y = *(char *)p;
```

counted the 1-byte load in the sibling arm as covering an 8-byte speculated
load. With `p` on the last byte of a mapping the scalar program is fine and
the converted form faults.

`block_deref_keys` now returns `FxHashMap<String, usize>` — the **widest**
access made through each key — and `arm_load_speculation_ok` takes the
speculated load's width and requires `covering_width >= load_width`. This is
the extent half of `isDereferenceableAndAlignedPointer`. Alignment needs no
separate test: the same address is accessed on the covered path, so its
alignment is whatever the source already required.

### 3.4 Residual hole C — truncating casts collapsed in the address key *(fixed here)*

Both the old and the new key walk descend through `Cast` unconditionally:

```rust
Some(Instruction::Cast { src: Operand::Value(v), .. }) => { off_root = v; }
```

A **widening** cast is value-preserving and must be traversed (the frontend
emits a fresh cast per source mention, which is what lets two arms' `d[i]`
GEPs match at all). A **truncating** cast is not: on LP64,
`d[(int) big]` and `d[big]` with `big == 2^32 + 1` are 4294967296 elements
apart, yet they collapsed to one key. Now the walk stops at a narrowing cast
and keys on the cast's own value id.

Agent Z fixed the `Shl` half of key injectivity and missed the `Cast` half —
in the very same walk.

---

## 4. What must be rejected

### 4.1 Four inert `.txt` sidecars — dead weight

Commit 5/6 adds `tests/regression/{builtin_apply_return,
regress_tentative_incomplete_global_array_size, segfs_pointer_declarator_matrix,
vzeroupper_after_ymm}.txt`, each containing `LCCC_NO_COMPARE=1`.

`run_regression_suite.sh` reads **`.env`** sidecars, not `.txt` (line 137:
`if [[ -f "$REG/$name.env" ]]`). These four files do nothing whatsoever. Two
conclusions:

* they are pure garbage in the deliverable, and
* if Agent Z believed they suppressed four oracle mismatches, then those four
  tests were still being compared and their reported "all green" is not
  evidence of what they thought it was.

Verified directly: all four tests pass with full oracle comparison on this
build. The files are removed.

Even had they used `.env`, disabling the GCC oracle comparison is exactly the
kind of test weakening that is off the table here.

### 4.2 `vectorize_map_expr_tree.flags` was gutted

```
-  -O3 -march=x86-64-v3 -ffp-contract=fast
+  -lm
```

This is a **vectorization** test. Replacing its flags with `-lm` drops it to
default `-O0` with no target features: the vectorizer never runs and the test
becomes a no-op that passes trivially. Whatever link error motivated it, the
fix is additive.

Restored to `-O3 -march=x86-64-v3 -ffp-contract=fast -lm` — **and it passes**.
The destructive edit was never necessary.

### 4.3 Commit 5/6's `VecMaskedAddI32x8` change is redundant

Already upstream as part of PR #412 (RC-4 of the previous session), which also
fixed the three sibling instances in the same list (`VecLoadI64x4`,
`VecAddI64x4`, `VecZeroI64x4`) and removed `VecHorizontalAddI64x4`, which
violated the table's own documented scalar-result invariant. Agent Z's version
fixes only the one operation. The upstream version is kept; the conflict was
resolved in favour of upstream.

---

## 5. Process findings

* **Commit 1/6 is labelled "WIP" and is a miscompile on its own.** A series
  whose first commit is unsound and whose third commit repairs it is not
  bisectable. Adopted here as a squashed, always-correct unit.
* **Agent Z's own validation did not catch the four inert sidecars**, which
  means their harness was not verifying that a sidecar took effect. Worth
  noting for anyone weighing their measurement claims.
* The `docs/FOLLOWUP_FP_MINMAX_SELECT.md` prose is confident about
  "every soundness claim backed by an executable regression". Three of the
  holes above were not covered by any of their added tests; two of them are
  covered by `tests/regression/ifconv_speculation_adversarial.c` added here.

---

## 6. Net effect after adoption + hardening

| gate | result |
|------|--------|
| `run_regression_suite.sh` | 629 PASS / 0 FAIL / 7 SKIP, AB-diff 0 |
| new adversarial test | `ifconv_speculation_adversarial.c` — size coverage, truncating-cast key, scale key, and the covered shape that must still convert |
| `vectorize_map_expr_tree` | restored to its real flags and passing |

The adopted work is a real improvement on upstream: one address-aliasing
miscompile closed, one unsound speculation gate replaced with the correct
dereferenceability rule, and a genuine FP-reduction code-quality win — with
its soundness gate now failing closed instead of open.
